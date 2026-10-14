//! Runs wall detection on a floor plan and writes an overlay PNG, to check
//! the detector on real plans:
//!
//! `cargo run --release -p ondalux-io --example detect_walls -- plan.pdf out.png [sensitivity]`
//!
//! With `--synthetic out_dir` it writes a realistic test plan (PDF) with
//! thick exterior walls, partitions, doors, text, dimensions, furniture,
//! dashed lines and a double-line wall, then detects on it.

use ondalux_io::walls::{plan_mask, Tuning};
use ondalux_io::{import_plan, ImportedPlan};
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--synthetic") {
        let dir = PathBuf::from(args.get(1).expect("output dir"));
        std::fs::create_dir_all(&dir).expect("create dir");
        let pdf = dir.join("synthetic-plan.pdf");
        std::fs::write(&pdf, synthetic_pdf()).expect("write pdf");
        run(&pdf, &dir.join("synthetic-detected.png"), 0.5);
        // Same plan as a plain image (as if a PNG export was imported).
        let ImportedPlan { image, .. } = import_plan(&pdf).expect("import");
        let png = dir.join("synthetic-plan.png");
        std::fs::write(&png, &image.bytes).expect("write png");
        run(&png, &dir.join("synthetic-detected-raster.png"), 0.5);
        return;
    }
    let input = PathBuf::from(args.first().expect("plan path"));
    let output = PathBuf::from(args.get(1).expect("output png"));
    let sensitivity = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.5);
    run(&input, &output, sensitivity);
}

fn run(input: &Path, output: &Path, sensitivity: f32) {
    let t0 = std::time::Instant::now();
    let ImportedPlan {
        decoded,
        source_pdf,
        ..
    } = import_plan(input).expect("import plan");
    let t1 = std::time::Instant::now();
    let pm = plan_mask(&decoded, source_pdf.as_deref()).expect("mask");
    let walls = pm.detect(Tuning { sensitivity });
    let t2 = std::time::Instant::now();
    println!(
        "{}: {}×{} px, source {:?}, params {:?}",
        input.display(),
        decoded.width,
        decoded.height,
        pm.source,
        pm.params(Tuning { sensitivity })
    );
    println!(
        "{} walls in {:.0} ms (import {:.0} ms)",
        walls.len(),
        (t2 - t1).as_secs_f64() * 1000.0,
        (t1 - t0).as_secs_f64() * 1000.0
    );
    // Overlay: plan faded, walls in red.
    let mut img =
        image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.rgba).expect("image");
    for p in img.pixels_mut() {
        for c in 0..3 {
            p[c] = 255 - (255 - p[c]) / 3;
        }
    }
    for w in &walls {
        let half = (w.thickness_px / 2.0).max(1.0);
        let (x0, x1) = (w.a.x.min(w.b.x) - half, w.a.x.max(w.b.x) + half);
        let (y0, y1) = (w.a.y.min(w.b.y) - half, w.a.y.max(w.b.y) + half);
        for y in y0.max(0.0) as u32..(y1 as u32).min(img.height()) {
            for x in x0.max(0.0) as u32..(x1 as u32).min(img.width()) {
                let p = img.get_pixel_mut(x, y);
                *p = image::Rgba([230, 40, 40, 255]);
            }
        }
    }
    img.save(output).expect("save overlay");
    println!("wrote {}", output.display());
}

