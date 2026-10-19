//! Python interpreters / virtual environments.

use super::{dim, section_header, small_icon_button};
use crate::app::App;
use crate::fonts::icon;
use crate::settings::PythonInterpreter;
use egui::RichText;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct PythonState {
    versions: HashMap<PathBuf, String>,
    pub new_venv_name: String,
}

/// `python --version`, cached per path.
fn version(path: &Path) -> String {
    std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()
        .map(|o| {
            let s = if o.stdout.is_empty() { o.stderr } else { o.stdout };
            String::from_utf8_lossy(&s).trim().to_owned()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "not found".into())
}

/// Common interpreter locations on macOS, plus `.venv`s in the workspace.
fn detect(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = dirs::home_dir().unwrap_or_default();
    for p in ["/opt/homebrew/bin/python3", "/usr/local/bin/python3", "/usr/bin/python3"] {
        out.push(PathBuf::from(p));
    }
    for base in [home.join(".pyenv/versions"), home.join(".virtualenvs"), home.join(".local/share/virtualenvs")] {
        if let Ok(rd) = std::fs::read_dir(base) {
            for e in rd.flatten() {
                out.push(e.path().join("bin/python"));
            }
        }
    }
    for r in roots {
        for v in [".venv", "venv", "env"] {
            out.push(r.join(v).join("bin/python"));
        }
    }
    out.into_iter().filter(|p| p.exists()).collect()
}

fn display_name(path: &Path) -> String {
    // `/x/proj/.venv/bin/python` -> `proj/.venv`
    let comps: Vec<_> = path.components().rev().take(4).collect();
    if path.ends_with("bin/python") || path.ends_with("bin/python3") {
        if let (Some(env), Some(parent @ std::path::Component::Normal(_))) = (comps.get(2), comps.get(3)) {
            return format!("{}/{}", parent.as_os_str().to_string_lossy(), env.as_os_str().to_string_lossy());
        }
    }
    path.display().to_string()
}

impl App {
    pub fn add_interpreter(&mut self, path: PathBuf) {
        if self.settings.coding.interpreters.iter().any(|i| i.path == path) {
            return;
        }
        let v = version(&path);
        let name = format!("{} ({})", v.trim_start_matches("Python "), display_name(&path));
        self.python.versions.insert(path.clone(), v);
        self.settings.coding.interpreters.push(PythonInterpreter { name, path });
        if self.settings.coding.active_interpreter.is_none() {
            self.settings.coding.active_interpreter = Some(self.settings.coding.interpreters.len() - 1);
        }
    }

    /// Shared by the Python panel and Settings → Coding.
    pub fn interpreters_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut remove = None;
        if self.settings.coding.interpreters.is_empty() {
            ui.label(dim("No interpreters configured.", &p));
        }
        for (i, interp) in self.settings.coding.interpreters.iter().enumerate() {
            ui.horizontal(|ui| {
                let active = self.settings.coding.active_interpreter == Some(i);
                if ui.radio(active, "").on_hover_text("Use this interpreter").clicked() {
                    self.settings.coding.active_interpreter = Some(i);
                }
                ui.vertical(|ui| {
                    ui.label(RichText::new(&interp.name).strong());
                    ui.label(dim(interp.path.display().to_string(), &p).small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_icon_button(ui, icon::TRASH, "Remove", &p).clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove {
            self.settings.coding.interpreters.remove(i);
            let a = &mut self.settings.coding.active_interpreter;
            *a = match *a {
                Some(x) if x == i => None,
                Some(x) if x > i => Some(x - 1),
                other => other,
            };
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(format!("{} Add executable…", icon::PLUS)).clicked() {
                if let Some(path) = rfd::FileDialog::new().set_title("Select Python executable").pick_file() {
                    self.add_interpreter(path);
                }
            }
            if ui.button(format!("{} Detect", icon::MAGNIFYING_GLASS)).clicked() {
                let found = detect(&self.workspace.roots);
                let before = self.settings.coding.interpreters.len();
                for f in found {
                    self.add_interpreter(f);
                }
                let added = self.settings.coding.interpreters.len() - before;
                self.notify(format!("Found {added} new interpreter(s)"));
            }
        });
    }

    pub fn python_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        section_header(ui, "Python interpreters", &p);
        ui.add_space(4.0);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.interpreters_ui(ui);
            ui.add_space(14.0);
            section_header(ui, "Create virtual environment", &p);
            ui.add_space(4.0);
            let base = self.context_dir();
            match (&base, self.settings.interpreter().cloned()) {
                (Some(dir), Some(interp)) => {
                    if self.python.new_venv_name.is_empty() {
                        self.python.new_venv_name = ".venv".into();
                    }
                    ui.label(dim(format!("in {}", dir.display()), &p).small());
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.python.new_venv_name).desired_width(120.0));
                        if ui.button("Create").on_hover_text(format!("{} -m venv", interp.path.display())).clicked() {
                            let target = dir.join(self.python.new_venv_name.trim());
                            self.new_terminal(
                                Some("venv".into()),
                                Some((interp.path.clone(), vec!["-m".into(), "venv".into(), target.to_string_lossy().into_owned()])),
                                Some(dir.clone()),
                                &ctx,
                            );
                            // Registered right away; it becomes usable once the terminal finishes.
                            let py = target.join("bin/python");
                            let name = format!("venv ({})", display_name(&py));
                            if !self.settings.coding.interpreters.iter().any(|i| i.path == py) {
                                self.settings.coding.interpreters.push(PythonInterpreter { name, path: py });
                            }
                        }
                    });
                }
                (None, _) => {
                    ui.label(dim("Open a project folder first.", &p));
                }
                (_, None) => {
                    ui.label(dim("Select a base interpreter first.", &p));
                }
            }
            ui.add_space(14.0);
            ui.label(dim("Python files run with the selected interpreter (⌘↵) in a new terminal, with the project root as working directory.", &p).small());
        });
    }
}
