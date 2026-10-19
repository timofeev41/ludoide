//! Themes. Adding a theme = adding a [`Theme`] to [`THEMES`].

use egui::{Color32, CornerRadius, Stroke, Visuals};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub panel_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub hover: Color32,
    pub error: Color32,
    pub warning: Color32,
    pub success: Color32,
    pub syn_keyword: Color32,
    pub syn_builtin: Color32,
    pub syn_string: Color32,
    pub syn_number: Color32,
    pub syn_comment: Color32,
    pub syn_key: Color32,
    pub syn_punct: Color32,
    pub ansi: [Color32; 16],
}

pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub palette: Palette,
}

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub static THEMES: &[Theme] = &[
    Theme {
        id: "dark",
        name: "Dark",
        palette: Palette {
            dark: true,
            bg: c(0x1e1f22),
            panel: c(0x2b2d30),
            panel_alt: c(0x25272a),
            border: c(0x393b40),
            text: c(0xdfe1e5),
            text_dim: c(0x8c8f96),
            accent: c(0x3574f0),
            selection: c(0x214283),
            hover: c(0x35373b),
            error: c(0xf75464),
            warning: c(0xe0b25a),
            success: c(0x5fb865),
            syn_keyword: c(0xcf8e6d),
            syn_builtin: c(0x56a8f5),
            syn_string: c(0x6aab73),
            syn_number: c(0x2aacb8),
            syn_comment: c(0x7a7e85),
            syn_key: c(0xc77dbb),
            syn_punct: c(0xbcbec4),
            ansi: [
                c(0x1e1f22),
                c(0xf75464),
                c(0x5fb865),
                c(0xe0b25a),
                c(0x3574f0),
                c(0xc77dbb),
                c(0x2aacb8),
                c(0xdfe1e5),
                c(0x6f737a),
                c(0xff7b86),
                c(0x7fd187),
                c(0xf2c97a),
                c(0x6c9cff),
                c(0xe39fd6),
                c(0x5ccfd9),
                c(0xffffff),
            ],
        },
    },
    Theme {
        id: "light",
        name: "Light",
        palette: Palette {
            dark: false,
            bg: c(0xffffff),
            panel: c(0xf7f8fa),
            panel_alt: c(0xf0f1f3),
            border: c(0xdfe1e5),
            text: c(0x1e1f22),
            text_dim: c(0x6c707e),
            accent: c(0x3574f0),
            selection: c(0xc5d8fc),
            hover: c(0xe8eaed),
            error: c(0xd8323c),
            warning: c(0xb57f12),
            success: c(0x2e8a3a),
            syn_keyword: c(0x0033b3),
            syn_builtin: c(0x00627a),
            syn_string: c(0x067d17),
            syn_number: c(0x1750eb),
            syn_comment: c(0x8c8c8c),
            syn_key: c(0x871094),
            syn_punct: c(0x3f4246),
            ansi: [
                c(0x1e1f22),
                c(0xd8323c),
                c(0x2e8a3a),
                c(0xb57f12),
                c(0x3574f0),
                c(0x871094),
                c(0x00827f),
                c(0xbfc2c8),
                c(0x6c707e),
                c(0xf05058),
                c(0x40a84e),
                c(0xd59a20),
                c(0x5a8ef7),
                c(0xa63cb0),
                c(0x16a3a0),
                c(0xffffff),
            ],
        },
    },
];

pub fn palette(id: &str) -> Palette {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0]).palette
}

/// Flat, borderless egui visuals derived from a palette.
pub fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark { Visuals::dark() } else { Visuals::light() };
    let r = CornerRadius::same(3);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.panel_alt;
    v.code_bg_color = p.panel_alt;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(6);
    v.menu_corner_radius = CornerRadius::same(4);
    v.window_shadow.spread = 0;
    v.popup_shadow.spread = 0;
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0, p.text);
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.error;
    v.warn_fg_color = p.warning;
    v.override_text_color = Some(p.text);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    for (ws, fill) in [
        (&mut w.inactive, p.panel_alt),
        (&mut w.hovered, p.hover),
        (&mut w.active, p.selection),
        (&mut w.open, p.hover),
    ] {
        ws.bg_fill = fill;
        ws.weak_bg_fill = fill;
        ws.bg_stroke = Stroke::NONE;
        ws.fg_stroke = Stroke::new(1.0, p.text);
        ws.corner_radius = r;
        ws.expansion = 0.0;
    }
    // Buttons stay flat (weak fill); checkboxes/radios/sliders keep a visible fill.
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = p.hover;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    v
}
