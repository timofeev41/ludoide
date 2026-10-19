//! Central area: tab strip, per-file toolbar, editor / Markdown preview.

use super::{copy_path_menu, dim, path_variants, small_icon_button};
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
        let mut scrolled = None;
        egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(4, 0)).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("tabs_scroll").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, doc) in self.tabs.docs.iter().enumerate() {
                        let selected = i == self.tabs.active;
                        let path_icon = doc.path.as_deref().map_or(icon::FILE_DASHED, file_icon);
                        let (resp, close_clicked) =
                            editor_tab(ui, path_icon, &doc.name(), doc.is_dirty(), selected, doc.uid, &p);
                        if selected && self.tab_in_view != doc.uid {
                            resp.scroll_to_me(None);
                            scrolled = Some(doc.uid);
                        }
                        if close_clicked || resp.middle_clicked() {
                            close = Some(i);
                        } else if resp.clicked() {
                            activate = Some(i);
                        }
                        let tip = doc.path.as_ref().map_or_else(|| doc.title.clone(), |p| p.display().to_string());
                        let variants = doc.path.as_deref().map(|pth| path_variants(pth, self.root_for(pth).as_deref()));
                        resp.on_hover_text(tip).context_menu(|ui| {
                            if ui.button("Close").clicked() {
                                close = Some(i);
                            }
                            if ui.button("Close others").clicked() {
                                close_others = Some(i);
                            }
                            if let Some(v) = &variants {
                                ui.separator();
                                if let Some(text) = copy_path_menu(ui, v) {
                                    copy = Some(text);
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
        if let Some(uid) = scrolled {
            self.tab_in_view = uid;
        }
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
        // Just the project the file belongs to; the full path is in the tooltip.
        let (path_text, path_tip) = match &doc.path {
            None => (format!("{} (not saved)", doc.title), String::new()),
            Some(pth) => {
                let name = match self.root_for(pth) {
                    Some(root) if root == self.settings.other.scratches_dir => "Scratches".to_owned(),
                    Some(root) => root.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    None => pth.parent().and_then(|d| d.file_name()).unwrap_or_default().to_string_lossy().into_owned(),
                };
                (name, pth.display().to_string())
            }
        };
        let is_dbt = doc.dbt.is_some();
        let mut compile = false;
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
                let r = ui.label(dim(format!("{} {path_text}", icon::FOLDER_SIMPLE), &p).small());
                if !path_tip.is_empty() {
                    r.on_hover_text(path_tip);
                }
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
                            if is_dbt
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!("{} Compile", icon::BRACKETS_CURLY)).color(p.accent),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text(
                                        "Resolve dbt Jinja (env_var/var, source, ref, config) into plain SQL\n\
                                         saved in Scratches/compiled/<project>/ — ready to run",
                                    )
                                    .clicked()
                            {
                                compile = true;
                            }
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
        if compile {
            self.open_compile_dialog();
        }
        if let Some(whole) = run {
            self.run_current(whole, &ctx);
        }
        if save_as {
            self.save_current_as();
        }
        if ask_claude {
            if let Some(path) = self.tabs.current().and_then(|d| d.path.clone()) {
                self.launch_claude(Some(format!("Let's work on {}", path.display())), None, &ctx);
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
        let mut disconnect = None;
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
                } else if let Some(id) = self.settings.coding.active_connection.clone() {
                    ui.separator();
                    if ui
                        .button(format!("{} Deactivate", icon::PLUGS))
                        .on_hover_text("Disconnect from the active connection")
                        .clicked()
                    {
                        disconnect = Some(id);
                    }
                }
            });
        if let Some(id) = disconnect {
            self.deactivate_connection(&id);
        }
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
                ("Find in project / in file", "⌘F / ⌘R"),
                ("Git", "⌘⇧G"),
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
            ui.add_space(6.0);
            if ui.link(dim("All shortcuts…", &p).small()).clicked() {
                self.settings_tab = crate::ui::settings_window::SettingsTab::Shortcuts;
                self.settings_open = true;
            }
            ui.add_space(12.0);
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

/// One editor tab: icon, name and a close button, all centred on one line. The
/// close button gets its own hit area on top of the tab's.
fn editor_tab(
    ui: &mut egui::Ui,
    file_icon: &str,
    name: &str,
    dirty: bool,
    selected: bool,
    uid: u64,
    p: &crate::theme::Palette,
) -> (egui::Response, bool) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    let text_color = if selected { p.text } else { p.text_dim };
    let icon_g = ui.painter().layout_no_wrap(file_icon.to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
    let name_g = ui.painter().layout_no_wrap(name.to_owned(), font.clone(), text_color);
    let (pad, gap, close_w) = (10.0, 6.0, 18.0);
    let h = font.size + 18.0;
    let w = pad + icon_g.size().x + gap + name_g.size().x + gap + close_w + pad - 4.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - pad + 4.0 - close_w / 2.0, rect.center().y),
        egui::vec2(close_w, close_w),
    );
    let close = ui.interact(close_rect, egui::Id::unique(("tab_close", uid)), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            p.bg
        } else if resp.hovered() || close.hovered() {
            p.hover
        } else {
            p.panel
        };
        painter.rect_filled(rect, 0.0, fill);
        if selected {
            painter.hline(rect.x_range(), rect.top() + 1.0, egui::Stroke::new(2.0, p.accent));
        }
        let cy = rect.center().y;
        let mut x = rect.left() + pad;
        let icon_color = if selected { p.accent } else { p.text_dim };
        painter.galley(egui::pos2(x, cy - icon_g.size().y / 2.0), icon_g.clone(), icon_color);
        x += icon_g.size().x + gap;
        painter.galley(egui::pos2(x, cy - name_g.size().y / 2.0), name_g, text_color);
        // A dot marks unsaved changes; hovering turns it into the close button.
        let show_x = !dirty || close.hovered();
        if close.hovered() {
            painter.rect_filled(close_rect, 4.0, p.panel_alt);
        }
        let glyph = if show_x { icon::X } else { icon::CIRCLE };
        let color = if close.hovered() { p.text } else { p.text_dim };
        painter.text(close_rect.center(), egui::Align2::CENTER_CENTER, glyph, small, color);
    }
    let clicked = close.on_hover_text("Close (⌘W)").clicked();
    (resp, clicked)
}
