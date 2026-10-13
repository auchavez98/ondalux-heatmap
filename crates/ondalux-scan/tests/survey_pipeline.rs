//! End-to-end survey pipeline without hardware:
//! simulated scans → CSV round trip → interpolation → comparison with the
//! prediction that generated them.

use ondalux_core::ap::{AccessPoint, ApId, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::{Segment, Vec2};
use ondalux_core::grid::{GridSpec, LayerCache};
use ondalux_core::interpolate::{InterpolationMethod, InterpolationSettings};
use ondalux_core::material::{MaterialId, MaterialLibrary, Wall};
use ondalux_core::measured::{self, MeasuredSettings};
use ondalux_core::metrics::{Metrics, Requirements, NO_SERVER};
use ondalux_core::predict::Obstacles;
use ondalux_core::project::{Calibration, FloorPlan};
use ondalux_core::propagation::PropagationSettings;
use ondalux_core::survey::{Measurement, Survey};
use ondalux_core::units::{Db, Dbm};
use ondalux_io::survey_csv::{read_survey_csv, write_survey_csv};
use ondalux_scan::{SimulatedScanner, WifiScanner};

const W: f32 = 30.0;
const H: f32 = 20.0;

fn aps() -> Vec<AccessPoint> {
    [(7.0, 10.0, 36), (23.0, 10.0, 149)]
        .into_iter()
        .enumerate()
        .map(|(i, (x, y, ch))| AccessPoint {
            id: ApId(i as u32 + 1),
            name: format!("AP{}", i + 1),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(x, y),
            height_m: 3.0,
            orientation_deg: 0.0,
            radios: vec![Radio {
                channel: Channel::new(Band::G5, ch, ChannelWidth::Mhz20).unwrap(),
                tx_power: Dbm(17.0),
                antenna_gain: Db(4.0),
                enabled: true,
            }],
        })
        .collect()
}

fn walls() -> Vec<Wall> {
    vec![Wall {
        segment: Segment::new(Vec2::new(15.0, 0.0), Vec2::new(15.0, 14.0)),
        material: MaterialId(5),
    }]
}

fn survey(fading: f32) -> Survey {
    let materials = MaterialLibrary::default();
    let mut scanner = SimulatedScanner::new(
        aps(),
        &walls(),
        &materials,
        PropagationSettings::default(),
        7,
    );
    scanner.fading_db = fading;
    let mut s = Survey::new("walk", scanner.name());
    let mut id = 1;
    // Walk the floor on a 2.5 m grid.
    for row in 0..8 {
        for col in 0..12 {
            let p = Vec2::new(1.25 + col as f32 * 2.5, 1.25 + row as f32 * 2.5);
            s.measurements.push(Measurement {
                id,
                position: p,
                timestamp_ms: 0,
                readings: scanner.scan(p).unwrap(),
            });
            id += 1;
        }
    }
    s
}

fn predicted(grid: GridSpec) -> Metrics {
    let settings = PropagationSettings::default();
    let obstacles = Obstacles::new(&walls(), &MaterialLibrary::default());
    let mut cache = LayerCache::new(grid);
    cache.sync(&aps(), &obstacles, &settings);
    Metrics::compute(
        grid,
        &cache.layers(),
        Some(Band::G5),
        &settings,
        Requirements::default(),
    )
}

/// Mean and RMS of (interpolated − predicted) best RSSI over cells with data.
fn map_error(method: InterpolationMethod, fading: f32) -> (f32, f32) {
    let grid = GridSpec::covering(W, H, 0.5);
    let settings = MeasuredSettings {
        interpolation: InterpolationSettings {
            method,
            ..Default::default()
        },
        ..Default::default()
    };
    let map = measured::build(&survey(fading), grid, &settings);
    assert_eq!(map.radios.len(), 2, "one radio per simulated AP");
    let layers: Vec<_> = map.layers.iter().collect();
    let meas = Metrics::compute(
        grid,
        &layers,
        Some(Band::G5),
        &PropagationSettings::default(),
        Requirements::default(),
    );
    let pred = predicted(grid);
    let errs: Vec<f32> = (0..grid.len())
        .filter(|&i| map.valid[i] && pred.serving[i] != NO_SERVER)
        .map(|i| meas.rssi[i] - pred.rssi[i])
        .collect();
    assert!(errs.len() > grid.len() / 2);
    let n = errs.len() as f32;
    let mean = errs.iter().sum::<f32>() / n;
    let rms = (errs.iter().map(|e| e * e).sum::<f32>() / n).sqrt();
    eprintln!("{method:?}, fading {fading} dB: mean {mean:+.2} dB, rms {rms:.2} dB");
    (mean, rms)
}

#[test]
fn interpolated_survey_reproduces_the_prediction_without_fading() {
    for method in [InterpolationMethod::Idw, InterpolationMethod::Kriging] {
        let (mean, rms) = map_error(method, 0.0);
        // Only interpolation error remains (sharp wall shadows, near-AP peaks).
        assert!(mean.abs() < 2.0, "{method:?}: mean {mean}");
        assert!(rms < 4.0, "{method:?}: rms {rms}");
    }
}

#[test]
fn interpolation_smooths_shadow_fading() {
    for method in [InterpolationMethod::Idw, InterpolationMethod::Kriging] {
        let (mean, rms) = map_error(method, 4.0);
        // 4 dB fading at the points; the interpolated map should not be worse.
        assert!(mean.abs() < 2.0, "{method:?}: mean {mean}");
        assert!(rms < 5.0, "{method:?}: rms {rms}");
    }
}

#[test]
fn csv_round_trip_preserves_the_survey() {
    let original = survey(4.0);
    let mut buf = Vec::new();
    write_survey_csv(&mut buf, &original).unwrap();
    let plan = FloorPlan {
        width_px: 600,
        height_px: 400,
        calibration: Calibration::new(Vec2::new(0.0, 0.0), Vec2::new(20.0, 0.0), 1.0),
    };
    let back = read_survey_csv(buf.as_slice(), "walk.csv", &plan).unwrap();
    assert!(back.warnings.is_empty());
    assert_eq!(back.survey.measurements.len(), original.measurements.len());
    for (a, b) in back.survey.measurements.iter().zip(&original.measurements) {
        assert_eq!(a.readings, b.readings);
        assert!(a.position.distance(b.position) < 1e-3);
    }
}
