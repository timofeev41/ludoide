//! The code editor widget: line-number gutter, highlighting, diagnostics and
//! IDE-style key handling layered on top of egui's `TextEdit`.

use super::{Document, highlight, ops};
use crate::languages;
use crate::lint::Severity;
use crate::theme::Palette;
use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, FontFamily, FontId, Id, Key, Modifiers, Rect, Sense, Stroke, TextEdit, vec2};
use std::hash::{Hash, Hasher};

pub struct EditorEnv<'a> {
    pub palette: &'a Palette,
    pub theme_id: &'static str,
    pub font_size: f32,
    pub indent: usize,
}

pub fn editor_id(doc: &Document) -> Id {
    Id::unique(("editor", doc.uid))
}

fn hash_text(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Apply keyboard shortcuts that egui's `TextEdit` does not provide.
/// Must run before the `TextEdit` is shown so the events are consumed first.
/// Events handled by [`handle_keys`] rather than by `TextEdit`.
fn is_special(e: &egui::Event, has_selection: bool) -> bool {
    match e {
        egui::Event::Key { key, pressed: true, modifiers, .. } => {
            matches!(key, Key::Enter | Key::Tab)
                || (modifiers.alt && modifiers.shift && matches!(key, Key::ArrowUp | Key::ArrowDown))
                || (modifiers.command && *key == Key::Slash)
        }
        egui::Event::Text(t) => {
            has_selection && t.chars().count() == 1 && ops::closing_pair(t.chars().next().unwrap()).is_some()
        }
        _ => false,
    }
}

fn handle_keys(ui: &mut egui::Ui, doc: &mut Document, id: Id, indent: usize) {
    if !ui.memory(|m| m.has_focus(id)) {
        doc.deferred_events.clear();
        return;
    }
    // Replay events held back last frame, ahead of this frame's input.
    if !doc.deferred_events.is_empty() {
        let mut held = std::mem::take(&mut doc.deferred_events);
        ui.input_mut(|i| {
            held.append(&mut i.events);
            i.events = held;
        });
    }
    let Some(mut state) = TextEdit::load_state(ui.ctx(), id) else { return };
    let Some(range) = state.cursor.char_range() else { return };
    let r = range.as_sorted_char_range();
    let sel = (r.start.0, r.end.0);
    // If ordinary typing precedes a special key in this frame, let TextEdit apply
    // the typing first and handle the rest next frame, so edits land in order.
    let split = ui.input(|i| {
        let first = i.events.iter().position(|e| is_special(e, sel.0 != sel.1))?;
        let typed_before = i.events[..first].iter().any(|e| {
            matches!(e, egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Key { pressed: true, .. })
        });
        typed_before.then_some(first)
    });
    if let Some(first) = split {
        doc.deferred_events = ui.input_mut(|i| i.events.split_off(first));
        ui.ctx().request_repaint();
        return;
    }
    let lang = languages::get(doc.lang);
    let alt_shift = Modifiers::ALT | Modifiers::SHIFT;

    let mut edit: Option<ops::Edit> = None;
    ui.input_mut(|i| {
        if i.consume_key(alt_shift, Key::ArrowUp) {
            edit = ops::move_lines(&doc.text, sel, true);
        } else if i.consume_key(alt_shift, Key::ArrowDown) {
            edit = ops::move_lines(&doc.text, sel, false);
        } else if i.consume_key(Modifiers::SHIFT, Key::Tab) {
            edit = Some(ops::outdent_lines(&doc.text, sel, indent));
        } else if i.consume_key(Modifiers::NONE, Key::Tab) {
            edit = Some(if ops::is_multiline(&doc.text, sel) {
                ops::indent_lines(&doc.text, sel, indent)
            } else {
                ops::insert_soft_tab(&doc.text, sel, indent)
            });
        } else if i.consume_key(Modifiers::COMMAND, Key::Slash) {
            if let Some(prefix) = lang.line_comment {
                edit = Some(ops::toggle_comment(&doc.text, sel, prefix));
            }
        } else if i.consume_key(Modifiers::NONE, Key::Enter) {
            edit = Some(ops::newline_with_indent(&doc.text, sel, indent));
        } else if sel.0 != sel.1 {
            // Typing an opening bracket/quote over a selection wraps it.
            let pos = i.events.iter().position(|e| {
                matches!(e, egui::Event::Text(t) if t.chars().count() == 1
                    && ops::closing_pair(t.chars().next().unwrap()).is_some())
            });
            if let Some(pos) = pos {
                if let egui::Event::Text(t) = i.events.remove(pos) {
                    edit = ops::wrap_selection(&doc.text, sel, t.chars().next().unwrap());
                }
            }
        }
    });

    if let Some(e) = edit {
        doc.text = e.text;
        doc.mark_edited();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(e.sel.0), CCursor::new(e.sel.1))));
        state.store(ui.ctx(), id);
    }
}

/// Char index of a 1-based (line, col).
fn char_index(text: &str, line: usize, col: usize) -> usize {
    let mut idx = 0;
    let mut cur_line = 1;
    for c in text.chars() {
        if cur_line == line {
            break;
        }
        idx += 1;
        if c == '\n' {
            cur_line += 1;
        }
    }
    let line_len = text.chars().skip(idx).take_while(|c| *c != '\n').count();
    idx + (col.saturating_sub(1)).min(line_len)
}

