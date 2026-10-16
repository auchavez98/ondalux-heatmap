//! Renders a promotional video of the real UI with egui_kittest: scripted
//! scenes, frames piped as raw RGBA to ffmpeg. Captions, title cards and
//! music are added afterwards.
//!
//! `ONDALUX_VIDEO=<dir> ONDALUX_FFMPEG=<ffmpeg.exe> ONDALUX_DEMO=<demo.lux>
//! ONDALUX_PLAN=<plan.pdf> ONDALUX_LANG=en cargo test -p ondalux-app video -- --ignored`
//!
//! Writes `<dir>/scenes.mp4` and `<dir>/scenes.json` (start and end time of
//! each scene, for the captions).

use crate::app::{OndaluxApp, RightTab, Selection, Tool};
use egui_kittest::Harness;
use ondalux_core::ap::{ApId, Mount};
use ondalux_core::geometry::Vec2;
use ondalux_core::metrics::MetricKind;
use ondalux_core::project::Floor;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const FPS: usize = 30;
/// Logical size; rendered at 1.25 px per point = 1920 × 1080.
const SIZE: [f32; 2] = [1536.0, 864.0];
const PPP: f32 = 1.25;

struct Recorder {
    ffmpeg: Child,
    frames: usize,
    scenes: Vec<(String, f32, f32)>,
}

impl Recorder {
    fn new(ffmpeg: &str, out: &std::path::Path) -> Self {
        let child = Command::new(ffmpeg)
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
            ])
            .args(["-s", "1920x1080", "-r", &FPS.to_string(), "-i", "-"])
            .args([
                "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-pix_fmt", "yuv420p",
            ])
            .arg(out)
            .stdin(Stdio::piped())
            .spawn()
            .expect("start ffmpeg");
        Self {
            ffmpeg: child,
            frames: 0,
            scenes: Vec::new(),
        }
    }

    fn frame(&mut self, h: &mut Harness<'static, OndaluxApp>) {
        h.run_steps(1);
        let img = h.render().expect("render");
        assert_eq!(img.dimensions(), (1920, 1080), "frame size");
        self.ffmpeg
            .stdin
            .as_mut()
            .expect("ffmpeg stdin")
            .write_all(img.as_raw())
            .expect("write frame");
        self.frames += 1;
    }

    fn now(&self) -> f32 {
        self.frames as f32 / FPS as f32
    }

    /// Records `seconds` of frames, calling `step(h, t)` with t in 0..1
    /// before each one.
    fn scene(
        &mut self,
        name: &str,
        h: &mut Harness<'static, OndaluxApp>,
        seconds: f32,
        mut step: impl FnMut(&mut Harness<'static, OndaluxApp>, f32),
    ) {
        let start = self.now();
        let n = (seconds * FPS as f32).round() as usize;
        for i in 0..n {
            step(h, i as f32 / (n - 1).max(1) as f32);
            // Give the background heat map worker time to keep up.
            std::thread::sleep(Duration::from_millis(12));
            self.frame(h);
        }
        self.scenes.push((name.to_owned(), start, self.now()));
    }

    fn finish(mut self, dir: &std::path::Path) {
        drop(self.ffmpeg.stdin.take());
        let status = self.ffmpeg.wait().expect("ffmpeg");
        assert!(status.success(), "ffmpeg failed");
        let json: Vec<_> = self
            .scenes
            .iter()
            .map(|(n, a, b)| serde_json::json!({"name": n, "start": a, "end": b}))
            .collect();
        std::fs::write(
            dir.join("scenes.json"),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();
    }
}

fn harness(path: Option<PathBuf>) -> Harness<'static, OndaluxApp> {
    Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(PPP)
        .wgpu()
        .build_eframe(move |cc| {
            let mut app = OndaluxApp::new(cc, path);
            if let Ok(lang) = std::env::var("ONDALUX_LANG") {
                crate::i18n::set_english(lang == "en");
            }
            app.status = None;
            app
        })
}

