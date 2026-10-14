//! Heat map performance against the Phase 1 target: moving one AP on a
//! 2000×2000 px plan must update in < 100 ms.
//!
//! Scenario: 100 m × 100 m office at 20 px/m (2000×2000 px), sampled every
//! 4 px (0.2 m → 500×500 = 250 000 cells), an office grid of ~440 walls and
//! 20 dual-band APs (40 radio layers).

use criterion::{criterion_group, criterion_main, Criterion};
use ondalux_core::ap::{AccessPoint, ApId, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::{Segment, Vec2};
use ondalux_core::grid::{compute_layer, GridSpec, LayerCache};
use ondalux_core::material::{MaterialId, MaterialLibrary, Wall};
use ondalux_core::metrics::{Metrics, Requirements};
use ondalux_core::predict::Obstacles;
use ondalux_core::propagation::PropagationSettings;
use ondalux_core::units::{Db, Dbm};

const SIZE_M: f32 = 100.0;

fn office_walls() -> Vec<Wall> {
    let mut walls = Vec::new();
    // Rooms of 5 m × 10 m along corridors, alternating drywall and concrete.
    for i in 0..=20 {
        let x = i as f32 * 5.0;
        for j in 0..10 {
            let y0 = j as f32 * 10.0 + 1.5;
            walls.push(Wall {
                segment: Segment::new(Vec2::new(x, y0), Vec2::new(x, y0 + 7.0)),
                material: MaterialId(if i % 4 == 0 { 5 } else { 1 }),
            });
        }
    }
    for j in 0..=10 {
        let y = j as f32 * 10.0;
        for i in 0..20 {
            let x0 = i as f32 * 5.0;
            walls.push(Wall {
                segment: Segment::new(Vec2::new(x0, y), Vec2::new(x0 + 4.0, y)),
                material: MaterialId(if j % 2 == 0 { 3 } else { 1 }),
            });
        }
    }
    walls
}

fn aps() -> Vec<AccessPoint> {
    let ch24 = [1, 6, 11];
    let ch5 = [36, 52, 100, 149];
    (0..20)
        .map(|i| {
            let x = 10.0 + (i % 5) as f32 * 20.0;
            let y = 12.5 + (i / 5) as f32 * 25.0;
            AccessPoint {
                id: ApId(i),
                name: format!("AP{i}"),
                vendor: String::new(),
                model: String::new(),
                position: Vec2::new(x, y),
                height_m: 3.0,
                orientation_deg: 0.0,
                mount: Default::default(),
                pattern: Default::default(),
                radios: vec![
                    Radio {
                        channel: Channel::new(
                            Band::G2_4,
                            ch24[i as usize % 3],
                            ChannelWidth::Mhz20,
                        )
                        .unwrap(),
                        tx_power: Dbm(14.0),
                        antenna_gain: Db(3.0),
                        enabled: true,
                    },
                    Radio {
                        channel: Channel::new(Band::G5, ch5[i as usize % 4], ChannelWidth::Mhz40)
                            .unwrap(),
                        tx_power: Dbm(17.0),
                        antenna_gain: Db(5.0),
                        enabled: true,
                    },
                ],
            }
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    let walls = office_walls();
    let obstacles = Obstacles::new(&walls, &MaterialLibrary::default());
    let settings = PropagationSettings::default();
    let grid = GridSpec::covering(SIZE_M, SIZE_M, 0.2);
    let mut aps = aps();

    c.bench_function("layer_250k_cells", |b| {
        b.iter(|| compute_layer(&grid, &aps[7], 1, &obstacles, &settings))
    });

    let mut cache = LayerCache::new(grid);
    cache.sync(&aps, &obstacles, &settings);
    c.bench_function("metrics_250k_cells_20_layers_5ghz", |b| {
        b.iter(|| {
            Metrics::compute(
                grid,
                &cache.layers(),
                Some(Band::G5),
                &settings,
                Requirements::default(),
            )
        })
    });

    // The real interactive path: one AP moves, its two radio layers are
    // recomputed and the 5 GHz metrics are re-aggregated.
    let mut step = 0u32;
    c.bench_function("move_one_ap_full_update", |b| {
        b.iter(|| {
            step += 1;
            aps[7].position.x = 40.0 + (step % 10) as f32;
            cache.sync(&aps, &obstacles, &settings);
            Metrics::compute(
                grid,
                &cache.layers(),
                Some(Band::G5),
                &settings,
                Requirements::default(),
            )
        })
    });
}

fn bench_measured(c: &mut Criterion) {
    use ondalux_core::interpolate::{InterpolationMethod, InterpolationSettings};
    use ondalux_core::measured::{self, MeasuredSettings};
    use ondalux_core::survey::{BssReading, Measurement, Security, Survey};

    // 200 survey points over 100 × 100 m, 40 radios heard everywhere.
    let mut survey = Survey::new("bench", "synthetic");
    for i in 0..200u32 {
        let p = Vec2::new((i % 20) as f32 * 5.0 + 2.5, (i / 20) as f32 * 10.0 + 5.0);
        let readings = (0..40u32)
            .map(|r| BssReading {
                bssid: format!("00:00:00:00:{:02x}:00", r),
                ssid: "Corp".into(),
                rssi: Dbm(-40.0 - ((i * 7 + r * 13) % 50) as f32),
                frequency_mhz: if r % 2 == 0 { 5180 } else { 2412 },
                security: Security::Wpa2Personal,
            })
            .collect();
        survey.measurements.push(Measurement {
            id: i + 1,
            position: p,
            timestamp_ms: 0,
            readings,
        });
    }
    let grid = GridSpec::covering(SIZE_M, SIZE_M, 0.2);
    for (name, method) in [
        (
            "measured_idw_250k_cells_200_points_40_radios",
            InterpolationMethod::Idw,
        ),
        (
            "measured_kriging_250k_cells_200_points_40_radios",
            InterpolationMethod::Kriging,
        ),
    ] {
        let settings = MeasuredSettings {
            interpolation: InterpolationSettings {
                method,
                max_distance_m: 15.0,
                ..Default::default()
            },
            ..Default::default()
        };
        c.bench_function(name, |b| {
            b.iter(|| measured::build(&survey, grid, &settings))
        });
    }
}

criterion_group!(benches, bench, bench_measured);
criterion_main!(benches);
