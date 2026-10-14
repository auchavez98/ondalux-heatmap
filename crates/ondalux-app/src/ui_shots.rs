//! Renders the main screens to PNG with egui_kittest (real UI code, wgpu
//! renderer, no window). Used to review the interface.
//!
//! `ONDALUX_SHOTS=<dir> ONDALUX_DEMO=<demo.lux> cargo test -p ondalux-app ui_shots -- --ignored`

use crate::app::{OndaluxApp, RightTab, Selection};
use egui_kittest::Harness;
use ondalux_core::ap::ApId;
use std::path::PathBuf;
use std::time::Duration;

const SIZE: [f32; 2] = [1400.0, 860.0];

fn harness(path: Option<PathBuf>) -> Harness<'static, OndaluxApp> {
    Harness::builder()
        .with_size(SIZE)
        .wgpu()
        .build_eframe(move |cc| OndaluxApp::new(cc, path))
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
}
