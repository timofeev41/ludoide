pub mod bottom;
pub mod chrome;
pub mod db_panel;
pub mod dbt_compile;
pub mod editor_area;
pub mod files;
pub mod find;
pub mod git_panel;
pub mod project_find;
pub mod python;
pub mod settings_window;

use crate::theme::Palette;
use egui::{Button, Color32, Response, RichText, Ui};

/// Flat icon button with a tooltip.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str, active: bool, p: &Palette) -> Response {
    let color = if active { p.text } else { p.text_dim };
    let size = ui.style().text_styles[&egui::TextStyle::Body].size + 4.0;
    let btn = Button::new(RichText::new(icon).size(size).color(color))
        .frame(false)
        .min_size(egui::vec2(size + 12.0, size + 10.0));
    let r = ui.add(btn).on_hover_text(tip);
    if active {
        let rect = r.rect;
        ui.painter().rect_filled(rect.shrink(1.0), 4.0, p.hover.gamma_multiply(0.9));
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(size), p.text);
    }
    r
}

pub fn small_icon_button(ui: &mut Ui, icon: &str, tip: &str, p: &Palette) -> Response {
    ui.add(Button::new(RichText::new(icon).color(p.text_dim)).frame(false)).on_hover_text(tip)
}

pub fn section_header(ui: &mut Ui, text: &str, p: &Palette) {
    ui.label(RichText::new(text.to_uppercase()).small().strong().color(p.text_dim));
}

pub fn dim(text: impl Into<String>, p: &Palette) -> RichText {
    RichText::new(text.into()).color(p.text_dim)
}

pub fn panel_frame(fill: Color32, margin: i8) -> egui::Frame {
    egui::Frame::new().fill(fill).inner_margin(egui::Margin::same(margin))
}

/// Single-line search box with a clear (×) button inside it once it has text.
pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, p: &Palette) -> Response {
    let mut r =
        ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(egui::Margin {
            left: 4,
            right: 22,
            top: 2,
            bottom: 2,
        }));
    if !text.is_empty() {
        let size = r.rect.height() - 4.0;
        let rect = egui::Rect::from_center_size(
            egui::pos2(r.rect.right() - size / 2.0 - 2.0, r.rect.center().y),
            egui::vec2(size, size),
        );
        let clear =
            ui.put(rect, Button::new(RichText::new(crate::fonts::icon::X).small().color(p.text_dim)).frame(false));
        if clear.on_hover_text("Clear (Esc)").clicked()
            || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)))
        {
            text.clear();
            r.mark_changed();
        }
    }
    r
}

/// The ways a path can be copied, each with a preview of the result.
pub fn path_variants(path: &std::path::Path, root: Option<&std::path::Path>) -> Vec<(&'static str, String)> {
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut out = vec![("Absolute path", path.display().to_string())];
    if let Some(root) = root.filter(|r| path.starts_with(r) && *r != path) {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let project = root.file_name().unwrap_or_default().to_string_lossy();
        out.push(("From project", format!("{project}/{}", rel.display())));
        out.push(("Relative to project", rel.display().to_string()));
    }
    out.push(("Name", name));
    if path.extension().is_some() {
        out.push(("Name without extension", path.file_stem().unwrap_or_default().to_string_lossy().into_owned()));
    }
    out
}

/// "Copy path ▸" submenu; returns the chosen text.
pub fn copy_path_menu(ui: &mut Ui, variants: &[(&'static str, String)]) -> Option<String> {
    let mut chosen = None;
    ui.menu_button("Copy path", |ui| {
        for (label, value) in variants {
            let r = ui.add(Button::new(*label).right_text(RichText::new(short(value, 48)).small().weak()));
            if r.on_hover_text(value).clicked() {
                chosen = Some(value.clone());
            }
        }
    });
    chosen
}

/// Shortened with a `…` in the middle, keeping both ends of a path.
pub fn short(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_owned();
    }
    let head = max / 3;
    let tail = max - head - 1;
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}
