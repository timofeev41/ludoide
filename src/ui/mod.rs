pub mod bottom;
pub mod chrome;
pub mod db_panel;
pub mod dbt_compile;
pub mod editor_area;
pub mod files;
pub mod find;
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
