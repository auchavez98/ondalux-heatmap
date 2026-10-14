//! File → Export: heat map PNG, PDF report (in the background), AP and
//! survey CSV.

use crate::app::OndaluxApp;
use crate::heatmap::Source;
use crate::i18n::t;
use crate::tr;
use eframe::egui;
use ondalux_core::channel::Band;
use ondalux_core::colormap::ColorScale;
use ondalux_io::{decode_plan, DecodedPlan};
use ondalux_report::map::{render_map, HeatLayer, MapOptions};
use ondalux_report::report::bands_in_use;
use ondalux_report::{build_pdf, Lang, ReportOptions};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

pub struct ExportState {
    pub report_open: bool,
    lang: Lang,
    author: String,
    bands: Vec<(Band, bool)>,
    include_survey: bool,
    job: Option<Receiver<(PathBuf, Result<usize, String>)>>,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            report_open: false,
            lang: Lang::Es,
            author: String::new(),
            bands: Vec::new(),
            include_survey: true,
            job: None,
        }
    }
}

impl ExportState {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

fn decoded_plan(app: &OndaluxApp) -> Option<DecodedPlan> {
    let plan = app.doc.as_ref()?.plan_image.as_ref()?;
    decode_plan(plan).ok()
}

fn safe_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if n.is_empty() {
        "project".into()
    } else {
        n
    }
}

/// Saves the heat map currently on screen (metric, band and source).
pub fn export_png(app: &mut OndaluxApp) {
    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    let Some(field) = app.heat.field.clone() else {
        app.error(t(
            "El mapa de calor aún se está calculando; inténtalo en un momento.",
            "The heat map is still being computed; try again in a moment.",
        ));
        return;
    };
    let metric = app.heat.metric;
    let band = app.heat.band;
    let default_name = format!(
        "{}-{}-{}.png",
        safe_name(&doc.project.name),
        safe_name(metric.label()),
        safe_name(band.label())
    );
    let Some(mut path) = rfd::FileDialog::new()
        .set_title(t(
            "Exportar imagen del mapa de calor",
            "Export heat map image",
        ))
        .set_file_name(default_name)
        .add_filter(t("Imagen PNG", "PNG image"), &["png"])
        .save_file()
    else {
        return;
    };
    if path.extension().is_none() {
        path.set_extension("png");
    }
    let title = match (field.source, field.scale) {
        (Source::Difference, _) | (_, ColorScale::Difference) => {
            format!(
                "{} · Measured − predicted ({})",
                band.label(),
                metric.label()
            )
        }
        (Source::Measured, _) => format!("{} · Measured {}", band.label(), metric.label()),
        (Source::Predicted, _) => {
            format!("{} · {} ({})", band.label(), metric.label(), metric.unit())
        }
    };
    let plan = decoded_plan(app);
    let doc = app.doc.as_ref().expect("checked above");
    let img = render_map(
        &doc.project,
        plan.as_ref(),
        &MapOptions {
            max_width_px: 4096,
            heat: Some(HeatLayer {
                grid: field.grid,
                values: &field.values,
                scale: field.scale,
                opacity: app.heat.opacity,
            }),
            legend_title: Some(title),
            band: Some(band),
            ..Default::default()
        },
    );
    match img.save(&path) {
        Ok(()) => app.info(tr!(
            "Guardado {} ({}×{} px).",
            "Saved {} ({}×{} px).",
            path.display(),
            img.width(),
            img.height()
        )),
        Err(e) => app.error(tr!(
            "No se pudo guardar {}: {e}",
            "Could not save {}: {e}",
            path.display()
        )),
    }
}

pub fn export_ap_csv(app: &mut OndaluxApp) {
    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title(t("Exportar puntos de acceso", "Export access points"))
        .set_file_name(format!("{}-aps.csv", safe_name(&doc.project.name)))
        .add_filter("CSV", &["csv"])
        .save_file()
    else {
        return;
    };
    let result = std::fs::File::create(&path)
        .map_err(ondalux_io::IoError::from)
        .and_then(|f| {
            ondalux_io::ap_csv::write_ap_csv(std::io::BufWriter::new(f), &doc.project.aps)
        });
    match result {
        Ok(()) => app.info(tr!(
            "{} puntos de acceso exportados a {}.",
            "Exported {} access points to {}.",
            doc.project.aps.len(),
            path.display()
        )),
        Err(e) => app.error(tr!("No se pudo exportar: {e}", "Could not export: {e}")),
    }
}

