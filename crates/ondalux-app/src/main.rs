//! Ondalux Heatmap desktop application entry point.

// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod catalog_ui;
mod document;
mod export_ui;
mod floors_ui;
mod heatmap;
mod i18n;
mod panels;
mod style;
mod survey_ui;
#[cfg(test)]
mod ui_shots;
mod update;
mod view;
mod view3d;
mod walls_ui;

fn main() -> eframe::Result {
    let path = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Ondalux Heatmap")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0])
            .with_maximized(true)
            .with_icon(eframe::egui::IconData {
                rgba: ondalux_report::logo::rgba(256),
                width: 256,
                height: 256,
            }),
        ..Default::default()
    };
    eframe::run_native(
        "Ondalux Heatmap",
        options,
        Box::new(|cc| Ok(Box::new(app::OndaluxApp::new(cc, path)))),
    )
}
