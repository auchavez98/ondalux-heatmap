//! End to end: build a project with a plan image, save it as .lux, load it
//! back, export the AP list, render a heat map PNG and a PDF report, and
//! render the report's first page to check it is a valid, drawable PDF.

use image::{Rgba, RgbaImage};
use ondalux_core::ap::{AccessPoint, ApId, Radio};
use ondalux_core::catalog::Catalog;
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::{Segment, Vec2};
use ondalux_core::material::{MaterialId, Wall};
use ondalux_core::project::{Calibration, Project};
use ondalux_core::units::{Db, Dbm};
use ondalux_io::ap_csv::write_ap_csv;
use ondalux_io::{decode_plan, PlanImage, ProjectFile};
use ondalux_report::map::{render_map, MapOptions};
use ondalux_report::{build_pdf, Lang, ReportOptions};

fn project_file() -> ProjectFile {
    let (w, h) = (1000u32, 500u32); // 25 × 12.5 m at 40 px/m
    let mut plan = RgbaImage::from_pixel(w, h, Rgba([250, 250, 250, 255]));
    for x in 0..w {
        plan.put_pixel(x, 0, Rgba([0, 0, 0, 255]));
        plan.put_pixel(x, h - 1, Rgba([0, 0, 0, 255]));
    }
    let mut png = std::io::Cursor::new(Vec::new());
    plan.write_to(&mut png, image::ImageFormat::Png).unwrap();

    let mut p = Project::new("Integración", w, h);
    p.set_calibration(Calibration::new(
        Vec2::new(0.0, 0.0),
        Vec2::new(400.0, 0.0),
        10.0,
    ));
    let c = [
        Vec2::new(0.0, 0.0),
        Vec2::new(25.0, 0.0),
        Vec2::new(25.0, 12.5),
        Vec2::new(0.0, 12.5),
    ];
    for k in 0..4 {
        p.walls.push(Wall {
            segment: Segment::new(c[k], c[(k + 1) % 4]),
            material: MaterialId(5),
        });
    }
    p.walls.push(Wall {
        segment: Segment::new(Vec2::new(12.5, 0.0), Vec2::new(12.5, 9.0)),
        material: MaterialId(1),
    });
    // One AP from the catalog and one generic AP.
    let catalog = Catalog::builtin();
    let model = catalog
        .find("Ubiquiti", "U6 Pro")
        .expect("catalog model")
        .clone();
    let mut a = AccessPoint {
        id: ApId(1),
        name: "Recepción".into(),
        vendor: String::new(),
        model: String::new(),
        position: Vec2::new(6.0, 6.0),
        height_m: 3.0,
        orientation_deg: 0.0,
        mount: Default::default(),
        pattern: Default::default(),
        radios: Vec::new(),
    };
    model.apply_to(&mut a, 0);
    p.aps.push(a);
    p.aps.push(AccessPoint {
        id: ApId(2),
        name: "Sala".into(),
        vendor: String::new(),
        model: "Generic".into(),
        position: Vec2::new(19.0, 6.0),
        height_m: 3.0,
        orientation_deg: 0.0,
        mount: Default::default(),
        pattern: Default::default(),
        radios: vec![Radio {
            channel: Channel::new(Band::G5, 149, ChannelWidth::Mhz40).unwrap(),
            tx_power: Dbm(17.0),
            antenna_gain: Db(5.0),
            enabled: true,
        }],
    });
    ProjectFile {
        project: p,
        plan_image: Some(PlanImage {
            extension: "png".into(),
            bytes: png.into_inner(),
        }),
        plan_source: None,
    }
}

#[test]
fn project_to_report_round_trip() {
    let dir = std::env::temp_dir().join(format!("ondalux-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("integracion.lux");
    project_file().save(&path).unwrap();
    let loaded = ProjectFile::load(&path).unwrap();
    assert_eq!(loaded.project.aps[0].vendor, "Ubiquiti");
    let plan = decode_plan(loaded.plan_image.as_ref().unwrap()).unwrap();

    // AP CSV: header + 2 radios of the U6 Pro + 1 radio of the generic AP.
    let mut csv = Vec::new();
    write_ap_csv(&mut csv, &loaded.project.aps).unwrap();
    assert_eq!(String::from_utf8(csv).unwrap().lines().count(), 4);

    // Plain map export (no heat layer) keeps the plan resolution.
    let img = render_map(&loaded.project, Some(&plan), &MapOptions::default());
    assert_eq!((img.width(), img.height()), (1000, 500));
    // Top edge: plan line + concrete wall drawn on top, both dark.
    assert!(img.get_pixel(250, 0).0[..3].iter().all(|&c| c < 100));
    // Interior stays the light plan background.
    assert!(img.get_pixel(300, 300).0[..3].iter().all(|&c| c > 200));

    // PDF report.
    let pdf = build_pdf(
        &loaded.project,
        Some(&plan),
        &ReportOptions {
            lang: Lang::Es,
            image_width_px: 800,
            date: Some("2026-10-13".into()),
            ..Default::default()
        },
    );
    std::fs::write(dir.join("report.pdf"), &pdf).unwrap();
    let doc = hayro::hayro_syntax::Pdf::new(pdf).expect("valid PDF");
    assert!(doc.pages().len() >= 4);

    // Render the first page: it must not be blank.
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
    let pix = render(
        &doc.pages()[0],
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: 0.5,
            y_scale: 0.5,
            bg_color: WHITE,
        },
    );
    let page = image::load_from_memory(&pix.into_png().unwrap())
        .unwrap()
        .to_rgb8();
    let dark = page
        .pixels()
        .filter(|p| p.0.iter().all(|&c| c < 128))
        .count();
    assert!(dark > 200, "first page looks empty ({dark} dark pixels)");

    std::fs::remove_dir_all(&dir).unwrap();
}