pub fn show(ui: &mut egui::Ui, doc: &mut Document, env: &EditorEnv) {
    let id = editor_id(doc);
    if doc.wants_focus > 0 {
        doc.wants_focus -= 1;
        ui.memory_mut(|m| m.request_focus(id));
        ui.ctx().request_repaint();
    }
    handle_keys(ui, doc, id, env.indent);
    if let Some((a, b)) = doc.pending_select.take() {
        let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(ui.ctx(), id);
        doc.sel = (a, b);
        doc.cursor_line_col = ops::line_col(&doc.text, b);
        doc.goto_line = Some(doc.cursor_line_col.0);
    }

    let p = env.palette;
    let font = FontId::new(env.font_size, FontFamily::Monospace);
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let line_count = doc.text.chars().filter(|c| *c == '\n').count() + 1;
    let digits = line_count.to_string().len().max(3);
    let gutter_w = digits as f32 * char_w + 22.0;
    let lang = languages::get(doc.lang);

    // Memoized highlighting, keyed by content, size and theme.
    let key = (hash_text(&doc.text), (env.font_size * 10.0) as u32, env.theme_id);
    let cache_valid = matches!(&doc.highlight_cache, Some((h, s, t, _)) if (*h, *s, *t) == key);
    if !cache_valid {
        let job = highlight::highlight(&doc.text, lang, font.clone(), p);
        doc.highlight_cache = Some((key.0, key.1, key.2, job));
    }
    let base_job = doc.highlight_cache.as_ref().map(|c| c.3.clone()).unwrap_or_default();
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = if buf.as_str().len() == base_job.text.len() && hash_text(buf.as_str()) == key.0 {
            base_job.clone()
        } else {
            highlight::highlight(buf.as_str(), lang, font.clone(), p)
        };
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };

    let goto = doc.goto_line.take();
    let mut scroll = egui::ScrollArea::both().id_salt(("editor_scroll", doc.uid)).auto_shrink([false, false]);
    if let Some(line) = goto {
        let y = (line.saturating_sub(5)) as f32 * row_h;
        scroll = scroll.vertical_scroll_offset(y);
    }
    scroll.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let avail_h = ui.available_height();
            let (gutter_rect, _) =
                ui.allocate_exact_size(vec2(gutter_w, (line_count as f32 * row_h + 8.0).max(avail_h)), Sense::hover());
            let text_origin_y = gutter_rect.top() + 2.0;

            // Current line highlight, painted beneath the text.
            let cursor_line = doc.cursor_line_col.0;
            let line_rect = Rect::from_min_size(
                egui::pos2(gutter_rect.left(), text_origin_y + (cursor_line - 1) as f32 * row_h),
                vec2(ui.available_width() + gutter_w + 4000.0, row_h),
            );
            ui.painter().rect_filled(line_rect, 0.0, p.panel_alt);

            let output = TextEdit::multiline(&mut doc.text)
                .id(id)
                .font(font.clone())
                .code_editor()
                .lock_focus(true)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin { left: 6, right: 8, top: 2, bottom: 2 })
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .min_size(vec2(0.0, avail_h))
                .background_color(Color32::TRANSPARENT)
                .layouter(&mut layouter)
                .show(ui);

            if output.response.changed() {
                doc.mark_edited();
            }
            if let Some(r) = output.cursor_range {
                let rr = r.as_sorted_char_range();
                doc.sel = (rr.start.0, rr.end.0);
                doc.cursor_line_col = ops::line_col(&doc.text, r.primary.index.0);
            }

            // Gutter: line numbers + diagnostic markers.
            let painter = ui.painter();
            painter.rect_filled(gutter_rect, 0.0, p.bg);
            let clip = ui.clip_rect();
            let first = (((clip.top() - text_origin_y) / row_h).floor().max(0.0)) as usize;
            let last = (((clip.bottom() - text_origin_y) / row_h).ceil() as usize + 1).min(line_count);
            let small = FontId::new(env.font_size * 0.9, FontFamily::Monospace);
            for line in first..last {
                let y = text_origin_y + line as f32 * row_h;
                let color = if line + 1 == cursor_line { p.text } else { p.text_dim };
                painter.text(
                    egui::pos2(gutter_rect.right() - 12.0, y + row_h / 2.0),
                    Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    small.clone(),
                    color,
                );
            }
            let hover = ui.input(|i| i.pointer.hover_pos());
            for d in &doc.diagnostics {
                let c = if d.severity == Severity::Error { p.error } else { p.warning };
                let y = text_origin_y + (d.line - 1) as f32 * row_h;
                painter.circle_filled(egui::pos2(gutter_rect.right() - 5.0, y + row_h / 2.0), 2.5, c);
                // Squiggle under the reported token.
                let idx = char_index(&doc.text, d.line, d.col);
                let r = output.galley.pos_from_cursor(CCursor::new(idx)).translate(output.galley_pos.to_vec2());
                let end_x = r.left() + char_w * 6.0;
                let base = r.bottom() - 1.0;
                let mut pts = Vec::new();
                let mut x = r.left();
                let mut up = true;
                while x < end_x {
                    pts.push(egui::pos2(x, if up { base - 1.5 } else { base + 0.5 }));
                    x += 2.0;
                    up = !up;
                }
                painter.add(egui::Shape::line(pts, Stroke::new(1.0, c)));
                let row_rect = Rect::from_min_size(egui::pos2(gutter_rect.left(), y), vec2(gutter_w + 4000.0, row_h));
                if hover.is_some_and(|h| row_rect.contains(h) && h.x < end_x.max(gutter_rect.right())) {
                    egui::Tooltip::always_open(
                        ui.ctx().clone(),
                        ui.layer_id(),
                        Id::unique(("diag", doc.uid)),
                        egui::PopupAnchor::Pointer,
                    )
                    .show(|ui| {
                        ui.colored_label(c, &d.message);
                    });
                }
            }
        });
    });
}
