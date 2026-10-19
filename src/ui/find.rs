//! Find / replace bar for the active editor (⌘F).

use super::{dim, small_icon_button};
use crate::app::App;
use crate::fonts::icon;
use egui::{Key, RichText};

#[derive(Default)]
pub struct FindState {
    pub query: String,
    pub replace: String,
    pub case_sensitive: bool,
    pub focus: bool,
}

/// Char ranges of all matches of `needle` in `hay`.
pub fn find_all(hay: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() {
        return Vec::new();
    }
    let fold = |s: &str| if case_sensitive { s.to_owned() } else { s.to_lowercase() };
    let hay_chars: Vec<char> = fold(hay).chars().collect();
    let needle: Vec<char> = fold(needle).chars().collect();
    // Lowercasing can change char counts for a few scripts; fall back to exact search then.
    let hay_chars = if hay_chars.len() == hay.chars().count() { hay_chars } else { hay.chars().collect() };
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay_chars.len() {
        if hay_chars[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

impl App {
    pub fn open_find(&mut self) {
        let selected = self.tabs.current().map(|d| d.selected_text()).unwrap_or_default();
        let st = self.find.get_or_insert_with(FindState::default);
        if !selected.is_empty() && !selected.contains('\n') {
            st.query = selected;
        }
        st.focus = true;
    }

    pub fn find_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(st) = &mut self.find else { return };
        let Some(doc) = self.tabs.docs.get_mut(self.tabs.active) else { return };
        let matches = find_all(&doc.text, &st.query, st.case_sensitive);
        let current = matches.iter().position(|m| *m == doc.sel);
        let mut step: Option<bool> = None;
        let mut replace_one = false;
        let mut replace_all = false;
        let mut close = false;
        egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(p.text_dim));
                let r = ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("Find").desired_width(220.0));
                if std::mem::take(&mut st.focus) {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    step = Some(!ui.input(|i| i.modifiers.shift));
                    r.request_focus();
                }
                ui.toggle_value(&mut st.case_sensitive, "Aa").on_hover_text("Match case");
                let count = match (current, matches.len()) {
                    (_, 0) if !st.query.is_empty() => "no results".to_owned(),
                    (Some(i), n) => format!("{}/{n}", i + 1),
                    (None, n) if n > 0 => format!("{n} found"),
                    _ => String::new(),
                };
                ui.label(dim(count, &p).small());
                if small_icon_button(ui, icon::CARET_UP, "Previous (⇧↵)", &p).clicked() {
                    step = Some(false);
                }
                if small_icon_button(ui, icon::CARET_DOWN, "Next (↵)", &p).clicked() {
                    step = Some(true);
                }
                ui.add_space(8.0);
                ui.add(egui::TextEdit::singleline(&mut st.replace).hint_text("Replace").desired_width(160.0));
                if ui.small_button("Replace").clicked() {
                    replace_one = true;
                }
                if ui.small_button("All").clicked() {
                    replace_all = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if small_icon_button(ui, icon::X, "Close (Esc)", &p).clicked()
                        || ui.input(|i| i.key_pressed(Key::Escape))
                    {
                        close = true;
                    }
                });
            });
        });
        if replace_all && !matches.is_empty() {
            let chars: Vec<char> = doc.text.chars().collect();
            let mut out = String::new();
            let mut last = 0;
            for (a, b) in &matches {
                out.extend(&chars[last..*a]);
                out.push_str(&st.replace);
                last = *b;
            }
            out.extend(&chars[last..]);
            doc.text = out;
            doc.mark_edited();
            self.toast_text = Some(format!("Replaced {} occurrence(s)", matches.len()));
        } else if replace_one {
            if let Some(i) = current {
                let (a, b) = matches[i];
                let chars: Vec<char> = doc.text.chars().collect();
                let mut out: String = chars[..a].iter().collect();
                out.push_str(&st.replace);
                out.extend(&chars[b..]);
                doc.text = out;
                doc.mark_edited();
                doc.sel = (a, a + st.replace.chars().count());
            }
            step = Some(true);
        }
        if let Some(forward) = step {
            let matches = find_all(&doc.text, &st.query, st.case_sensitive);
            let caret = if forward { doc.sel.1 } else { doc.sel.0 };
            let next = if forward {
                matches.iter().find(|m| m.0 >= caret).or(matches.first())
            } else {
                matches.iter().rev().find(|m| m.1 <= caret).or(matches.last())
            };
            if let Some(&m) = next {
                doc.pending_select = Some(m);
            }
        }
        if close {
            self.find = None;
            if let Some(d) = self.tabs.current_mut() {
                d.wants_focus = 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds() {
        assert_eq!(super::find_all("Select a, sElEct", "select", false), vec![(0, 6), (10, 16)]);
        assert_eq!(super::find_all("Select a, sElEct", "select", true), vec![]);
        assert_eq!(super::find_all("ééé", "é", false), vec![(0, 1), (1, 2), (2, 3)]);
    }
}
