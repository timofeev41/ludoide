//! Activity bars, status bar, dialogs and toasts.

use super::{icon_button, panel_frame};
use crate::app::{App, Confirm, LeftTool};
use crate::fonts::icon;
use crate::languages;
use crate::ui::bottom::BottomTab;
use egui::{Align, Layout, Panel, RichText};

impl App {
    pub fn activity_bar_left(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        Panel::left("activity_left").resizable(false).exact_size(44.0).frame(panel_frame(p.panel, 4)).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let tools = [
                    (LeftTool::Explorer, icon::FOLDER_SIMPLE, "Explorer (⌘B)"),
                    (LeftTool::Python, icon::FILE_PY, "Python interpreters"),
                ];
                for (tool, ic, tip) in tools {
                    let active = self.left_open && self.left_tool == tool;
                    if icon_button(ui, ic, tip, active, &p).clicked() {
                        if active {
                            self.left_open = false;
                        } else {
                            self.left_tool = tool;
                            self.left_open = true;
                        }
                    }
                }
                ui.add_space(6.0);
                let term_active = self.bottom_open && self.bottom_tab == BottomTab::Terminal;
                if icon_button(ui, icon::TERMINAL_WINDOW, "Terminal (⌘J)", term_active, &p).clicked() {
                    if term_active {
                        self.bottom_open = false;
                    } else {
                        self.bottom_open = true;
                        self.bottom_tab = BottomTab::Terminal;
                        if self.terms.sessions.is_empty() {
                            self.new_terminal(None, None, None, &ui.ctx().clone());
                        }
                    }
                }
                if icon_button(
                    ui,
                    icon::TABLE,
                    "Query results",
                    self.bottom_open && self.bottom_tab == BottomTab::Results,
                    &p,
                )
                .clicked()
                {
                    self.bottom_open = true;
                    self.bottom_tab = BottomTab::Results;
                }
                if icon_button(ui, icon::SPARKLE, "Claude Code (opens a terminal in the project)", false, &p).clicked()
                {
                    self.launch_claude(None, None, &ui.ctx().clone());
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if icon_button(ui, icon::GEAR_SIX, "Settings (⌘,)", self.settings_open, &p).clicked() {
                        self.settings_open = !self.settings_open;
                    }
                });
            });
        });
    }

    pub fn activity_bar_right(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        Panel::right("activity_right").resizable(false).exact_size(44.0).frame(panel_frame(p.panel, 4)).show(
            ui,
            |ui| {
                ui.vertical_centered(|ui| {
                    if icon_button(ui, icon::DATABASE, "Databases (⌘⇧D)", self.right_open, &p).clicked() {
                        self.right_open = !self.right_open;
                    }
                    if icon_button(
                        ui,
                        icon::LIST_BULLETS,
                        "Database log",
                        self.bottom_open && self.bottom_tab == BottomTab::DbLog,
                        &p,
                    )
                    .clicked()
                    {
                        self.bottom_open = true;
                        self.bottom_tab = BottomTab::DbLog;
                    }
                });
            },
        );
    }

    pub fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        Panel::bottom("status_bar")
            .exact_size(24.0)
            .frame(panel_frame(p.panel, 0).inner_margin(egui::Margin::symmetric(10, 2)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let small = |s: String| RichText::new(s).small().color(p.text_dim);
                    let ctx_path = self
                        .tabs
                        .current()
                        .and_then(|d| d.path.clone())
                        .or_else(|| self.workspace.roots.first().cloned());
                    if let Some(branch) = self.git.get(ctx_path.as_deref()) {
                        ui.label(small(format!("{} {branch}", icon::GIT_BRANCH)));
                    }
                    let interp =
                        self.settings.interpreter().map_or_else(|| "No interpreter".to_owned(), |i| i.name.clone());
                    if ui
                        .add(egui::Label::new(small(format!("{} {interp}", icon::FILE_PY))).sense(egui::Sense::click()))
                        .on_hover_text("Python interpreter — click to change")
                        .clicked()
                    {
                        self.left_tool = LeftTool::Python;
                        self.left_open = true;
                    }
                    let conn =
                        self.active_connection().map_or_else(|| "No connection".to_owned(), |c| c.display_name());
                    let running = if self.db.running.is_some() { " ⏳" } else { "" };
                    if ui
                        .add(
                            egui::Label::new(small(format!("{} {conn}{running}", icon::DATABASE)))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Active database connection")
                        .clicked()
                    {
                        self.right_open = true;
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(small(chrono::Local::now().format("%H:%M").to_string()));
                        if let Some(doc) = self.tabs.current() {
                            let (errors, warnings) =
                                doc.diagnostics.iter().fold((0, 0), |(e, w), d| match d.severity {
                                    crate::lint::Severity::Error => (e + 1, w),
                                    crate::lint::Severity::Warning => (e, w + 1),
                                });
                            let r = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{} {errors}  {} {warnings}", icon::X_CIRCLE, icon::WARNING))
                                        .small()
                                        .color(if errors > 0 {
                                            p.error
                                        } else if warnings > 0 {
                                            p.warning
                                        } else {
                                            p.text_dim
                                        }),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if r.clicked() {
                                self.bottom_open = true;
                                self.bottom_tab = BottomTab::Problems;
                            }
                            ui.label(small("UTF-8".into()));
                            ui.label(small(format!("Spaces: {}", self.settings.indent())));
                            ui.label(small(languages::get(doc.lang).name.to_owned()));
                            let (l, c) = doc.cursor_line_col;
                            let sel = doc.sel.1 - doc.sel.0;
                            let sel = if sel > 0 { format!(" ({sel} chars)") } else { String::new() };
                            ui.label(small(format!("Ln {l}, Col {c}{sel}")));
                        }
                    });
                });
            });
    }

    pub fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.toast else { return };
        let p = self.palette;
        let (text, error) = (t.text.clone(), t.error);
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-60.0, -36.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.panel)
                    .stroke(egui::Stroke::new(1.0, if error { p.error } else { p.border }))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(460.0);
                        let ic = if error { icon::WARNING } else { icon::INFO };
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(ic).color(if error { p.error } else { p.accent }));
                            ui.label(text);
                        });
                    });
            });
    }

    pub fn dialogs(&mut self, ctx: &egui::Context) {
        self.file_dialog_ui(ctx);
        self.connection_editor(ctx);
        self.compile_dialog_ui(ctx);
        let Some(confirm) = &self.confirm else { return };
        let p = self.palette;
        let (title, body) = match confirm {
            Confirm::CloseDirty(i) => (
                "Unsaved changes",
                format!(
                    "Save changes to '{}' before closing?",
                    self.tabs.docs.get(*i).map(|d| d.name()).unwrap_or_default()
                ),
            ),
            Confirm::DeletePath(path) => ("Delete", format!("Delete '{}'? This cannot be undone.", path.display())),
            Confirm::DeleteConnection(id) => (
                "Remove connection",
                format!("Remove connection '{}'?", self.connection(id).map(|c| c.display_name()).unwrap_or_default()),
            ),
        };
        let mut choice: Option<u8> = None;
        egui::Modal::new(egui::Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(body);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if matches!(self.confirm, Some(Confirm::CloseDirty(_))) {
                    if ui.button("Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't save").clicked() {
                        choice = Some(2);
                    }
                } else if ui.button(RichText::new("Delete").color(p.error)).clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(0);
                }
            });
        });
        let Some(choice) = choice else { return };
        let confirm = self.confirm.take().unwrap();
        if choice == 0 {
            return;
        }
        match confirm {
            Confirm::CloseDirty(i) => {
                if choice == 1 {
                    if let Some(d) = self.tabs.docs.get_mut(i) {
                        if let Err(e) = d.save() {
                            self.error(format!("Save failed: {e}"));
                            return;
                        }
                    }
                }
                self.tabs.close(i);
            }
            Confirm::DeletePath(path) => {
                let res = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                match res {
                    Ok(()) => {
                        while let Some(i) =
                            self.tabs.docs.iter().position(|d| d.path.as_ref().is_some_and(|p| p.starts_with(&path)))
                        {
                            self.tabs.close(i);
                        }
                        if let Some(parent) = path.parent() {
                            self.workspace.invalidate(parent);
                        }
                    }
                    Err(e) => self.error(format!("Delete failed: {e}")),
                }
            }
            Confirm::DeleteConnection(id) => {
                if let Err(e) = crate::secrets::delete(&id) {
                    self.error(format!("Couldn't remove the saved password from the Keychain: {e}"));
                }
                self.settings.coding.connections.retain(|c| c.id != id);
                if self.settings.coding.active_connection.as_deref() == Some(&id) {
                    self.settings.coding.active_connection =
                        self.settings.coding.connections.first().map(|c| c.id.clone());
                }
                self.db.trees.remove(&id);
            }
        }
    }
}
