//! Renders the main screens to PNG with egui_kittest (real UI code, wgpu
//! renderer, no window). Used to review the interface.
//!
//! `ONDALUX_SHOTS=<dir> ONDALUX_DEMO=<demo.lux> cargo test -p ondalux-app ui_shots -- --ignored`

use crate::app::{OndaluxApp, RightTab, Selection, Tool};
use egui_kittest::Harness;
use ondalux_core::ap::{ApId, Mount};
use std::path::PathBuf;
use std::time::Duration;

const SIZE: [f32; 2] = [1400.0, 860.0];

fn harness(path: Option<PathBuf>) -> Harness<'static, OndaluxApp> {
    Harness::builder()
        .with_size(SIZE)
        .wgpu()
        .build_eframe(move |cc| {
            let mut app = OndaluxApp::new(cc, path);
            // ONDALUX_LANG=en renders the English UI (for the website).
            if let Ok(lang) = std::env::var("ONDALUX_LANG") {
                crate::i18n::set_english(lang == "en");
                app.status = None;
            }
            app
        })
}

/// Steps the UI until the background heat map has arrived.
fn settle(h: &mut Harness<'static, OndaluxApp>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.state().heat_ready() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    h.run_steps(4);
}

fn save(h: &mut Harness<'static, OndaluxApp>, dir: &std::path::Path, name: &str) {
    let img = h.render().expect("render UI");
    img.save(dir.join(format!("{name}.png"))).expect("save PNG");
}

#[test]
#[ignore = "needs a GPU; run manually to review the UI"]
fn ui_shots() {
    let dir = PathBuf::from(std::env::var("ONDALUX_SHOTS").expect("set ONDALUX_SHOTS"));
    std::fs::create_dir_all(&dir).unwrap();
    let demo = std::env::var("ONDALUX_DEMO").ok().map(PathBuf::from);

    let mut h = harness(None);
    h.run_steps(4);
    save(&mut h, &dir, "01-welcome");

    let mut h = harness(demo);
    settle(&mut h);
    save(&mut h, &dir, "02-project");

    h.state_mut().selection = Selection::Ap(ApId(1));
    h.run_steps(4);
    save(&mut h, &dir, "03-ap-selected");

    h.state_mut().selection = Selection::None;
    h.state_mut().right_tab = RightTab::Survey;
    h.run_steps(4);
    save(&mut h, &dir, "04-survey");

    h.state_mut().right_tab = RightTab::Inspector;
    h.state_mut().catalog.open = true;
    h.run_steps(4);
    save(&mut h, &dir, "05-catalog");

    h.state_mut().catalog.open = false;
    h.state_mut().export.report_open = true;
    h.run_steps(4);
    save(&mut h, &dir, "06-report");

    h.state_mut().export.report_open = false;
    if let Some(doc) = h.state_mut().doc.as_mut() {
        let ap = &mut doc.project.aps[0];
        ap.set_mount(Mount::Wall);
        ap.orientation_deg = 90.0;
    }
    h.state_mut().selection = Selection::Ap(ApId(1));
    settle(&mut h);
    save(&mut h, &dir, "07-ap-wall");

    // 3D view of the same project.
    h.state_mut().view3d.enabled = true;
    h.run_steps(6);
    save(&mut h, &dir, "13-3d");
    h.state_mut().view3d.yaw = 2.4;
    h.state_mut().view3d.pitch = 0.55;
    h.run_steps(4);
    save(&mut h, &dir, "14-3d-rotated");
    h.state_mut().view3d.enabled = false;
    h.state_mut().view3d.reset();

    h.state_mut().selection = Selection::None;
    h.state_mut().set_tool(Tool::Wall);
    h.state_mut().show_materials = true;
    h.run_steps(4);
    save(&mut h, &dir, "08-materials");

    h.state_mut().show_materials = false;
    h.state_mut().set_tool(crate::app::Tool::Select);
    h.state_mut().selection = Selection::Ap(ApId(2));
    h.ctx.set_theme(eframe::egui::ThemePreference::Light);
    h.run_steps(6);
    save(&mut h, &dir, "09-light");

    // Wall detection proposal over the demo plan (dark theme again).
    h.ctx.set_theme(eframe::egui::ThemePreference::Dark);
    h.state_mut().selection = Selection::None;
    let ctx = h.ctx.clone();
    crate::walls_ui::start(h.state_mut(), &ctx);
    for _ in 0..200 {
        h.run_steps(2);
        if !h.state().walls.preview.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    h.run_steps(4);
    save(&mut h, &dir, "11-detect-walls");

    // Importing a plan proposes walls right away.
    if let Some(plan) = std::env::var("ONDALUX_PLAN").ok().map(PathBuf::from) {
        let mut h = harness(Some(plan));
        if !h.state().walls.open {
            let ctx = h.ctx.clone();
            crate::walls_ui::start(h.state_mut(), &ctx);
        }
        for _ in 0..300 {
            h.run_steps(2);
            if !h.state().walls.preview.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        h.run_steps(6);
        save(&mut h, &dir, "12-import-detect");
    }

    let mut h = harness(None);
    h.ctx.set_theme(eframe::egui::ThemePreference::Light);
    h.run_steps(6);
    save(&mut h, &dir, "10-welcome-light");
}
