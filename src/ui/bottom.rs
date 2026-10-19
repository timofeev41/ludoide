//! Bottom panel: terminals, query results, database log, problems.

use super::{dim, panel_frame, small_icon_button};
use crate::app::App;
use crate::db::{self, export, export::CopyFormat};
use crate::fonts::icon;
use crate::lint::Severity;
use crate::terminal::{self, TerminalSession};
use egui::{RichText, TextWrapMode};
use egui_extras::{Column, TableBuilder};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    Terminal,
    Results,
    DbLog,
    Problems,
}

#[derive(Default)]
pub struct Terminals {
    pub sessions: Vec<TerminalSession>,
    pub active: usize,
    /// `(session uid, edited name)` while renaming.
    pub renaming: Option<(u64, String)>,
    pub focus_next_frame: bool,
    rename_focus: bool,
}

impl App {
    pub fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Panel::bottom("bottom_panel")
            .resizable(true)
            .default_size(280.0)
            .size_range(80.0..=1200.0)
            .frame(panel_frame(p.panel, 0))
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let tabs = [
                        (BottomTab::Terminal, format!("{} Terminal", icon::TERMINAL_WINDOW)),
                        (BottomTab::Results, format!("{} Results", icon::TABLE)),
                        (BottomTab::DbLog, format!("{} DB Log", icon::LIST_BULLETS)),
                        (BottomTab::Problems, format!("{} Problems", icon::WARNING)),
                    ];
                    for (tab, label) in tabs {
                        let selected = self.bottom_tab == tab;
                        let text = if selected { RichText::new(label).strong() } else { dim(label, &p) };
                        if ui.selectable_label(selected, text).clicked() {
                            self.bottom_tab = tab;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        if small_icon_button(ui, icon::X, "Hide panel (⌘J)", &p).clicked() {
                            self.bottom_open = false;
                        }
                    });
                });
                ui.add(egui::Separator::default().spacing(0.0));
                match self.bottom_tab {
                    BottomTab::Terminal => self.terminals_ui(ui),
                    BottomTab::Results => self.results_ui(ui),
                    BottomTab::DbLog => self.db_log_ui(ui),
                    BottomTab::Problems => self.problems_ui(ui),
                }
            });
    }

    fn terminals_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        let mut close: Option<usize> = None;
        let mut new = false;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in self.terms.sessions.iter_mut().enumerate() {
                    let renaming = self.terms.renaming.as_ref().is_some_and(|(uid, _)| *uid == t.uid);
                    if renaming {
                        let (_, name) = self.terms.renaming.as_mut().unwrap();
                        let r = ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
                        if std::mem::take(&mut self.terms.rename_focus) {
                            r.request_focus();
                        }
                        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if r.lost_focus() || enter || escape {
                            let new_name = name.trim().to_owned();
                            if !new_name.is_empty() && !escape {
                                t.name = new_name;
                            }
                            self.terms.renaming = None;
                        }
                    } else {
                        let selected = i == self.terms.active;
                        let mut label = RichText::new(&t.name);
                        if t.exited.is_some() {
                            label = label.color(p.text_dim).italics();
                        }
                        let r = ui.selectable_label(selected, label).on_hover_text("Double-click to rename");
                        if r.clicked() {
                            self.terms.active = i;
                            self.terms.focus_next_frame = true;
                        }
                        if r.double_clicked() {
                            self.terms.renaming = Some((t.uid, t.name.clone()));
                            self.terms.rename_focus = true;
                        }
                        r.context_menu(|ui| {
                            if ui.button("Rename").clicked() {
                                self.terms.renaming = Some((t.uid, t.name.clone()));
                                self.terms.rename_focus = true;
                            }
                            if ui.button("Close").clicked() {
                                close = Some(i);
                            }
                        });
                        if small_icon_button(ui, icon::X, "Close terminal", &p).clicked() {
                            close = Some(i);
                        }
                    }
                    ui.add_space(4.0);
                }
                if small_icon_button(ui, icon::PLUS, "New terminal (⌘⇧`)", &p).clicked() {
                    new = true;
                }
            });
        });
        if let Some(i) = close {
            self.terms.sessions.remove(i);
            if self.terms.active >= self.terms.sessions.len() {
                self.terms.active = self.terms.sessions.len().saturating_sub(1);
            }
        }
        if new {
            self.new_terminal(None, None, None, &ctx);
        }
        let font_size = self.settings.terminal.font_size;
        let focus = std::mem::take(&mut self.terms.focus_next_frame);
        match self.terms.sessions.get_mut(self.terms.active) {
            Some(t) => {
                if focus {
                    ui.memory_mut(|m| m.request_focus(egui::Id::unique(("terminal", t.uid))));
                }
                terminal::show(ui, t, &p, font_size);
            }
            None => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    if ui.button(format!("{} New terminal", icon::PLUS)).clicked() {
                        self.new_terminal(None, None, None, &ctx);
                    }
                });
            }
        }
    }

    fn results_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut cancel = false;
        let mut copy: Option<String> = None;
        let default_format = self.settings.coding.copy_format;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(run) = &self.db.running {
                    ui.spinner();
                    ui.label(format!("Running… {:.1}s", run.started.elapsed().as_secs_f32())).on_hover_text(&run.sql);
                    if ui.button(format!("{} Cancel", icon::STOP)).clicked() {
                        cancel = true;
                    }
                }
                let Some(res) = &self.db.result else {
                    if self.db.running.is_none() {
                        ui.label(dim(
                            "Run a query with ⌘↵ (statement at cursor or selection) or ⌘⇧↵ (whole script).",
                            &p,
                        ));
                    }
                    return;
                };
                let step = if res.total > 1 { format!("[{}/{}] ", res.index, res.total) } else { String::new() };
                match &res.outcome {
                    Ok(q) => {
                        let mut parts = vec![format!("{}{} rows", step, q.rows.len())];
                        if q.truncated {
                            parts.push("(limited)".into());
                        }
                        parts.push(format!("{:.3}s", q.elapsed.as_secs_f64()));
                        if let Some(r) = q.rows_read {
                            parts.push(format!("read {} rows", db::human_count(r)));
                        }
                        if let Some(b) = q.bytes_read {
                            parts.push(db::human_bytes(b));
                        }
                        ui.label(
                            RichText::new(format!("{} {}", icon::CHECK_CIRCLE, parts.join(" · "))).color(p.success),
                        );
                        ui.label(dim(format!("on {}", res.conn), &p).small());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !q.rows.is_empty() {
                                // Split button: copy in the default format, or pick another.
                                let table = export::table_from_sql(&res.sql);
                                ui.menu_button(icon::CARET_DOWN, |ui| {
                                    for f in CopyFormat::ALL {
                                        let mut text = f.label().to_owned();
                                        if f == default_format {
                                            text.push_str("  (default)");
                                        }
                                        if ui.button(text).clicked() {
                                            copy = Some(export::export(q, f, &table));
                                            ui.close();
                                        }
                                    }
                                });
                                let label = format!("{} Copy {}", icon::COPY, default_format.label());
                                if ui.small_button(label).on_hover_text("Default format: Settings → Coding").clicked()
                                {
                                    copy = Some(export::export(q, default_format, &table));
                                }
                            }
                        });
                    }
                    Err(e) => {
                        ui.label(RichText::new(format!("{} {step}Query failed", icon::X_CIRCLE)).color(p.error));
                        let _ = e;
                    }
                }
            });
        });
        if cancel {
            if let Some(cfg) = self.db.running.as_ref().and_then(|r| self.connection(&r.conn_id)).cloned() {
                self.db.cancel(&cfg, self.timeout());
            }
        }
        if let Some(text) = copy {
            self.copy_to_clipboard(text);
            self.notify("Results copied to clipboard");
        }
        let Some(res) = &self.db.result else { return };
        match &res.outcome {
            Err(e) => {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add_space(4.0);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
                        ui.label(RichText::new(e).monospace().color(p.error));
                        ui.add_space(8.0);
                        ui.label(dim(&res.sql, &p).monospace());
                    });
                });
            }
            Ok(q) if q.columns.is_empty() => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(q.message.clone().unwrap_or_else(|| "OK".into()));
                });
            }
            Ok(q) => {
                let mut copied: Option<String> = None;
                egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
                    let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
                    let mut table = TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::auto().at_least(36.0));
                    let char_w = row_h * 0.55;
                    for (ci, (name, _)) in q.columns.iter().enumerate() {
                        // Size from the header and a sample of values.
                        let longest = q.rows.iter().take(100).map(|r| r[ci].chars().count()).max().unwrap_or(0);
                        let w = (longest.max(name.len()) as f32 * char_w + 24.0).clamp(60.0, 420.0);
                        table = table.column(Column::initial(w).at_least(40.0).clip(true));
                    }
                    table
                        .auto_shrink([false, false])
                        .header(row_h + 2.0, |mut header| {
                            header.col(|ui| {
                                ui.label(dim("#", &p));
                            });
                            for (name, ty) in &q.columns {
                                header.col(|ui| {
                                    ui.add(egui::Label::new(RichText::new(name).strong()).sense(egui::Sense::hover()))
                                        .on_hover_text(RichText::new(ty).monospace());
                                });
                            }
                        })
                        .body(|body| {
                            body.rows(row_h, q.rows.len(), |mut row| {
                                let i = row.index();
                                row.col(|ui| {
                                    ui.label(dim((i + 1).to_string(), &p).monospace());
                                });
                                for (ci, cell) in q.rows[i].iter().enumerate() {
                                    row.col(|ui| {
                                        let is_null = cell == "NULL";
                                        let text = if is_null {
                                            RichText::new("NULL").italics().color(p.text_dim)
                                        } else {
                                            RichText::new(cell.replace('\n', "⏎")).monospace()
                                        };
                                        let r = ui.add(
                                            egui::Label::new(text)
                                                .wrap_mode(TextWrapMode::Truncate)
                                                .sense(egui::Sense::click()),
                                        );
                                        if r.double_clicked() {
                                            copied = Some(cell.clone());
                                        }
                                        r.context_menu(|ui| {
                                            if ui.button("Copy value").clicked() {
                                                copied = Some(cell.clone());
                                            }
                                            if ui.button("Copy row (TSV)").clicked() {
                                                copied = Some(q.rows[i].join("\t"));
                                            }
                                            if ui.button("Copy column name").clicked() {
                                                copied = Some(q.columns[ci].0.clone());
                                            }
                                        });
                                    });
                                }
                            });
                        });
                });
                if let Some(c) = copied {
                    self.copy_to_clipboard(c);
                    self.notify("Copied to clipboard");
                }
            }
        }
    }

    fn db_log_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(dim(format!("{} entries", self.db.log.len()), &p));
                if ui.small_button("Clear").clicked() {
                    self.db.log.clear();
                }
            });
        });
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
                for (i, e) in self.db.log.iter().enumerate() {
                    let status = if e.ok {
                        RichText::new(icon::CHECK_CIRCLE).color(p.success)
                    } else {
                        RichText::new(icon::X_CIRCLE).color(p.error)
                    };
                    let header = format!(
                        "{}  {}  {}  ({:.0} ms)",
                        e.at.format("%H:%M:%S%.3f"),
                        e.conn,
                        e.what,
                        e.elapsed.as_secs_f64() * 1000.0
                    );
                    ui.horizontal(|ui| {
                        ui.label(status);
                        egui::CollapsingHeader::new(RichText::new(header).monospace()).id_salt(("log", i)).show(
                            ui,
                            |ui| {
                                for l in &e.lines {
                                    ui.label(dim(l, &p).monospace().small());
                                }
                            },
                        );
                    });
                }
            });
        });
    }

    fn problems_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let mut goto: Option<(usize, usize)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
                let mut any = false;
                for (ti, doc) in self.tabs.docs.iter().enumerate() {
                    if doc.diagnostics.is_empty() {
                        continue;
                    }
                    any = true;
                    ui.label(RichText::new(format!("{} ({})", doc.name(), doc.diagnostics.len())).strong());
                    for d in &doc.diagnostics {
                        let (ic, c) = match d.severity {
                            Severity::Error => (icon::X_CIRCLE, p.error),
                            Severity::Warning => (icon::WARNING, p.warning),
                        };
                        let r = ui.add(
                            egui::Label::new(
                                RichText::new(format!("  {ic} {}:{}  {}", d.line, d.col, d.message)).color(c),
                            )
                            .sense(egui::Sense::click()),
                        );
                        if r.clicked() {
                            goto = Some((ti, d.line));
                        }
                    }
                }
                if !any {
                    ui.label(dim("No problems in open files.", &p));
                }
            });
        });
        if let Some((ti, line)) = goto {
            self.tabs.activate(ti);
            if let Some(d) = self.tabs.current_mut() {
                d.goto_line = Some(line);
            }
        }
    }
}
