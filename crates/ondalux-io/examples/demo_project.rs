//! Generates a demo project: a synthetic 40 m × 25 m office floor plan with
//! matching walls and four dual-band APs.
//!
//! Usage: `cargo run -p ondalux-io --example demo_project -- [out.lux]`

use image::{Rgba, RgbaImage};
use ondalux_core::ap::{AccessPoint, ApId, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::{Segment, Vec2};
use ondalux_core::material::{MaterialId, Wall};
use ondalux_core::project::{Calibration, Project};
use ondalux_core::units::{Db, Dbm};
use ondalux_io::{PlanImage, ProjectFile};
use std::path::PathBuf;

const PX_PER_M: f32 = 40.0;
const W_M: f32 = 40.0;
const H_M: f32 = 25.0;

const DRYWALL: u32 = 1;
const GLASS: u32 = 3;
const CONCRETE: u32 = 5;
const METAL: u32 = 6;

fn main() {
    let out = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("demo.lux"), PathBuf::from);

    // (x0, y0, x1, y1, material) in meters.
    let walls: &[(f32, f32, f32, f32, u32)] = &[
        // Outer shell (concrete).
        (0.5, 0.5, 39.5, 0.5, CONCRETE),
        (39.5, 0.5, 39.5, 24.5, CONCRETE),
        (39.5, 24.5, 0.5, 24.5, CONCRETE),
        (0.5, 24.5, 0.5, 0.5, CONCRETE),
        // Corridor at y = 10..13.
        (0.5, 10.0, 17.0, 10.0, DRYWALL),
        (19.0, 10.0, 39.5, 10.0, DRYWALL),
        (0.5, 13.0, 25.0, 13.0, GLASS),
        (27.0, 13.0, 39.5, 13.0, GLASS),
        // North offices.
        (8.0, 0.5, 8.0, 10.0, DRYWALL),
        (16.0, 0.5, 16.0, 10.0, DRYWALL),
        (24.0, 0.5, 24.0, 10.0, DRYWALL),
        (32.0, 0.5, 32.0, 10.0, DRYWALL),
        // South: meeting room, core with elevator, open space.
        (12.0, 13.0, 12.0, 24.5, GLASS),
        (20.0, 13.0, 20.0, 24.5, CONCRETE),
        (20.0, 18.0, 26.0, 18.0, CONCRETE),
        (26.0, 13.0, 26.0, 18.0, CONCRETE),
        (22.0, 13.5, 22.0, 17.5, METAL),
    ];

    let w_px = (W_M * PX_PER_M) as u32;
    let h_px = (H_M * PX_PER_M) as u32;
    let mut img = RgbaImage::from_pixel(w_px, h_px, Rgba([250, 250, 247, 255]));
    for &(x0, y0, x1, y1, mat) in walls {
        let thickness = if mat == CONCRETE { 6 } else { 3 };
        draw_line(
            &mut img,
            x0 * PX_PER_M,
            y0 * PX_PER_M,
            x1 * PX_PER_M,
            y1 * PX_PER_M,
            thickness,
        );
    }
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png)
        .expect("encode PNG");

    let mut project = Project::new("Demo office", w_px, h_px);
    project.set_calibration(Calibration::new(
        Vec2::new(0.0, 0.0),
        Vec2::new(10.0 * PX_PER_M, 0.0),
        10.0,
    ));
    project.walls = walls
        .iter()
        .map(|&(x0, y0, x1, y1, m)| Wall {
            segment: Segment::new(Vec2::new(x0, y0), Vec2::new(x1, y1)),
            material: MaterialId(m),
            thickness_cm: None,
        })
        .collect();

    let positions = [(8.0, 5.0), (28.0, 5.0), (6.0, 19.0), (32.0, 19.0)];
    let ch24 = [1, 6, 11, 1];
    let ch5 = [36, 52, 100, 149];
    project.aps = positions
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| AccessPoint {
            id: ApId(i as u32 + 1),
            name: format!("AP-{}", i + 1),
            vendor: String::new(),
            model: "Generic dual-band".into(),
            position: Vec2::new(x, y),
            height_m: 3.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![
                Radio {
                    channel: Channel::new(Band::G2_4, ch24[i], ChannelWidth::Mhz20).unwrap(),
                    tx_power: Dbm(14.0),
                    antenna_gain: Db(3.0),
                    enabled: true,
                },
                Radio {
                    channel: Channel::new(Band::G5, ch5[i], ChannelWidth::Mhz40).unwrap(),
                    tx_power: Dbm(17.0),
                    antenna_gain: Db(5.0),
                    enabled: true,
                },
            ],
        })
        .collect();

    let file = ProjectFile::single(
        project,
        Some(PlanImage {
            extension: "png".into(),
            bytes: png.into_inner(),
        }),
    );
    file.save(&out).expect("save project");
    println!("wrote {}", out.display());
}

fn draw_line(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, thickness: i32) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()).ceil() as i32).max(1);
    let half = thickness / 2;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let (cx, cy) = ((x0 + (x1 - x0) * t) as i32, (y0 + (y1 - y0) * t) as i32);
        for dy in -half..=half {
            for dx in -half..=half {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, Rgba([40, 40, 45, 255]));
                }
            }
        }
    }
}
