//! Font setup: built-in fonts, Phosphor icons and optional system fonts.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

pub use egui_phosphor::regular as icon;

/// Lazily-loaded index of installed system fonts.
#[derive(Default)]
pub struct SystemFonts {
    db: Option<fontdb::Database>,
    families: Vec<String>,
    monospace: Vec<String>,
}

impl SystemFonts {
    fn ensure(&mut self) -> &fontdb::Database {
        if self.db.is_none() {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let mut all = std::collections::BTreeSet::new();
            let mut mono = std::collections::BTreeSet::new();
            for face in db.faces() {
                if let Some((name, _)) = face.families.first() {
                    all.insert(name.clone());
                    if face.monospaced {
                        mono.insert(name.clone());
                    }
                }
            }
            self.families = all.into_iter().collect();
            self.monospace = mono.into_iter().collect();
            self.db = Some(db);
        }
        self.db.as_ref().unwrap()
    }

    pub fn families(&mut self) -> &[String] {
        self.ensure();
        &self.families
    }

    pub fn monospace_families(&mut self) -> &[String] {
        self.ensure();
        &self.monospace
    }

    fn load(&mut self, family: &str) -> Option<FontData> {
        let db = self.ensure();
        let id = db.query(&fontdb::Query { families: &[fontdb::Family::Name(family)], ..Default::default() })?;
        db.with_face_data(id, |data, index| {
            let mut fd = FontData::from_owned(data.to_vec());
            fd.index = index;
            fd
        })
    }
}

pub fn build(system: &mut SystemFonts, ui_font: &str, editor_font: &str) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("phosphor".into(), Arc::new(FontData::from_static(egui_phosphor::bytes::regular::FONT)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let list = fonts.families.entry(family).or_default();
        let at = list.len().min(1);
        list.insert(at, "phosphor".into());
    }
    for (name, family) in [(ui_font, FontFamily::Proportional), (editor_font, FontFamily::Monospace)] {
        if name.is_empty() {
            continue;
        }
        if let Some(data) = system.load(name) {
            let key = format!("sys:{name}");
            fonts.font_data.insert(key.clone(), Arc::new(data));
            fonts.families.entry(family).or_default().insert(0, key);
        }
    }
    fonts
}
