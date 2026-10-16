//! Report generation speed: heat map image rendering and a full PDF for a
//! 40 × 25 m office with 4 dual-band APs.

use criterion::{criterion_group, criterion_main, Criterion};
use ondalux_core::ap::{AccessPoint, ApId, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::colormap::ColorScale;
use ondalux_core::geometry::{Segment, Vec2};
use ondalux_core::grid::{GridSpec, LayerCache};
use ondalux_core::material::{MaterialId, Wall};
use ondalux_core::metrics::{MetricKind, Metrics};
use ondalux_core::predict::Obstacles;
use ondalux_core::project::Project;
use ondalux_core::units::{Db, Dbm};
use ondalux_report::map::{render_map, HeatLayer, MapOptions};
use ondalux_report::{build_pdf, ReportOptions};

fn project() -> Project {
    let mut p = Project::new("bench", 1600, 1000); // 80 × 50 m at 20 px/m
    for i in 0..=8 {
        let x = i as f32 * 10.0;
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(x, 0.0), Vec2::new(x, 50.0)),
            material: MaterialId(if i % 4 == 0 { 5 } else { 1 }),
            thickness_cm: None,
        });
    }
    for (i, (x, y)) in [
        (10.0, 12.0),
        (50.0, 12.0),
        (10.0, 38.0),
        (50.0, 38.0),
        (30.0, 25.0),
        (70.0, 25.0),
    ]
    .into_iter()
    .enumerate()
    {
        p.aps.push(AccessPoint {
            id: ApId(i as u32 + 1),
            name: format!("AP-{}", i + 1),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(x, y),
            height_m: 3.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![
                Radio {
                    channel: Channel::new(Band::G2_4, [1, 6, 11][i % 3], ChannelWidth::Mhz20)
                        .unwrap(),
                    tx_power: Dbm(14.0),
                    antenna_gain: Db(3.0),
                    enabled: true,
                },
                Radio {
                    channel: Channel::new(Band::G5, [36, 52, 100, 149][i % 4], ChannelWidth::Mhz40)
                        .unwrap(),
                    tx_power: Dbm(17.0),
                    antenna_gain: Db(5.0),
                    enabled: true,
                },
            ],
        });
    }
    p
}

fn bench(c: &mut Criterion) {
    let p = project();
    let grid = GridSpec::covering(80.0, 50.0, 0.25);
    let obstacles = Obstacles::new(&p.walls, &p.materials);
    let mut cache = LayerCache::new(grid);
    cache.sync(&p.aps, &obstacles, &p.settings);
    let m = Metrics::compute(
        grid,
        &cache.layers(),
        Some(Band::G5),
        &p.settings,
        p.requirements,
    );

    c.bench_function("render_map_1600px", |b| {
        b.iter(|| {
            render_map(
                &p,
                None,
                &MapOptions {
                    heat: Some(HeatLayer {
                        grid,
                        values: &m.rssi,
                        scale: ColorScale::Metric(MetricKind::Rssi),
                        opacity: 0.6,
                    }),
                    legend_title: Some("Signal (dBm)".into()),
                    band: Some(Band::G5),
                    ..Default::default()
                },
            )
        })
    });

    let mut group = c.benchmark_group("pdf");
    group.sample_size(10);
    group.bench_function("full_report_2_bands", |b| {
        b.iter(|| {
            build_pdf(
                &p,
                None,
                &ReportOptions {
                    date: Some("2026-10-13".into()),
                    ..Default::default()
                },
            )
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