/// Collects a finished background report.
pub fn poll(app: &mut OndaluxApp) {
    let Some(rx) = &app.export.job else {
        return;
    };
    let Ok((path, result)) = rx.try_recv() else {
        return;
    };
    app.export.job = None;
    match result {
        Ok(kb) => app.info(tr!(
            "Reporte guardado: {} ({kb} KB).",
            "Report saved: {} ({kb} KB).",
            path.display()
        )),
        Err(e) => app.error(tr!(
            "No se pudo guardar el reporte: {e}",
            "Could not save the report: {e}"
        )),
    }
}

pub fn report_window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.export.report_open {
        return;
    }
    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    // Keep the band list in sync with the project.
    let in_use = bands_in_use(&doc.project);
    let st = &mut app.export;
    st.bands.retain(|(b, _)| in_use.contains(b));
    for b in &in_use {
        if !st.bands.iter().any(|(x, _)| x == b) {
            st.bands.push((*b, true));
        }
    }
    st.bands.sort_by_key(|(b, _)| *b);
    let has_survey = !doc.project.surveys.is_empty();

    let mut open = true;
    let mut generate = false;
    egui::Window::new(t("Reporte PDF", "PDF report"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("report-opts")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label(t("Idioma", "Language"));
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut st.lang, Lang::Es, "Español");
                        ui.radio_value(&mut st.lang, Lang::En, "English");
                    });
                    ui.end_row();
                    ui.label(t("Autor", "Author"));
                    ui.text_edit_singleline(&mut st.author);
                    ui.end_row();
                    ui.label(t("Bandas", "Bands"));
                    ui.horizontal(|ui| {
                        if st.bands.is_empty() {
                            ui.weak(t("ningún AP tiene una radio activa", "no AP has an enabled radio"));
                        }
                        for (b, on) in &mut st.bands {
                            ui.checkbox(on, b.label());
                        }
                    });
                    ui.end_row();
                    ui.label(t("Mediciones", "Survey"));
                    ui.add_enabled(
                        has_survey,
                        egui::Checkbox::new(
                            &mut st.include_survey,
                            t("Comparar con la medición activa", "Compare with the active survey"),
                        ),
                    );
                    ui.end_row();
                });
            ui.weak(t(
                "Incluye un resumen, mapas de señal y cobertura por banda, zonas con mala cobertura, \
                 recomendaciones y la tabla de APs.",
                "Includes a summary, signal and coverage maps per band, areas with poor coverage, \
                 recommendations and the AP table.",
            ));
            ui.horizontal(|ui| {
                if st.busy() {
                    ui.spinner();
                    ui.label(t("Generando…", "Generating…"));
                } else if ui.button(t("Guardar PDF…", "Save PDF…")).clicked() {
                    generate = true;
                }
            });
        });
    app.export.report_open = open;
    if !generate {
        return;
    }

    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    let Some(mut path) = rfd::FileDialog::new()
        .set_title(t("Guardar reporte", "Save report"))
        .set_file_name(format!("{}-report.pdf", safe_name(&doc.project.name)))
        .add_filter("PDF", &["pdf"])
        .save_file()
    else {
        return;
    };
    if path.extension().is_none() {
        path.set_extension("pdf");
    }
    let opts = ReportOptions {
        lang: app.export.lang,
        bands: app
            .export
            .bands
            .iter()
            .filter(|(_, on)| *on)
            .map(|(b, _)| *b)
            .collect(),
        survey: (app.export.include_survey && has_survey).then_some(app.survey.active),
        author: app.export.author.clone(),
        ..Default::default()
    };
    if opts.bands.is_empty() && !app.export.bands.is_empty() {
        app.error(t(
            "Selecciona al menos una banda.",
            "Select at least one band.",
        ));
        return;
    }
    let project = doc.project.clone();
    let plan_bytes = doc.plan_image.clone();
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("report".into())
        .spawn(move || {
            let plan = plan_bytes.as_ref().and_then(|p| decode_plan(p).ok());
            let pdf = build_pdf(&project, plan.as_ref(), &opts);
            let result = std::fs::write(&path, &pdf)
                .map(|()| pdf.len() / 1024)
                .map_err(|e| e.to_string());
            let _ = tx.send((path, result));
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => app.export.job = Some(rx),
        Err(e) => app.error(tr!(
            "No se pudo iniciar el reporte: {e}",
            "Could not start the report: {e}"
        )),
    }
}
