//! Generates the PDF report and heat map PNGs for a project file.
//!
//! Usage: `cargo run -p ondalux-report --example report -- project.lux out_dir [es|en] [--pages]`
//! `--pages` also rasterizes every PDF page to PNG (for previews).

use ondalux_io::{decode_plan, ProjectFile};
use ondalux_report::report::{bands_in_use, compute_bands};
use ondalux_report::{build_pdf, Lang, ReportOptions};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: report <project.lux> <out_dir> [es|en] [--pages]");
        std::process::exit(2);
    }
    let file = ProjectFile::load(&PathBuf::from(&args[1])).expect("open project");
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).expect("create output dir");
    let lang = if args.iter().any(|a| a == "en") {
        Lang::En
    } else {
        Lang::Es
    };
    let plan = file
        .plan_image(file.project.floor_index)
        .map(|p| decode_plan(p).expect("decode plan"));
    let project = &file.project;

    let start = std::time::Instant::now();
    for r in compute_bands(project, plan.as_ref(), &bands_in_use(project), lang, 1400) {
        let tag = r.band.label().replace([' ', '.'], "");
        for (kind, img) in &r.maps {
            let name = format!("{kind:?}").to_lowercase();
            img.save(out.join(format!("{name}-{tag}.png")))
                .expect("write PNG");
        }
        println!(
            "{}: coverage {:.1} %, {} gaps, {} findings",
            r.band,
            r.analysis.coverage_ratio * 100.0,
            r.analysis.gaps.len(),
            r.analysis.findings.len()
        );
    }
    let pdf = build_pdf(
        project,
        plan.as_ref(),
        &ReportOptions {
            lang,
            survey: (!project.surveys.is_empty()).then_some(0),
            ..Default::default()
        },
    );
    let pdf_path = out.join("report.pdf");
    std::fs::write(&pdf_path, &pdf).expect("write PDF");
    println!(
        "wrote {} ({} KB) in {:.1} s",
        pdf_path.display(),
        pdf.len() / 1024,
        start.elapsed().as_secs_f32()
    );

    if args.iter().any(|a| a == "--pages") {
        use hayro::hayro_interpret::InterpreterSettings;
        use hayro::vello_cpu::color::palette::css::WHITE;
        use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
        let doc = hayro::hayro_syntax::Pdf::new(pdf).expect("parse PDF");
        let cache = RenderCache::new();
        for (i, page) in doc.pages().iter().enumerate() {
            let pix = render(
                page,
                &cache,
                &InterpreterSettings::default(),
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: 1.2,
                    y_scale: 1.2,
                    bg_color: WHITE,
                },
            );
            std::fs::write(
                out.join(format!("page-{}.png", i + 1)),
                pix.into_png().unwrap(),
            )
            .expect("write page PNG");
        }
        println!("rendered {} pages", doc.pages().len());
    }
}
