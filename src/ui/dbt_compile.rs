//! "Compile" for dbt models: ask for variable values, write plain SQL to Scratches.

use super::dim;
use crate::app::App;
use crate::dbt::{self, DbtProject};
use crate::fonts::icon;
use egui::{Key, RichText};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct CompileDialog {
    path: PathBuf,
    text: String,
    project: DbtProject,
    /// `(name, value)` in order of first appearance.
    vars: Vec<(String, String)>,
    schema: String,
    focus: bool,
}

impl App {
    pub fn open_compile_dialog(&mut self) {
        let Some(doc) = self.tabs.current() else { return };
        let (Some(path), Some(project)) = (doc.path.clone(), doc.dbt.clone()) else { return };
        let saved = &self.settings.coding.dbt_vars;
        let vars = dbt::variables(&doc.text)
            .into_iter()
            .map(|v| {
                let value = saved.get(&v.name).cloned().or(v.default).unwrap_or_default();
                (v.name, value)
            })
            .collect();
        let schema =
            self.settings.coding.dbt_ref_schema.get(&project.name).cloned().unwrap_or_else(|| project.default_schema());
        self.compile = Some(CompileDialog { path, text: doc.text.clone(), project, vars, schema, focus: true });
    }

    pub fn compile_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.compile else { return };
        let p = self.palette;
        let mut submit = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("dbt_compile")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(format!("{} Compile {}", icon::BRACKETS_CURLY, dlg.path.file_name().unwrap_or_default().to_string_lossy()));
            ui.label(dim(format!("dbt project '{}'", dlg.project.name), &p).small());
            ui.add_space(8.0);
            if dlg.vars.is_empty() {
                ui.label(dim("No env_var()/var() variables in this model.", &p));
            } else {
                ui.label("What to insert instead?");
            }
            egui::Grid::new("dbt_vars").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                for (i, (name, value)) in dlg.vars.iter_mut().enumerate() {
                    ui.label(RichText::new(name.as_str()).monospace());
                    let r = ui.add(egui::TextEdit::singleline(value).desired_width(280.0));
                    if i == 0 && std::mem::take(&mut dlg.focus) {
                        r.request_focus();
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                    ui.end_row();
                }
                ui.label("ref() schema").on_hover_text("{{ ref('model') }} becomes <schema>.model");
                ui.add(egui::TextEdit::singleline(&mut dlg.schema).desired_width(280.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(
                dim(
                    "source('a','b') → a.b · ref('m') → schema.m · config() and {% if is_incremental() %} blocks are removed",
                    &p,
                )
                .small(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        cancel = true;
                    }
                    if ui.button(RichText::new("Compile").strong()).clicked() {
                        submit = true;
                    }
                });
            });
        });
        if cancel {
            self.compile = None;
            return;
        }
        if !submit {
            return;
        }
        let dlg = self.compile.take().unwrap();
        let values: HashMap<String, String> = dlg.vars.iter().cloned().collect();
        for (k, v) in &values {
            self.settings.coding.dbt_vars.insert(k.clone(), v.clone());
        }
        self.settings.coding.dbt_ref_schema.insert(dlg.project.name.clone(), dlg.schema.clone());
        let model = dlg.path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let compiled = dbt::compile(&dlg.text, &values, dlg.schema.trim(), &model);
        let out = dbt::output_path(&self.settings.other.scratches_dir, &dlg.project.name, &dlg.path);
        let header = format!("-- compiled by Ludo from {}\n", dlg.path.display());
        let written = out
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&out, header + &compiled.sql));
        match written {
            Ok(()) => {
                self.workspace.invalidate_all();
                self.open_file(&out);
                if compiled.unresolved.is_empty() {
                    let short = out.strip_prefix(&self.settings.other.scratches_dir).unwrap_or(&out);
                    self.notify(format!("Compiled → Scratches/{}", short.display()));
                } else {
                    self.error(format!(
                        "Compiled with {} Jinja tag(s) left as-is (macros/loops): {}",
                        compiled.unresolved.len(),
                        compiled.unresolved.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
            }
            Err(e) => self.error(format!("Cannot write {}: {e}", out.display())),
        }
    }
}
