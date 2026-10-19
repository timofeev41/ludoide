//! Find in project panel (⌘F): file names and contents everywhere in the workspace.

use super::{dim, search_field, section_header, small_icon_button};
use crate::app::App;
use crate::fonts::icon;
use crate::project_search::{Query, Results, Searcher};
use crate::ui::files::file_icon;
use egui::RichText;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ProjectFind {
    pub query: Query,
    pub focus: bool,
    searcher: Searcher,
    /// Query of the search in flight / shown, its generation, and when the input last changed.
    submitted: Option<Query>,
    generation: u64,
    changed_at: Option<Instant>,
    results: Option<Results>,
    searching: bool,
    /// Bumped by "Collapse all".
    collapse_gen: u64,
}

impl App {
    pub fn open_project_find(&mut self) {
        let selected = self.tabs.current().map(|d| d.selected_text()).unwrap_or_default();
        if !selected.is_empty() && !selected.contains('\n') {
            self.project_find.query.text = selected;
        }
        self.project_find.focus = true;
        self.left_tool = crate::app::LeftTool::Find;
        self.left_open = true;
    }

    pub fn project_find_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            section_header(ui, "Find in project", &p);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if small_icon_button(ui, icon::ARROWS_IN_LINE_VERTICAL, "Collapse all", &p).clicked() {
                    self.project_find.collapse_gen += 1;
                }
                if small_icon_button(ui, icon::ARROWS_CLOCKWISE, "Search again", &p).clicked() {
                    self.project_find.submitted = None;
                }
            });
        });
        let st = &mut self.project_find;
        // Right to left: "Aa" first, then the box takes exactly the width that is left
        // (a fixed guess at the button's width made the panel grow every frame).
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y + 4.0);
        let r = ui
            .allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(&mut st.query.case_sensitive, "Aa").on_hover_text("Match case");
                search_field(ui, &mut st.query.text, &format!("{} Text or file name", icon::MAGNIFYING_GLASS), &p)
            })
            .inner;
        if std::mem::take(&mut st.focus) {
            // Focus with the query selected, so typing replaces it.
            r.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), r.id).unwrap_or_default();
            let n = st.query.text.chars().count();
            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
            state.store(ui.ctx(), r.id);
        }
        search_field(ui, &mut st.query.include, "Files to include: .sql|.yml, models/", &p)
            .on_hover_text("Same syntax as the Explorer filter; empty = all files");
        st.query.show_hidden = self.settings.other.show_hidden_files;

        // Debounced (re)search when the query changes.
        let q = st.query.clone();
        if q.text.is_empty() {
            st.results = None;
            st.submitted = None;
            st.searching = false;
        } else if st.submitted.as_ref() != Some(&q) {
            let since = *st.changed_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(200) {
                let mut roots = self.workspace.roots.clone();
                roots.push(self.settings.other.scratches_dir.clone());
                st.generation = st.searcher.submit(roots, q.clone(), ctx.clone());
                st.submitted = Some(q);
                st.searching = true;
                st.changed_at = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(200));
            }
        }
        if let Some(r) = st.searcher.poll(st.generation) {
            st.results = Some(r);
            st.searching = false;
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if st.searching {
                ui.spinner();
            }
            if let Some(r) = &st.results {
                let text = match (r.files.len(), r.line_hits) {
                    (0, _) => "No results".to_owned(),
                    (f, 0) => format!("{f} file name match(es)"),
                    (f, l) => format!("{l} result(s) in {f} file(s)"),
                };
                ui.label(dim(text, &p).small());
                if r.truncated {
                    ui.label(RichText::new("· showing the first 5000").small().color(p.warning));
                }
            }
        });
        ui.add_space(2.0);

        let mut open: Option<(std::path::PathBuf, Option<(usize, usize)>)> = None;
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let small_font = egui::FontId::new(font.size - 1.0, font.family.clone());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let Some(res) = &self.project_find.results else { return };
            let open_gen = (self.project_find.collapse_gen, self.project_find.generation);
            for f in &res.files {
                let name = f.path.file_name().unwrap_or_default().to_string_lossy();
                let rel_dir = f
                    .path
                    .parent()
                    .and_then(|d| d.strip_prefix(f.root.parent().unwrap_or(&f.root)).ok())
                    .map(|d| d.display().to_string())
                    .unwrap_or_default();
                if f.lines.is_empty() {
                    // Matched by name only.
                    let r =
                        ui.add(egui::Button::selectable(false, format!("{} {name}", file_icon(&f.path))).truncate());
                    let r = r.on_hover_text(f.path.display().to_string());
                    if r.clicked() {
                        open = Some((f.path.clone(), None));
                    }
                    ui.add(egui::Label::new(dim(format!("    {rel_dir}"), &p).small()).truncate());
                    continue;
                }
                let mut title = LayoutJob::default();
                let body = egui::TextStyle::Body.resolve(ui.style());
                let name_color = if f.name_match { p.accent } else { p.text };
                title.append(
                    &format!("{} {name}  ", file_icon(&f.path)),
                    0.0,
                    TextFormat::simple(body.clone(), name_color),
                );
                title.append(&format!("{}  ", f.lines.len()), 0.0, TextFormat::simple(body.clone(), p.text_dim));
                let small_body = egui::TextStyle::Small.resolve(ui.style());
                title.append(&rel_dir, 0.0, TextFormat::simple(small_body, p.text_dim));
                // Custom header so a long title is cut instead of widening the panel.
                let id = ui.make_persistent_id(("find_file", &f.path, open_gen));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                    .show_header(ui, |ui| {
                        let r = ui.add(egui::Label::new(title).truncate().sense(egui::Sense::click()));
                        if r.clicked() {
                            open = Some((f.path.clone(), None));
                        }
                        r.on_hover_text(format!("{}\nClick to open", f.path.display()));
                    })
                    .body(|ui| {
                        for h in &f.lines {
                            let mut job = LayoutJob::default();
                            job.append(
                                &format!("{:>4}  ", h.line),
                                0.0,
                                TextFormat::simple(small_font.clone(), p.text_dim),
                            );
                            let chars: Vec<char> = h.preview.chars().collect();
                            let (a, b) = (h.preview_range.0.min(chars.len()), h.preview_range.1.min(chars.len()));
                            let s = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
                            job.append(&s(0..a), 0.0, TextFormat::simple(small_font.clone(), p.text));
                            let mut hit = TextFormat::simple(small_font.clone(), p.text);
                            hit.background = p.accent.gamma_multiply(0.35);
                            job.append(&s(a..b), 0.0, hit);
                            job.append(&s(b..chars.len()), 0.0, TextFormat::simple(small_font.clone(), p.text));
                            job.wrap.max_rows = 1;
                            job.wrap.max_width = ui.available_width();
                            if ui
                                .add(egui::Button::new(job).frame(false).wrap_mode(egui::TextWrapMode::Truncate))
                                .clicked()
                            {
                                open = Some((f.path.clone(), Some(h.range)));
                            }
                        }
                    });
            }
        });
        if let Some((path, range)) = open {
            self.open_file(&path);
            if let Some(doc) = self.tabs.current_mut().filter(|d| d.path.as_deref() == Some(path.as_path())) {
                if let Some(r) = range {
                    doc.pending_select = Some(r);
                }
                doc.wants_focus = 3;
            }
        }
    }
}