/// A 60 × 40 m office at 1:200 on a 1190 × 842 pt page (A3), drawn the way
/// CAD exports usually are.
fn synthetic_pdf() -> Vec<u8> {
    let m = 1190.0 / 70.0; // pt per meter, with margins
    let x = |v: f64| 60.0 + v * m;
    let y = |v: f64| 842.0 - 60.0 - v * m;
    let mut c = String::new();
    let line = |c: &mut String, w: f64, x0: f64, y0: f64, x1: f64, y1: f64| {
        c.push_str(&format!(
            "{w:.2} w {:.1} {:.1} m {:.1} {:.1} l S\n",
            x(x0),
            y(y0),
            x(x1),
            y(y1)
        ));
    };
    // Exterior walls: filled 30 cm bands (poché).
    c.push_str("0 0 0 rg\n");
    let band = |c: &mut String, x0: f64, y0: f64, x1: f64, y1: f64| {
        c.push_str(&format!(
            "{:.1} {:.1} {:.1} {:.1} re f\n",
            x(x0),
            y(y1),
            x(x1) - x(x0),
            y(y0) - y(y1)
        ));
    };
    band(&mut c, 0.0, 0.0, 60.0, 0.3);
    band(&mut c, 0.0, 39.7, 27.0, 40.0); // entrance gap 27–29 m
    band(&mut c, 29.0, 39.7, 60.0, 40.0);
    band(&mut c, 0.0, 0.0, 0.3, 40.0);
    band(&mut c, 59.7, 0.0, 60.0, 40.0);
    // Interior partitions: 10 cm strokes, with door gaps.
    c.push_str("0 0 0 RG\n");
    let t = 0.10 * m;
    line(&mut c, t, 0.3, 15.0, 9.0, 15.0);
    line(&mut c, t, 10.0, 15.0, 30.0, 15.0);
    line(&mut c, t, 31.0, 15.0, 59.7, 15.0);
    line(&mut c, t, 15.0, 0.3, 15.0, 15.0);
    line(&mut c, t, 30.0, 0.3, 30.0, 6.0);
    line(&mut c, t, 30.0, 7.0, 30.0, 15.0);
    line(&mut c, t, 45.0, 0.3, 45.0, 15.0);
    line(&mut c, t, 40.0, 15.0, 40.0, 39.7);
    // A wall drawn as two lines 20 cm apart (corridor wall).
    line(&mut c, 0.6, 0.3, 25.0, 39.7, 25.0);
    line(&mut c, 0.6, 0.3, 25.2, 39.7, 25.2);
    // Door swings (quarter arcs) and door leaves, thin.
    for (cx, cy) in [(9.0, 15.0), (30.0, 15.0), (30.0, 6.0)] {
        let r = 1.0 * m;
        c.push_str(&format!(
            "0.3 w {:.1} {:.1} m {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} c S\n",
            x(cx) + r,
            y(cy),
            x(cx) + r,
            y(cy) - r * 0.55,
            x(cx) + r * 0.55,
            y(cy) - r,
            x(cx),
            y(cy) - r
        ));
    }
    // Furniture: desks (thin rectangles) and a meeting table.
    for i in 0..6 {
        let x0 = 2.0 + i as f64 * 2.0;
        c.push_str(&format!(
            "0.4 w {:.1} {:.1} {:.1} {:.1} re S\n",
            x(x0),
            y(4.0),
            1.6 * m,
            0.8 * m
        ));
    }
    c.push_str(&format!(
        "0.4 w {:.1} {:.1} {:.1} {:.1} re S\n",
        x(34.0),
        y(10.0),
        6.0 * m,
        2.0 * m
    ));
    // Dimension line with ticks, thin, below the plan.
    line(&mut c, 0.25, 0.0, 41.5, 60.0, 41.5);
    for v in [0.0, 15.0, 30.0, 45.0, 60.0] {
        line(&mut c, 0.25, v, 41.0, v, 42.0);
    }
    // Dashed line (overhead beam): must be ignored in vector mode.
    c.push_str("[6 4] 0 d\n");
    line(&mut c, 1.0, 0.3, 32.0, 59.7, 32.0);
    c.push_str("[] 0 d\n");
    // Room labels.
    c.push_str("BT /F1 14 Tf\n");
    for (label, lx, ly) in [
        ("OFICINA 1", 5.0, 8.0),
        ("OFICINA 2", 20.0, 8.0),
        ("SALA DE JUNTAS", 34.0, 6.0),
        ("RECEPCION", 20.0, 33.0),
        ("BODEGA", 48.0, 30.0),
    ] {
        c.push_str(&format!(
            "1 0 0 1 {:.1} {:.1} Tm ({label}) Tj\n",
            x(lx),
            y(ly)
        ));
    }
    c.push_str("ET\n");

    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1190 842] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
