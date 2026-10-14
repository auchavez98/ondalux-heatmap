//! Background heat map computation and color mapping.
//!
//! The UI thread sends the current inputs whenever they change; the worker
//! always processes only the most recent request. It keeps the predicted
//! per-radio layer cache and the interpolated survey layers between
//! requests, and sends back a colored image plus the displayed values.

use eframe::egui::{Color32, ColorImage, Context};
use ondalux_core::ap::AccessPoint;
use ondalux_core::channel::Band;
use ondalux_core::grid::{GridSpec, LayerCache};
use ondalux_core::interpolate::Variogram;
use ondalux_core::material::{MaterialLibrary, Wall};
use ondalux_core::measured::{self, MeasuredMap, MeasuredSettings};
use ondalux_core::metrics::{MetricKind, Metrics, Requirements, NO_SERVER};
use ondalux_core::predict::Obstacles;
use ondalux_core::propagation::PropagationSettings;
use ondalux_core::survey::{Measurement, Survey};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Which data the heat map shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Predicted,
    Measured,
    /// Measured − predicted.
    Difference,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Predicted, Source::Measured, Source::Difference];
}

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
    pub source: Source,
    /// Active survey, if any.
    pub survey: Option<Survey>,
    pub measured: MeasuredSettings,
    /// In difference mode, cells with |Δ| below this are hidden (0 = show all).
    pub diff_threshold: f32,
}

pub use ondalux_core::colormap::ColorScale as Scale;
use ondalux_core::colormap::RSSI_CUTOFF;

/// The values currently displayed (`NaN` = no data / transparent).
pub struct Field {
    pub grid: GridSpec,
    pub values: Vec<f32>,
    pub scale: Scale,
    pub source: Source,
    /// Share of cells (with data) meeting the coverage requirements.
    pub coverage: Option<f32>,
}

impl Field {
    pub fn value_at(&self, col: usize, row: usize) -> Option<f32> {
        if col >= self.grid.cols || row >= self.grid.rows {
            return None;
        }
        let v = self.values[row * self.grid.cols + col];
        (!v.is_nan()).then_some(v)
    }
}

/// Measured vs predicted best signal at the survey points.
#[derive(Debug, Clone, Default)]
pub struct Comparison {
    /// `(measurement id, measured − predicted)` in dB.
    pub points: Vec<(u32, f32)>,
    pub mean: f32,
    pub rmse: f32,
}

