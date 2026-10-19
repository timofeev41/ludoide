#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod dbt;
mod editor;
mod fonts;
mod git;
mod languages;
mod lint;
mod secrets;
mod settings;
mod terminal;
mod theme;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ludo")
            .with_inner_size([1400.0, 880.0])
            .with_min_inner_size([720.0, 420.0]),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("Ludo", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
