//! Settings: General, Coding, Terminal, Others.

use super::{dim, section_header};
use crate::app::App;
use crate::fonts::icon;
use crate::theme::THEMES;
use crate::ui::db_panel::ConnEditor;
use egui::RichText;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Coding,
    Terminal,
    Others,
}

impl SettingsTab {
    const ALL: [SettingsTab; 4] =
        [SettingsTab::General, SettingsTab::Coding, SettingsTab::Terminal, SettingsTab::Others];

    fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Coding => "Coding",
            SettingsTab::Terminal => "Terminal",
            SettingsTab::Others => "Others",
        }
    }
}

fn font_combo(ui: &mut egui::Ui, id: &str, value: &mut String, families: &[String], default_label: &str) {
    let text = if value.is_empty() { default_label.to_owned() } else { value.clone() };
    egui::ComboBox::from_id_salt(id).selected_text(text).width(260.0).height(400.0).show_ui(ui, |ui| {
        ui.selectable_value(value, String::new(), default_label);
        for f in families {
            ui.selectable_value(value, f.clone(), f);
        }
    });
}

impl App {
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut open = self.settings_open;
        egui::Window::new(RichText::new(format!("{} Settings", icon::GEAR_SIX)).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([720.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(130.0);
                        for tab in SettingsTab::ALL {
                            if ui.selectable_label(self.settings_tab == tab, tab.label()).clicked() {
                                self.settings_tab = tab;
                            }
                        }
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            match self.settings_tab {
                                SettingsTab::General => self.general_settings(ui),
                                SettingsTab::Coding => self.coding_settings(ui),
                                SettingsTab::Terminal => self.terminal_settings(ui),
                                SettingsTab::Others => self.other_settings(ui),
                            }
                        });
                    });
                });
                let _ = p;
            });
        self.settings_open = open;
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        section_header(ui, "Appearance", &p);
        egui::Grid::new("general").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Theme");
            ui.horizontal(|ui| {
                for t in THEMES {
                    ui.selectable_value(&mut self.settings.general.theme, t.id.to_owned(), t.name);
                }
            });
            ui.end_row();

            let families = self.sys_fonts.families().to_vec();
            ui.label("Interface font");
            font_combo(ui, "ui_font", &mut self.settings.general.ui_font, &families, "Built-in (Ubuntu-Light)");
            ui.end_row();

            let mono = self.sys_fonts.monospace_families().to_vec();
            ui.label("Editor font");
            ui.vertical(|ui| {
                font_combo(ui, "editor_font", &mut self.settings.general.editor_font, &mono, "Built-in (Hack)");
                ui.label(dim("Monospaced system fonts; used by the editor and terminal.", &p).small());
            });
            ui.end_row();

            ui.label("Font size");
            ui.add(egui::Slider::new(&mut self.settings.general.font_size, 9.0..=24.0).step_by(0.5).suffix(" pt"));
            ui.end_row();

            ui.label("Interface scale");
            ui.horizontal(|ui| {
                let s = &mut self.settings.general.ui_scale_pct;
                if ui.button("−").clicked() {
                    *s = s.saturating_sub(10).max(50);
                }
                ui.label(RichText::new(format!("{s}%")).monospace());
                if ui.button("+").clicked() {
                    *s = (*s + 10).min(300);
                }
                if ui.button("Reset").clicked() {
                    *s = 100;
                }
                ui.label(dim("⌘+ / ⌘−", &p).small());
            });
            ui.end_row();

            ui.label("Indentation");
            ui.horizontal(|ui| {
                for n in [2usize, 4, 6] {
                    ui.selectable_value(&mut self.settings.general.indent_size, n, format!("{n} spaces"));
                }
            });
            ui.end_row();
        });
    }

    fn coding_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        section_header(ui, "Python interpreters", &p);
        self.interpreters_ui(ui);
        ui.add_space(16.0);

        section_header(ui, "Database connections", &p);
        let mut edit = None;
        for c in &self.settings.coding.connections {
            ui.horizontal(|ui| {
                let active = self.settings.coding.active_connection.as_deref() == Some(&c.id);
                if ui.radio(active, "").on_hover_text("Active connection").clicked() {
                    self.settings.coding.active_connection = Some(c.id.clone());
                }
                ui.label(RichText::new(c.display_name()).strong());
                ui.label(dim(format!("{} · {} · {}", c.kind.name(), c.url, c.username), &p).small());
                if ui.small_button("Edit").clicked() {
                    edit = Some(c.clone());
                }
            });
        }
        if ui.button(format!("{} Add connection", icon::PLUS)).clicked() {
            edit = Some(Default::default());
        }
        if let Some(cfg) = edit {
            let is_new = !self.settings.coding.connections.iter().any(|c| c.id == cfg.id);
            self.db.editing = Some(ConnEditor::new(cfg, is_new));
        }
        ui.add_space(16.0);

        section_header(ui, "SQL & linting", &p);
        egui::Grid::new("coding").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Lint SQL");
            ui.checkbox(&mut self.settings.coding.sql_lint, "Check syntax with the active connection's dialect");
            ui.end_row();
            ui.label("Lint YAML");
            ui.checkbox(&mut self.settings.coding.yaml_lint, "Syntax, duplicate keys, tabs, trailing spaces");
            ui.end_row();
            ui.label("Quick inspect limit");
            ui.add(egui::DragValue::new(&mut self.settings.coding.inspect_limit).range(1..=100_000));
            ui.end_row();
            ui.label("Max result rows");
            ui.add(egui::DragValue::new(&mut self.settings.coding.max_result_rows).range(1..=1_000_000));
            ui.end_row();
            ui.label("Query timeout");
            ui.add(egui::DragValue::new(&mut self.settings.coding.query_timeout_secs).range(1..=86_400).suffix(" s"));
            ui.end_row();
        });
    }

    fn terminal_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Grid::new("terminal").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Shell");
            ui.add(egui::TextEdit::singleline(&mut self.settings.terminal.shell).desired_width(260.0));
            ui.end_row();
            ui.label("Font size");
            ui.add(egui::Slider::new(&mut self.settings.terminal.font_size, 9.0..=24.0).step_by(0.5).suffix(" pt"));
            ui.end_row();
            ui.label("Scrollback");
            ui.add(egui::DragValue::new(&mut self.settings.terminal.scrollback).range(100..=100_000).suffix(" lines"));
            ui.end_row();
        });
        ui.add_space(10.0);
        section_header(ui, "Environment variables", &p);
        ui.label(dim("One KEY=VALUE per line, added to every new terminal.", &p).small());
        let mut text = self.settings.terminal.env.join("\n");
        if ui
            .add(egui::TextEdit::multiline(&mut text).code_editor().desired_rows(5).desired_width(f32::INFINITY))
            .changed()
        {
            self.settings.terminal.env = text.lines().map(str::to_owned).filter(|l| !l.trim().is_empty()).collect();
        }
    }

    fn other_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Grid::new("others").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Scratches folder");
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.settings.other.scratches_dir.display().to_string()).monospace());
                if ui.button("Change…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        self.settings.other.scratches_dir = dir;
                    }
                }
            });
            ui.end_row();
            ui.label("Session");
            ui.checkbox(&mut self.settings.other.restore_session, "Restore folders, tabs and terminals on start");
            ui.end_row();
            ui.label("Autosave");
            ui.checkbox(&mut self.settings.other.autosave_on_focus_loss, "Save files when the window loses focus");
            ui.end_row();
            ui.label("Explorer");
            ui.checkbox(&mut self.settings.other.show_hidden_files, "Show hidden files");
            ui.end_row();
            ui.label("Claude Code command");
            ui.add(egui::TextEdit::singleline(&mut self.settings.other.claude_command).desired_width(260.0));
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.label(
            dim(format!("Settings file: {}", crate::settings::config_dir().join("settings.json").display()), &p)
                .small(),
        );
    }
}