pub struct Output {
    id: u64,
    pub image: ColorImage,
    pub field: Arc<Field>,
    pub comparison: Option<Comparison>,
    /// Radios inferred from the survey (after SSID filtering).
    pub measured_radios: usize,
    pub variogram: Option<Variogram>,
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

type MeasuredKey = (Vec<Measurement>, MeasuredSettings, GridSpec);

#[derive(Default)]
struct WorkerState {
    obstacles_key: Option<(Vec<Wall>, MaterialLibrary)>,
    obstacles: Option<Obstacles>,
    settings: Option<PropagationSettings>,
    cache: Option<LayerCache>,
    measured_key: Option<MeasuredKey>,
    measured: Option<MeasuredMap>,
}

fn run(rx: Receiver<(u64, Inputs)>, tx: Sender<Output>, ctx: Context) {
    let mut st = WorkerState::default();
    while let Ok((mut id, mut job)) = rx.recv() {
        // Skip stale requests: only the newest one matters.
        while let Ok((newer_id, newer)) = rx.try_recv() {
            (id, job) = (newer_id, newer);
        }
        let start = Instant::now();
        let (predicted, recomputed) = predict(&mut st, &job);

        let survey = job.survey.as_ref().filter(|s| !s.measurements.is_empty());
        let measured = survey.map(|s| {
            let key = (s.measurements.clone(), job.measured.clone(), job.grid);
            if st.measured_key.as_ref() != Some(&key) {
                st.measured = Some(measured::build(s, job.grid, &job.measured));
                st.measured_key = Some(key);
            }
            st.measured.as_ref().expect("set above")
        });
        let measured_metrics = measured.map(|m| {
            let layers: Vec<_> = m.layers.iter().collect();
            Metrics::compute(
                job.grid,
                &layers,
                Some(job.band),
                &job.settings,
                job.requirements,
            )
        });

        let field = match (job.source, &measured_metrics, measured) {
            (Source::Measured, Some(mm), Some(m)) => {
                metric_field(mm, job.metric, Some(&m.valid), Source::Measured)
            }
            (Source::Difference, Some(mm), Some(m)) => {
                let meas = metric_field(mm, job.metric, Some(&m.valid), Source::Measured);
                let pred = metric_field(&predicted, job.metric, None, Source::Predicted);
                difference_field(&meas, &pred, job.diff_threshold)
            }
            // Without survey data the measured views are empty.
            (Source::Measured | Source::Difference, _, _) => Field {
                grid: job.grid,
                values: vec![f32::NAN; job.grid.len()],
                scale: if job.source == Source::Difference {
                    Scale::Difference
                } else {
                    Scale::Metric(job.metric)
                },
                source: job.source,
                coverage: None,
            },
            (Source::Predicted, _, _) => {
                metric_field(&predicted, job.metric, None, Source::Predicted)
            }
        };
        let comparison = survey.map(|s| compare(s, &predicted, &job));
        let image = colorize(&field);
        let out = Output {
            id,
            image,
            field: Arc::new(field),
            comparison,
            measured_radios: measured.map_or(0, |m| m.radios.len()),
            variogram: measured.and_then(|m| m.variogram),
            elapsed: start.elapsed(),
            layers_recomputed: recomputed,
        };
        if tx.send(out).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

fn predict(st: &mut WorkerState, job: &Inputs) -> (Metrics, usize) {
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
    (metrics, recomputed)
}

fn metric_field(m: &Metrics, kind: MetricKind, valid: Option<&[bool]>, source: Source) -> Field {
    let mut meets = 0usize;
    let mut cells = 0usize;
    let values = (0..m.grid.len())
        .map(|i| {
            if valid.is_some_and(|v| !v[i]) {
                return f32::NAN;
            }
            cells += 1;
            if m.meets_requirements(i) {
                meets += 1;
            }
            let unusable = m.serving[i] == NO_SERVER || m.rssi[i] < RSSI_CUTOFF;
            if unusable && kind != MetricKind::Coverage && kind != MetricKind::ApCount {
                f32::NAN
            } else {
                m.value(kind, i)
            }
        })
        .collect();
    Field {
        grid: m.grid,
        values,
        scale: Scale::Metric(kind),
        source,
        coverage: (cells > 0).then(|| meets as f32 / cells as f32),
    }
}

fn difference_field(meas: &Field, pred: &Field, threshold: f32) -> Field {
    let values = meas
        .values
        .iter()
        .zip(&pred.values)
        .map(|(m, p)| {
            let d = m - p;
            if d.is_nan() || d.abs() < threshold {
                f32::NAN
            } else {
                d
            }
        })
        .collect();
    Field {
        grid: meas.grid,
        values,
        scale: Scale::Difference,
        source: Source::Difference,
        coverage: None,
    }
}

/// Measured best RSSI at each survey point vs the predicted best RSSI there.
fn compare(survey: &Survey, predicted: &Metrics, job: &Inputs) -> Comparison {
    let g = predicted.grid;
    let mut points = Vec::new();
    for m in &survey.measurements {
        let Some(measured) = measured::best_rssi(m, &job.measured, Some(job.band)) else {
            continue;
        };
        let (c, r) = (
            ((m.position.x - g.origin.x) / g.cell_m).floor(),
            ((m.position.y - g.origin.y) / g.cell_m).floor(),
        );
        if c < 0.0 || r < 0.0 || c as usize >= g.cols || r as usize >= g.rows {
            continue;
        }
        let i = r as usize * g.cols + c as usize;
        if predicted.serving[i] == NO_SERVER {
            continue;
        }
        points.push((m.id, measured.0 - predicted.rssi[i]));
    }
    let n = points.len().max(1) as f32;
    let mean = points.iter().map(|(_, e)| e).sum::<f32>() / n;
    let rmse = (points.iter().map(|(_, e)| e * e).sum::<f32>() / n).sqrt();
    Comparison { points, mean, rmse }
}

pub fn stops(scale: Scale) -> ondalux_core::colormap::Stops {
    scale.stops()
}

pub fn is_discrete(scale: Scale) -> bool {
    scale.is_discrete()
}

pub fn color_at(scale: Scale, v: f32) -> Color32 {
    let [r, g, b] = scale.rgb(v);
    Color32::from_rgb(r, g, b)
}
fn colorize(f: &Field) -> ColorImage {
    let pixels = f
        .values
        .iter()
        .map(|v| {
            if v.is_nan() {
                Color32::TRANSPARENT
            } else {
                color_at(f.scale, *v)
            }
        })
        .collect();
    ColorImage::new([f.grid.cols, f.grid.rows], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSSI: Scale = Scale::Metric(MetricKind::Rssi);

    #[test]
    fn difference_hides_small_and_missing_values() {
        let grid = GridSpec::covering(3.0, 1.0, 1.0);
        let mk = |values: Vec<f32>| Field {
            grid,
            values,
            scale: RSSI,
            source: Source::Measured,
            coverage: None,
        };
        let d = difference_field(
            &mk(vec![-60.0, -70.0, f32::NAN]),
            &mk(vec![-62.0, -60.0, -50.0]),
            6.0,
        );
        assert!(d.values[0].is_nan(), "2 dB is below the threshold");
        assert_eq!(d.values[1], -10.0);
        assert!(d.values[2].is_nan());
    }
}