fn settle(h: &mut Harness<'static, OndaluxApp>) {
    for _ in 0..300 {
        h.run_steps(2);
        if h.state().heat_ready() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(6);
}

/// Smooth start and stop.
fn ease(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[test]
#[ignore = "needs a GPU and ffmpeg; run manually to make the video"]
fn video() {
    let dir = PathBuf::from(std::env::var("ONDALUX_VIDEO").expect("set ONDALUX_VIDEO"));
    let ffmpeg = std::env::var("ONDALUX_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let demo = PathBuf::from(std::env::var("ONDALUX_DEMO").expect("set ONDALUX_DEMO"));
    let plan = std::env::var("ONDALUX_PLAN").ok().map(PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rec = Recorder::new(&ffmpeg, &dir.join("scenes.mp4"));

    let mut h = harness(Some(demo));
    settle(&mut h);
    h.state_mut().status = None;
    let ap1 = h.state().doc.as_ref().unwrap().project.aps[0].position;

    // 1. The project.
    rec.scene("overview", &mut h, 4.0, |_, _| {});

    // 2. Drag an AP around: the heat map follows.
    h.state_mut().selection = Selection::Ap(ApId(1));
    rec.scene("drag", &mut h, 7.0, |h, t| {
        let a = ease(t) * std::f32::consts::TAU;
        if let Some(doc) = h.state_mut().doc.as_mut() {
            doc.project.aps[0].position =
                Vec2::new(ap1.x + 6.0 * a.sin(), ap1.y + 3.5 * (1.0 - a.cos()));
        }
    });
    if let Some(doc) = h.state_mut().doc.as_mut() {
        doc.project.aps[0].position = ap1;
    }

    // 3. Heat map types.
    h.state_mut().selection = Selection::None;
    let metrics = [
        MetricKind::Rssi,
        MetricKind::Snr,
        MetricKind::Sinr,
        MetricKind::ApCount,
        MetricKind::Coverage,
    ];
    rec.scene("metrics", &mut h, 7.5, |h, t| {
        let i = ((t * metrics.len() as f32) as usize).min(metrics.len() - 1);
        h.state_mut().heat.metric = metrics[i];
    });
    h.state_mut().heat.metric = MetricKind::Rssi;

    // 4. A wall-mounted AP aimed around.
    h.state_mut().selection = Selection::Ap(ApId(1));
    if let Some(doc) = h.state_mut().doc.as_mut() {
        doc.project.aps[0].set_mount(Mount::Wall);
    }
    rec.scene("wall-ap", &mut h, 6.0, |h, t| {
        if let Some(doc) = h.state_mut().doc.as_mut() {
            doc.project.aps[0].orientation_deg = lerp(20.0, 200.0, ease(t));
        }
    });
    if let Some(doc) = h.state_mut().doc.as_mut() {
        doc.project.aps[0].set_mount(Mount::Ceiling);
    }

    // 5. 3D view, orbiting.
    h.state_mut().selection = Selection::None;
    h.state_mut().view3d.enabled = true;
    h.run_steps(4);
    rec.scene("3d", &mut h, 9.0, |h, t| {
        let v = &mut h.state_mut().view3d;
        v.yaw = lerp(0.35, 0.35 + 2.4, ease(t));
        v.pitch = lerp(0.95, 0.55, ease(t));
    });

    // 6. A second floor: 2D with the floor below, then both in 3D.
    if let Some(doc) = h.state_mut().doc.as_mut() {
        let p = &doc.project;
        let mut floor = Floor::new(Default::default(), 0, 0);
        floor.plan = p.plan.clone();
        floor.walls = p.walls.clone();
        floor.aps = p
            .aps
            .iter()
            .take(2)
            .map(|a| {
                let mut a = a.clone();
                a.id = ApId(a.id.0 + 100);
                a.name = format!("{}-F2", a.name);
                a.position = a.position + Vec2::new(4.0, 3.0);
                a
            })
            .collect();
        let assets = (**doc.assets()).clone();
        doc.add_floors(1, vec![(floor, assets)]);
    }
    h.state_mut().view3d.enabled = false;
    settle(&mut h);
    rec.scene("floor-2d", &mut h, 4.0, |_, _| {});
    h.state_mut().view3d.enabled = true;
    h.state_mut().view3d.reset();
    rec.scene("floors-3d", &mut h, 8.0, |h, t| {
        let v = &mut h.state_mut().view3d;
        v.yaw = lerp(-0.6, 0.9, ease(t));
        v.pitch = lerp(0.35, 0.6, ease(t));
        v.distance = 0.0;
    });
    h.state_mut().view3d.enabled = false;
    h.state_mut().view3d.reset();
    if let Some(doc) = h.state_mut().doc.as_mut() {
        doc.remove_floor(1);
    }
    settle(&mut h);

    // 7. Report export. (No catalog window: it shows the local path of
    // the custom models file.)
    h.state_mut().export.lang = if crate::i18n::is_english() {
        ondalux_report::Lang::En
    } else {
        ondalux_report::Lang::Es
    };
    h.state_mut().export.report_open = true;
    rec.scene("report", &mut h, 3.5, |_, _| {});
    h.state_mut().export.report_open = false;
    let _ = (RightTab::Inspector, Tool::Select);

    // 8. Wall detection on an imported PDF.
    if let Some(plan) = plan {
        let mut h = harness(Some(plan));
        if !h.state().walls.open {
            let ctx = h.ctx.clone();
            crate::walls_ui::start(h.state_mut(), &ctx);
        }
        for _ in 0..400 {
            h.run_steps(2);
            if !h.state().walls.preview.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        h.state_mut().status = None;
        h.run_steps(6);
        rec.scene("detect", &mut h, 5.0, |_, _| {});
    }

    rec.finish(&dir);
}
