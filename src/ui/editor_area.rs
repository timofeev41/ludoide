//! Central area: tab strip, per-file toolbar, editor / Markdown preview.

use super::{dim, small_icon_button};
use crate::app::App;
use crate::editor::{MAX_TABS, MdMode, view};
use crate::fonts::icon;
use crate::languages::{self, Runner};
use crate::ui::files::file_icon;
use egui::RichText;

impl App {
    pub fn editor_area(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ui, |ui| {
            self.tab_strip(ui);
            if self.tabs.docs.is_empty() {
                self.welcome(ui);
                return;
            }
            self.toolbar(ui);
            ui.add(egui::Separator::default().spacing(0.0));
            if self.find.is_some() {
                self.find_bar(ui);
            }
            let env = view::EditorEnv {
                palette: &self.palette,
                theme_id: self.theme_id,
                font_size: self.settings.general.font_size,
                indent: self.settings.indent(),
            };
            let active = self.tabs.active;
            let Some(doc) = self.tabs.docs.get_mut(active) else { return };
            let preview = languages::get(doc.lang).previewable;
            match (preview, doc.md_mode) {
                (true, MdMode::Preview) => markdown_preview(ui, &mut self.md_cache, &doc.text, doc.uid),
                (true, MdMode::Split) => {
                    let w = ui.available_width() / 2.0;
                    ui.horizontal_top(|ui| {
                        ui.allocate_ui(egui::vec2(w, ui.available_height()), |ui| view::show(ui, doc, &env));
                        ui.separator();
                        ui.allocate_ui(egui::vec2(ui.available_width(), ui.available_height()), |ui| {
                            markdown_preview(ui, &mut self.md_cache, &doc.text, doc.uid)
                        });
                    });
                }
                _ => view::show(ui, doc, &env),
            }
        });
    }

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut close = None;
        let mut activate = None;
        let mut close_others = None;
        let mut copy = None;
        egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(4, 0)).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("tabs_scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, doc) in self.tabs.docs.iter().enumerate() {
                        let selected = i == self.tabs.active;
                        let path_icon = doc.path.as_deref().map_or(icon::FILE_DASHED, file_icon);
                        let fill = if selected { p.bg } else { p.panel };
                        let r =
                            egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new(path_icon).color(if selected { p.accent } else { p.text_dim }));
                                let name = RichText::new(doc.name());
                                ui.label(if selected { name.color(p.text) } else { name.color(p.text_dim) });
                                let marker = if doc.is_dirty() { icon::CIRCLE } else { icon::X };
                                if ui
                                    .add(
                                        egui::Button::new(RichText::new(marker).small().color(p.text_dim)).frame(false),
                                    )
                                    .on_hover_text("Close (⌘W)")
                                    .clicked()
                                {
                                    close = Some(i);
                                }
                            });
                        if selected {
                            let rect = r.response.rect;
                            ui.painter().hline(rect.x_range(), rect.top() + 1.0, egui::Stroke::new(2.0, p.accent));
                        }
                        let resp = r.response.interact(egui::Sense::click());
                        if resp.clicked() {
                            activate = Some(i);
                        }
                        if resp.middle_clicked() {
                            close = Some(i);
                        }
                        let tip = doc.path.as_ref().map_or_else(|| doc.title.clone(), |p| p.display().to_string());
                        resp.on_hover_text(tip).context_menu(|ui| {
                            if ui.button("Close").clicked() {
                                close = Some(i);
                            }
                            if ui.button("Close others").clicked() {
                                close_others = Some(i);
                            }
                            if let Some(path) = &doc.path {
                                if ui.button("Copy path").clicked() {
                                    copy = Some(path.display().to_string());
                                }
                            }
                        });
                    }
                    if !self.tabs.docs.is_empty() {
                        ui.add_space(8.0);
                        ui.label(dim(format!("{}/{MAX_TABS}", self.tabs.docs.len()), &p).small())
                            .on_hover_text("Open tabs (Ctrl+Tab / Ctrl+Shift+Tab to switch)");
                    }
                });
            });
        });
        if let Some(i) = activate {
            self.tabs.activate(i);
        }
        if let Some(i) = close {
            self.request_close(i);
        }
        if let Some(keep) = close_others {
            let uid = self.tabs.docs[keep].uid;
            let mut i = 0;
            while i < self.tabs.docs.len() {
                if self.tabs.docs[i].uid != uid && !self.tabs.docs[i].is_dirty() {
                    self.tabs.close(i);
                } else {
                    i += 1;
                }
            }
            if let Some(i) = self.tabs.docs.iter().position(|d| d.uid == uid) {
                self.tabs.activate(i);
            }
        }
        if let Some(c) = copy {
            self.copy_to_clipboard(c);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        let Some(doc) = self.tabs.current() else { return };
        let lang = languages::get(doc.lang);
        let path_text = doc.path.as_ref().map_or_else(
            || format!("{} (not saved)", doc.title),
            |pth| match self.root_for(pth) {
                Some(root) => format!(
                    "{} › {}",
                    root.file_name().unwrap_or_default().to_string_lossy(),
                    pth.strip_prefix(&root).unwrap_or(pth).display()
                ),
                None => pth.display().to_string(),
            },
        );
        let previewable = lang.previewable;
        let runner = lang.runner;
        let md_mode = doc.md_mode;
        let has_path = doc.path.is_some();
        let unsaved_scratch = !has_path;
        let mut run: Option<bool> = None;
        let mut new_md_mode = None;
        let mut save_as = false;
        let mut ask_claude = false;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(dim(path_text, &p).small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if has_path && small_icon_button(ui, icon::SPARKLE, "Ask Claude Code about this file", &p).clicked()
                    {
                        ask_claude = true;
                    }
                    if unsaved_scratch && small_icon_button(ui, icon::FLOPPY_DISK, "Save as…", &p).clicked() {
                        save_as = true;
                    }
                    if previewable {
                        for (mode, ic, tip) in [
                            (MdMode::Preview, icon::EYE, "Preview"),
                            (MdMode::Split, icon::COLUMNS, "Editor + preview"),
                            (MdMode::Edit, icon::PENCIL_SIMPLE, "Editor"),
                        ] {
                            let text = RichText::new(ic).color(if md_mode == mode { p.accent } else { p.text_dim });
                            if ui.add(egui::Button::new(text).frame(false)).on_hover_text(tip).clicked() {
                                new_md_mode = Some(mode);
                            }
                        }
                    }
                    match runner {
                        Runner::Database => {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("{} Script", icon::PLAY)).color(p.text_dim),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Run all statements (⌘⇧↵)")
                                .clicked()
                            {
                                run = Some(true);
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(format!("{} Run", icon::PLAY)).color(p.success))
                                        .frame(false),
                                )
                                .on_hover_text("Run statement at cursor or selection (⌘↵)")
                                .clicked()
                            {
                                run = Some(false);
                            }
                            self.connection_picker(ui);
                        }
                        Runner::Python => {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(format!("{} Run", icon::PLAY)).color(p.success))
                                        .frame(false),
                                )
                                .on_hover_text("Run with the selected interpreter (⌘↵)")
                                .clicked()
                            {
                                run = Some(false);
                            }
                            let interp =
                                self.settings.interpreter().map_or("no interpreter".to_owned(), |i| i.name.clone());
                            ui.label(dim(interp, &p).small());
                        }
                        _ => {}
                    }
                });
            });
        });
        if let Some(m) = new_md_mode {
            if let Some(d) = self.tabs.current_mut() {
                d.md_mode = m;
            }
        }
        if let Some(whole) = run {
            self.run_current(whole, &ctx);
        }
        if save_as {
            self.save_current_as();
        }
        if ask_claude {
            if let Some(path) = self.tabs.current().and_then(|d| d.path.clone()) {
                self.launch_claude(Some(format!("Let's work on {}", path.display())), &ctx);
            }
        }
    }

    fn connection_picker(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let current = self.active_connection().map_or_else(|| "No connection".to_owned(), |c| c.display_name());
        let db = self
            .active_connection()
            .map(|c| self.db.current_db.get(&c.id).cloned().unwrap_or_else(|| c.database.clone()))
            .filter(|d| !d.is_empty());
        let label = match db {
            Some(d) => format!("{} {current} · {d}", icon::DATABASE),
            None => format!("{} {current}", icon::DATABASE),
        };
        let conns = self.settings.coding.connections.clone();
        egui::ComboBox::from_id_salt("conn_picker")
            .selected_text(RichText::new(label).small().color(p.text_dim))
            .show_ui(ui, |ui| {
                for c in &conns {
                    let selected = self.settings.coding.active_connection.as_deref() == Some(&c.id);
                    if ui.selectable_label(selected, c.display_name()).clicked() {
                        self.settings.coding.active_connection = Some(c.id.clone());
                    }
                }
                if conns.is_empty() {
                    ui.label(dim("Add a connection in the Databases panel", &p));
                }
            });
    }

    fn save_current_as(&mut self) {
        let dir = self.settings.other.scratches_dir.clone();
        let Some(doc) = self.tabs.current() else { return };
        let Some(path) = rfd::FileDialog::new().set_directory(&dir).set_file_name(doc.name()).save_file() else {
            return;
        };
        let active = self.tabs.active;
        let doc = &mut self.tabs.docs[active];
        doc.path = Some(path.clone());
        doc.lang = languages::detect(&path);
        doc.version += 1;
        if let Err(e) = doc.save() {
            self.error(format!("Save failed: {e}"));
        }
        self.workspace.invalidate_all();
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.add_space(ui.available_height() * 0.22);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Ludo").size(34.0).strong().color(p.text_dim));
            ui.label(dim("a small, fast IDE for data engineering", &p));
            ui.add_space(18.0);
            let rows = [
                ("New SQL scratch", "⌘N"),
                ("Run statement / script", "⌘↵ / ⌘⇧↵"),
                ("Switch tabs", "Ctrl+Tab / Ctrl+Shift+Tab"),
                ("Toggle explorer / panel / databases", "⌘B / ⌘J / ⌘⇧D"),
                ("New terminal", "⌘⇧`"),
                ("Find / replace", "⌘F"),
                ("Settings", "⌘,"),
            ];
            for (a, k) in rows {
                ui.horizontal(|ui| {
                    let w = ui.available_width();
                    ui.add_space((w / 2.0 - 230.0).max(0.0));
                    ui.add_sized([220.0, 18.0], egui::Label::new(dim(a, &p)));
                    ui.add_space(20.0);
                    ui.label(RichText::new(k).monospace());
                });
            }
            ui.add_space(18.0);
            if ui.button(format!("{} Open folder…", icon::FOLDER_OPEN)).clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.workspace.add_root(dir);
                }
            }
        });
    }
}

fn markdown_preview(ui: &mut egui::Ui, cache: &mut egui_commonmark::CommonMarkCache, text: &str, uid: u64) {
    egui::ScrollArea::vertical().id_salt(("md_preview", uid)).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
            egui_commonmark::CommonMarkViewer::new().show(ui, cache, text);
        });
    });
}
