//! Background heat map computation and color mapping.
//!
//! The UI thread sends the current inputs whenever they change; the worker
//! always processes only the most recent request, keeps the per-radio layer
//! cache between requests and sends back a colored image plus the metrics.

use eframe::egui::{Color32, ColorImage, Context};
use heatcraft_core::ap::AccessPoint;
use heatcraft_core::channel::Band;
use heatcraft_core::grid::{GridSpec, LayerCache};
use heatcraft_core::material::{MaterialLibrary, Wall};
use heatcraft_core::metrics::{MetricKind, Metrics, Requirements};
use heatcraft_core::predict::Obstacles;
use heatcraft_core::propagation::PropagationSettings;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub walls: Vec<Wall>,
    pub materials: MaterialLibrary,
    pub settings: PropagationSettings,
    pub aps: Vec<AccessPoint>,
    pub requirements: Requirements,
    pub grid: GridSpec,
    pub band: Band,
    pub metric: MetricKind,
}

pub struct Output {
    id: u64,
    pub image: ColorImage,
    pub metrics: Arc<Metrics>,
    pub elapsed: Duration,
    pub layers_recomputed: usize,
}

pub struct Worker {
    tx: Sender<(u64, Inputs)>,
    rx: Receiver<Output>,
    last_sent: Option<Inputs>,
    sent_id: u64,
    done_id: u64,
}

impl Worker {
    pub fn spawn(ctx: Context) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<(u64, Inputs)>();
        let (out_tx, out_rx) = mpsc::channel::<Output>();
        std::thread::Builder::new()
            .name("heatmap".into())
            .spawn(move || run(job_rx, out_tx, ctx))
            .expect("failed to spawn heat map thread");
        Self {
            tx: job_tx,
            rx: out_rx,
            last_sent: None,
            sent_id: 0,
            done_id: 0,
        }
    }

    /// Sends `inputs` if they differ from the last request.
    pub fn request(&mut self, inputs: Inputs) {
        if self.last_sent.as_ref() == Some(&inputs) {
            return;
        }
        self.last_sent = Some(inputs.clone());
        self.sent_id += 1;
        let _ = self.tx.send((self.sent_id, inputs));
    }

    /// Forgets the last request so the next one is always sent.
    pub fn reset(&mut self) {
        self.last_sent = None;
    }

    /// Latest finished result, if any arrived since the last call.
    pub fn poll(&mut self) -> Option<Output> {
        let mut latest = None;
        while let Ok(out) = self.rx.try_recv() {
            latest = Some(out);
        }
        if let Some(out) = &latest {
            self.done_id = out.id;
        }
        latest
    }

    /// True while a request is still being computed.
    pub fn busy(&self) -> bool {
        self.done_id < self.sent_id
    }
}

struct WorkerState {
    obstacles_key: Option<(Vec<Wall>, MaterialLibrary)>,
    obstacles: Option<Obstacles>,
    settings: Option<PropagationSettings>,
    cache: Option<LayerCache>,
}

