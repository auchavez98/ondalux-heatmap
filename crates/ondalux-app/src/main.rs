//! Ondalux Heatmap desktop application entry point.

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod catalog_ui;
mod document;
mod export_ui;
mod heatmap;
mod panels;
mod survey_ui;
mod view;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Ondalux Heatmap")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_maximized(true),
        ..Default::default()
    };
    eframe::run_native(
        "Ondalux Heatmap",
        options,
        Box::new(|cc| Ok(Box::new(app::OndaluxApp::new(cc, path)))),
    )
}