fn run(rx: Receiver<(u64, Inputs)>, tx: Sender<Output>, ctx: Context) {
    let mut st = WorkerState {
        obstacles_key: None,
        obstacles: None,
        settings: None,
        cache: None,
    };
    while let Ok((mut id, mut job)) = rx.recv() {
        // Skip stale requests: only the newest one matters.
        while let Ok((newer_id, newer)) = rx.try_recv() {
            (id, job) = (newer_id, newer);
        }
        let start = Instant::now();
        let key = (job.walls.clone(), job.materials.clone());
        let mut invalidate = false;
        if st.obstacles_key.as_ref() != Some(&key) {
            st.obstacles = Some(Obstacles::new(&job.walls, &job.materials));
            st.obstacles_key = Some(key);
            invalidate = true;
        }
        if st.settings.as_ref() != Some(&job.settings) {
            st.settings = Some(job.settings.clone());
            invalidate = true;
        }
        if st.cache.as_ref().map(|c| *c.grid()) != Some(job.grid) {
            st.cache = Some(LayerCache::new(job.grid));
        }
        let cache = st.cache.as_mut().expect("cache set above");
        if invalidate {
            cache.invalidate_all();
        }
        let obstacles = st.obstacles.as_ref().expect("obstacles set above");
        let recomputed = cache.sync(&job.aps, obstacles, &job.settings);
        let metrics = Metrics::compute(
            job.grid,
            &cache.layers(),
            Some(job.band),
            &job.settings,
            job.requirements,
        );
        let image = colorize(&metrics, job.metric);
        let out = Output {
            id,
            image,
            metrics: Arc::new(metrics),
            elapsed: start.elapsed(),
            layers_recomputed: recomputed,
        };
        if tx.send(out).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// Color stops `(value, color)`, ascending by value.
type Stops = &'static [(f32, [u8; 3])];

const RED: [u8; 3] = [215, 48, 39];
const ORANGE: [u8; 3] = [244, 140, 50];
const YELLOW: [u8; 3] = [240, 220, 70];
const LIME: [u8; 3] = [160, 210, 80];
const GREEN: [u8; 3] = [50, 170, 80];
const DARK_GREEN: [u8; 3] = [20, 110, 60];
const TEAL: [u8; 3] = [40, 160, 170];
const BLUE: [u8; 3] = [60, 100, 200];

const RSSI_STOPS: Stops = &[
    (-90.0, RED),
    (-80.0, ORANGE),
    (-72.0, YELLOW),
    (-67.0, LIME),
    (-60.0, GREEN),
    (-45.0, DARK_GREEN),
];
const SNR_STOPS: Stops = &[
    (5.0, RED),
    (12.0, ORANGE),
    (20.0, YELLOW),
    (25.0, LIME),
    (32.0, GREEN),
    (45.0, DARK_GREEN),
];
const COUNT_STOPS: Stops = &[
    (0.0, RED),
    (1.0, YELLOW),
    (2.0, GREEN),
    (3.0, TEAL),
    (4.0, BLUE),
];
const OVERLAP_STOPS: Stops = &[(0.0, GREEN), (1.0, YELLOW), (2.0, ORANGE), (3.0, RED)];
const COVERAGE_STOPS: Stops = &[(0.0, RED), (1.0, GREEN)];

/// Values below this RSSI are drawn transparent (no usable signal).
const RSSI_CUTOFF: f32 = -95.0;

pub fn stops(kind: MetricKind) -> Stops {
    match kind {
        MetricKind::Rssi => RSSI_STOPS,
        MetricKind::Snr | MetricKind::Sinr => SNR_STOPS,
        MetricKind::ApCount => COUNT_STOPS,
        MetricKind::ChannelOverlap => OVERLAP_STOPS,
        MetricKind::Coverage => COVERAGE_STOPS,
    }
}

/// Whether the metric is discrete (drawn with hard color steps).
pub fn is_discrete(kind: MetricKind) -> bool {
    matches!(
        kind,
        MetricKind::ApCount | MetricKind::ChannelOverlap | MetricKind::Coverage
    )
}

pub fn color_at(kind: MetricKind, v: f32) -> Color32 {
    let s = stops(kind);
    let rgb = if is_discrete(kind) {
        s.iter()
            .rev()
            .find(|(t, _)| v >= *t - 0.5)
            .map_or(s[0].1, |(_, c)| *c)
    } else if v <= s[0].0 {
        s[0].1
    } else if v >= s[s.len() - 1].0 {
        s[s.len() - 1].1
    } else {
        let i = s.windows(2).position(|w| v < w[1].0).unwrap_or(0);
        let ((a, ca), (b, cb)) = (s[i], s[i + 1]);
        let t = (v - a) / (b - a);
        let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        [lerp(ca[0], cb[0]), lerp(ca[1], cb[1]), lerp(ca[2], cb[2])]
    };
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn colorize(m: &Metrics, kind: MetricKind) -> ColorImage {
    let pixels = (0..m.grid.len())
        .map(|i| {
            let unusable =
                m.serving[i] == heatcraft_core::metrics::NO_SERVER || m.rssi[i] < RSSI_CUTOFF;
            if unusable && kind != MetricKind::Coverage && kind != MetricKind::ApCount {
                Color32::TRANSPARENT
            } else {
                color_at(kind, m.value(kind, i))
            }
        })
        .collect();
    ColorImage::new([m.grid.cols, m.grid.rows], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_scale_interpolates_and_clamps() {
        assert_eq!(
            color_at(MetricKind::Rssi, -200.0),
            Color32::from_rgb(215, 48, 39)
        );
        assert_eq!(
            color_at(MetricKind::Rssi, -20.0),
            Color32::from_rgb(20, 110, 60)
        );
        let mid = color_at(MetricKind::Rssi, -85.0);
        assert!(mid.r() < 244 && mid.r() > 215);
    }

    #[test]
    fn discrete_scale_steps() {
        assert_eq!(
            color_at(MetricKind::ApCount, 0.0),
            Color32::from_rgb(215, 48, 39)
        );
        assert_eq!(
            color_at(MetricKind::ApCount, 2.0),
            Color32::from_rgb(50, 170, 80)
        );
        assert_eq!(
            color_at(MetricKind::ApCount, 9.0),
            Color32::from_rgb(60, 100, 200)
        );
        assert_eq!(
            color_at(MetricKind::Coverage, 1.0),
            Color32::from_rgb(50, 170, 80)
        );
    }
}
