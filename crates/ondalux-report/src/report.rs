//! Builds the PDF design report: summary, heat maps per band, coverage
//! gaps, recommendations, AP table and (optionally) survey comparison.

use crate::map::{render_map, GapOverlay, HeatLayer, MapOptions};
use crate::pdfdoc::{
    Doc, Kpi, Rgb, ACCENT, BAD, CONTENT_BOTTOM, INK, MARGIN, MUTED, OK, PAGE_W, WARN,
};
use image::RgbaImage;
use ondalux_core::analysis::{analyze_band, BandAnalysis, Finding, Severity, TARGET_COVERAGE};
use ondalux_core::ap::Mount;
use ondalux_core::channel::Band;
use ondalux_core::colormap::{ColorScale, RSSI_CUTOFF};
use ondalux_core::grid::{GridSpec, LayerCache};
use ondalux_core::measured::best_rssi;
use ondalux_core::metrics::{MetricKind, Metrics, NO_SERVER};
use ondalux_core::predict::Obstacles;
use ondalux_core::project::Project;
use ondalux_core::propagation::PathLossModel;
use ondalux_io::DecodedPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    fn t<'a>(self, es: &'a str, en: &'a str) -> &'a str {
        match self {
            Lang::Es => es,
            Lang::En => en,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub lang: Lang,
    /// Bands to include; empty = every band with at least one radio.
    pub bands: Vec<Band>,
    /// Survey (index into `project.surveys`) to compare with, if any.
    pub survey: Option<usize>,
    pub author: String,
    /// Width of the map images embedded in the PDF.
    pub image_width_px: u32,
    /// "YYYY-MM-DD"; defaults to today (UTC).
    pub date: Option<String>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            lang: Lang::Es,
            bands: Vec::new(),
            survey: None,
            author: String::new(),
            image_width_px: 1400,
            date: None,
        }
    }
}

/// Today's date (UTC) as YYYY-MM-DD, without external crates.
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    civil_date(secs.div_euclid(86_400))
}

/// Days since 1970-01-01 → "YYYY-MM-DD" (Howard Hinnant's algorithm).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Everything computed for one band.
pub struct BandResult {
    pub band: Band,
    pub metrics: Metrics,
    pub analysis: BandAnalysis,
    /// One map per metric, in [`MetricKind::ALL`] order.
    pub maps: Vec<(MetricKind, RgbaImage)>,
}

impl BandResult {
    pub fn map(&self, kind: MetricKind) -> Option<&RgbaImage> {
        self.maps.iter().find(|(k, _)| *k == kind).map(|(_, m)| m)
    }
}

/// Bands with at least one enabled radio.
pub fn bands_in_use(project: &Project) -> Vec<Band> {
    Band::ALL
        .into_iter()
        .filter(|b| {
            project
                .aps
                .iter()
                .any(|a| a.enabled_radios().any(|(_, r)| r.channel.band == *b))
        })
        .collect()
}

/// Grid for reports: the project resolution, coarsened if the plan is huge.
pub fn report_grid(project: &Project) -> GridSpec {
    let (w, h) = project.plan.size_m();
    let mut cell = project.grid_cell_m.max(0.05);
    while (w / cell) * (h / cell) > 1_500_000.0 {
        cell *= 1.5;
    }
    GridSpec::covering(w, h, cell)
}

/// Title and one-line explanation of a metric map.
pub fn metric_text(kind: MetricKind, project: &Project, l: Lang) -> (String, String) {
    let req = project.requirements;
    match kind {
        MetricKind::Rssi => (
            l.t("Intensidad de señal", "Signal strength").into(),
            l.t(
                "Potencia del AP más fuerte en cada punto, en dBm. Más cerca de 0 es mejor; −67 dBm es lo habitual para voz y video.",
                "Power of the strongest AP at each point, in dBm. Closer to 0 is better; −67 dBm is typical for voice and video.",
            )
            .into(),
        ),
        MetricKind::Snr => (
            l.t("Relación señal/ruido (SNR)", "Signal-to-noise ratio (SNR)").into(),
            l.t(
                "Cuánto supera la señal al ruido de fondo, en dB. 25 dB o más permite conexiones rápidas y estables.",
                "How far the signal is above the noise floor, in dB. 25 dB or more allows fast, stable links.",
            )
            .into(),
        ),
        MetricKind::Sinr => (
            l.t("Señal/interferencia (SINR)", "Signal-to-interference (SINR)").into(),
            l.t(
                "Como la SNR, pero restando también la señal de otros APs en el mismo canal o en canales que comparten espectro. Los APs en canales distintos no interfieren.",
                "Like SNR, but also subtracting other APs on the same channel or on channels sharing spectrum. APs on different channels do not interfere.",
            )
            .into(),
        ),
        MetricKind::ApCount => (
            l.t("Número de APs que se escuchan", "Number of APs heard").into(),
            format!(
                "{} {:.0} dBm. {}",
                l.t("APs recibidos con al menos", "APs received at"),
                req.min_rssi.0,
                l.t(
                    "2 o más dan respaldo ante fallas y roaming fluido.",
                    "or more. 2 or more give redundancy and smooth roaming.",
                )
            ),
        ),
        MetricKind::ChannelOverlap => (
            l.t("Solapamiento de canales", "Channel overlap").into(),
            format!(
                "{} {:.0} dBm. {}",
                l.t(
                    "Otros radios en canales solapados que se escuchan por encima de",
                    "Other radios on overlapping channels heard above",
                ),
                req.cca_threshold.0,
                l.t(
                    "Comparten el tiempo de aire; lo ideal es 0 o 1.",
                    "They share airtime; ideally 0 or 1.",
                )
            ),
        ),
        MetricKind::Coverage => (
            l.t("Cumplimiento de requisitos", "Requirements met").into(),
            format!(
                "{} {:.0} dBm {} {:.0} dB. {}",
                l.t("Verde donde la señal es ≥", "Green where signal ≥"),
                req.min_rssi.0,
                l.t("y la SNR ≥", "and SNR ≥"),
                req.min_snr.0,
                l.t(
                    "Las zonas que no cumplen están numeradas.",
                    "Areas that fail are numbered.",
                )
            ),
        ),
    }
}

fn legend_title(kind: MetricKind, band: Band, l: Lang) -> String {
    let unit = match kind {
        MetricKind::Rssi => " (dBm)",
        MetricKind::Snr | MetricKind::Sinr => " (dB)",
        _ => "",
    };
    let name = match kind {
        MetricKind::Rssi => l.t("Señal", "Signal"),
        MetricKind::Snr => "SNR",
        MetricKind::Sinr => "SINR",
        MetricKind::ApCount => l.t("APs que se escuchan", "APs heard"),
        MetricKind::ChannelOverlap => l.t("Radios solapadas", "Overlapping radios"),
        MetricKind::Coverage => l.t("Cumple requisitos", "Requirements met"),
    };
    format!("{name}{unit} · {}", band.label())
}

/// The plan with only the access points and their names.
pub fn placement_map(project: &Project, plan: Option<&DecodedPlan>, width_px: u32) -> RgbaImage {
    render_map(
        project,
        plan,
        &MapOptions {
            max_width_px: width_px,
            show_walls: false,
            label_scale: 2.8,
            ..Default::default()
        },
    )
}

pub fn compute_bands(
    project: &Project,
    plan: Option<&DecodedPlan>,
    bands: &[Band],
    lang: Lang,
    image_width_px: u32,
) -> Vec<BandResult> {
    let grid = report_grid(project);
    let obstacles = Obstacles::new(&project.walls, &project.materials);
    let mut cache = LayerCache::new(grid);
    cache.sync(&project.aps, &obstacles, &project.settings);
    let inside = ondalux_core::analysis::footprint_mask(project, &grid);
    bands
        .iter()
        .map(|&band| {
            let metrics = Metrics::compute(
                grid,
                &cache.layers(),
                Some(band),
                &project.settings,
                project.requirements,
            );
            let analysis = analyze_band(project, band, &metrics, &obstacles);
            let maps = MetricKind::ALL
                .into_iter()
                .map(|kind| {
                    let values: Vec<f32> = (0..grid.len())
                        .map(|i| {
                            let unusable =
                                metrics.serving[i] == NO_SERVER || metrics.rssi[i] < RSSI_CUTOFF;
                            let keep_unusable =
                                matches!(kind, MetricKind::Coverage | MetricKind::ApCount);
                            if !inside[i] || (unusable && !keep_unusable) {
                                f32::NAN
                            } else {
                                metrics.value(kind, i)
                            }
                        })
                        .collect();
                    let coverage = kind == MetricKind::Coverage;
                    let img = render_map(
                        project,
                        plan,
                        &MapOptions {
                            max_width_px: image_width_px,
                            heat: Some(HeatLayer {
                                grid,
                                values: &values,
                                scale: ColorScale::Metric(kind),
                                opacity: if coverage { 0.45 } else { 0.6 },
                            }),
                            gaps: coverage.then_some(GapOverlay {
                                grid,
                                labels: &analysis.gap_labels,
                                zones: &analysis.gaps,
                            }),
                            legend_title: Some(legend_title(kind, band, lang)),
                            band: Some(band),
                            label_scale: 2.4,
                            discrete_labels: coverage.then(|| {
                                vec![
                                    lang.t("No cumple", "Fails").to_owned(),
                                    lang.t("Cumple", "Meets").to_owned(),
                                ]
                            }),
                            ..Default::default()
                        },
                    );
                    (kind, img)
                })
                .collect();
            BandResult {
                band,
                metrics,
                analysis,
                maps,
            }
        })
        .collect()
}

fn pct(x: f32) -> String {
    format!("{:.1} %", x * 100.0)
}

fn names(v: &[String]) -> String {
    v.join(", ")
}

pub fn finding_text(f: &Finding, l: Lang) -> String {
    match f {
        Finding::NoAccessPoints => l
            .t(
                "No hay APs con radios activas en esta banda.",
                "No AP has an enabled radio in this band.",
            )
            .into(),
        Finding::LowCoverage { ratio } => format!(
            "{} {} {} {}.",
            l.t("La cobertura que cumple los requisitos es", "Coverage meeting the requirements is"),
            pct(*ratio),
            l.t("del área analizada; el objetivo es", "of the analysed area; the target is"),
            pct(TARGET_COVERAGE)
        ),
        Finding::AddApNearGap { zone, at, area_m2 } => format!(
            "{} {zone} ({area_m2:.0} m²): {} x = {:.1} m, y = {:.1} m, {}",
            l.t("Zona", "Zone"),
            l.t("considere agregar un AP cerca de", "consider adding an AP near"),
            at.x,
            at.y,
            l.t(
                "o aumentar la potencia de los APs cercanos.",
                "or raising the power of nearby APs."
            )
        ),
        Finding::ChannelOverlap { share } => format!(
            "{} {} {}",
            l.t("En el", "In"),
            pct(*share),
            l.t(
                "del área cubierta se escuchan 2 o más radios en canales solapados por encima del umbral CCA. Reduzca la potencia, use más canales o canales más angostos.",
                "of the covered area, 2 or more radios on overlapping channels are heard above the CCA threshold. Lower the power, use more channels or narrower channels."
            )
        ),
        Finding::CoChannelPair { a, b, channel, rssi } => format!(
            "{a} {} {b} {} {channel} ({:.0} dBm). {}",
            l.t("y", "and"),
            l.t("se escuchan entre sí en el canal", "hear each other on channel"),
            rssi.0,
            l.t(
                "Asigne canales distintos si es posible.",
                "Assign different channels if possible."
            )
        ),
        Finding::NonStandard24Channels { aps } => format!(
            "{} {}. {}",
            l.t("Radios de 2.4 GHz fuera de los canales 1/6/11:", "2.4 GHz radios outside channels 1/6/11:"),
            names(aps),
            l.t(
                "Use solo 1, 6 y 11 para evitar interferencia de canal adyacente.",
                "Use only 1, 6 and 11 to avoid adjacent-channel interference."
            )
        ),
        Finding::Wide24Channels { aps } => format!(
            "{} {}. {}",
            l.t("Radios de 2.4 GHz con 40 MHz:", "2.4 GHz radios using 40 MHz:"),
            names(aps),
            l.t(
                "En 2.4 GHz se recomienda 20 MHz.",
                "20 MHz is recommended in 2.4 GHz."
            )
        ),
        Finding::ReduceChannelWidth { width, aps } => format!(
            "{} {} MHz {} {}. {}",
            l.t("Canales de", "Channels of"),
            width.mhz(),
            l.t("en un diseño denso:", "in a dense design:"),
            names(aps),
            l.t(
                "Considere 40 o 20 MHz para tener más canales sin solapamiento.",
                "Consider 40 or 20 MHz to get more non-overlapping channels."
            )
        ),
        Finding::ModelBias {
            mean_db,
            rmse_db,
            points,
        } => format!(
            "{} {points} {}: {} {mean_db:+.1} dB, RMSE {rmse_db:.1} dB. {}",
            l.t("En", "At"),
            l.t("puntos medidos", "survey points"),
            l.t("diferencia media medido − predicho", "mean measured − predicted difference"),
            l.t(
                "Ajuste las atenuaciones de las paredes o el exponente de pérdida para calibrar el modelo.",
                "Adjust wall losses or the path loss exponent to calibrate the model."
            )
        ),
        Finding::MinorGaps { count, area_m2 } => format!(
            "{} {count} {} ({area_m2:.1} m²) {}",
            l.t("La cobertura cumple el objetivo, pero", "Coverage meets the target, but"),
            l.t("zona(s) pequeña(s)", "small area(s)"),
            l.t(
                "no cumplen los requisitos; vea la tabla de zonas.",
                "do not meet the requirements; see the table of areas."
            )
        ),
        Finding::AllGood => l
            .t(
                "La banda cumple los requisitos en toda el área analizada.",
                "The band meets the requirements across the analysed area.",
            )
            .into(),
    }
}

fn severity_color(s: &Severity) -> Rgb {
    match s {
        Severity::Critical => BAD,
        Severity::Warning => WARN,
        Severity::Info => ACCENT,
    }
}

fn coverage_color(ratio: f32) -> Rgb {
    if ratio >= TARGET_COVERAGE {
        OK
    } else if ratio >= 0.8 {
        WARN
    } else {
        BAD
    }
}

/// Measured − predicted best RSSI at survey points, for one band.
fn survey_bias(project: &Project, survey: usize, band: Band, metrics: &Metrics) -> Option<Finding> {
    let s = project.surveys.get(survey)?;
    let g = metrics.grid;
    let errs: Vec<f32> = s
        .measurements
        .iter()
        .filter_map(|m| {
            let measured = best_rssi(m, &project.survey_settings, Some(band))?;
            let (c, r) = (
                ((m.position.x - g.origin.x) / g.cell_m).floor(),
                ((m.position.y - g.origin.y) / g.cell_m).floor(),
            );
            if c < 0.0 || r < 0.0 || c as usize >= g.cols || r as usize >= g.rows {
                return None;
            }
            let i = r as usize * g.cols + c as usize;
            (metrics.serving[i] != NO_SERVER).then(|| measured.0 - metrics.rssi[i])
        })
        .collect();
    if errs.is_empty() {
        return None;
    }
    let n = errs.len() as f32;
    let mean = errs.iter().sum::<f32>() / n;
    let rmse = (errs.iter().map(|e| e * e).sum::<f32>() / n).sqrt();
    Some(Finding::ModelBias {
        mean_db: mean,
        rmse_db: rmse,
        points: errs.len(),
    })
}

fn radios_in(project: &Project, band: Band) -> usize {
    project
        .aps
        .iter()
        .flat_map(|a| a.enabled_radios())
        .filter(|(_, x)| x.channel.band == band)
        .count()
}

/// Builds the full PDF report.
pub fn build_pdf(project: &Project, plan: Option<&DecodedPlan>, opts: &ReportOptions) -> Vec<u8> {
    let l = opts.lang;
    let bands = if opts.bands.is_empty() {
        bands_in_use(project)
    } else {
        opts.bands.clone()
    };
    let date = opts.date.clone().unwrap_or_else(today);
    let mut results = compute_bands(project, plan, &bands, l, opts.image_width_px);
    if let Some(si) = opts.survey {
        for r in &mut results {
            if let Some(f) = survey_bias(project, si, r.band, &r.metrics) {
                r.analysis.findings.push(f);
            }
        }
    }

    let report_name = l.t("Informe de diseño WiFi", "WiFi design report");
    let title = format!("{report_name} — {}", project.name);
    let logo = RgbaImage::from_raw(128, 128, crate::logo::rgba(128)).expect("logo size");
    let mut doc = Doc::new(
        &title,
        &format!("{report_name} · {}", project.name),
        &format!("Ondalux Heatmap  ·  {}  ·  {date}", project.name),
        Some(&logo),
    );

    // ---- cover ----
    doc.fill_rect(0.0, 0.0, PAGE_W, 8.0, ACCENT);
    let logo_idx = doc.add_image(&logo);
    doc.draw_image(logo_idx, MARGIN, 40.0, 42.0, 42.0);
    doc.text_at(MARGIN + 54.0, 45.0, "Ondalux Heatmap", 14.0, true, INK);
    doc.text_at(
        MARGIN + 54.0,
        64.0,
        l.t(
            "Diseño y medición de redes WiFi",
            "WiFi network design and survey",
        ),
        9.0,
        false,
        MUTED,
    );
    doc.set_y(116.0);
    doc.text(&report_name.to_uppercase(), 9.5, true, ACCENT, 0.0);
    doc.space(4.0);
    doc.text(&project.name, 28.0, true, INK, 0.0);
    doc.space(2.0);
    let mut meta = date.clone();
    if !opts.author.trim().is_empty() {
        meta = format!("{meta}  ·  {}", opts.author.trim());
    }
    doc.text(&meta, 10.5, false, MUTED, 0.0);
    doc.space(16.0);

    let area: f32 = results.first().map_or(0.0, |r| {
        r.analysis.analysed_cells as f32 * r.metrics.grid.cell_m * r.metrics.grid.cell_m
    });
    let mut kpis: Vec<Kpi> = results
        .iter()
        .map(|r| Kpi {
            value: format!("{:.0} %", r.analysis.coverage_ratio * 100.0),
            label: format!("{} {}", l.t("cobertura", "coverage"), r.band.label()),
            color: coverage_color(r.analysis.coverage_ratio),
        })
        .collect();
    kpis.truncate(3);
    kpis.push(Kpi {
        value: project.aps.len().to_string(),
        label: l.t("puntos de acceso", "access points").into(),
        color: INK,
    });
    if kpis.len() < 4 {
        kpis.push(Kpi {
            value: project.walls.len().to_string(),
            label: l.t("paredes", "walls").into(),
            color: INK,
        });
    }
    if kpis.len() < 5 && area > 0.0 {
        kpis.push(Kpi {
            value: format!("{area:.0} m²"),
            label: l.t("área analizada", "analysed area").into(),
            color: INK,
        });
    }
    doc.kpis(&kpis);

    let placement = placement_map(project, plan, opts.image_width_px);
    let room = CONTENT_BOTTOM - doc.y() - 70.0;
    doc.figure_card(
        l.t("Sembrado de puntos de acceso", "Access point placement"),
        &format!(
            "{} {} {}",
            l.t("Ubicación de los", "Location of the"),
            project.aps.len(),
            l.t(
                "puntos de acceso sobre el plano.",
                "access points on the floor plan."
            )
        ),
        &placement,
        room.max(200.0),
    );

    // ---- project data ----
    doc.new_page();
    doc.section(
        l.t("Resumen", "Summary"),
        l.t("Datos del proyecto", "Project data"),
        l.t(
            "Parámetros usados para la predicción y resultados por banda.",
            "Parameters used for the prediction and results per band.",
        ),
    );
    let (w, h) = project.plan.size_m();
    let mut kv = vec![
        (l.t("Fecha", "Date").to_owned(), date.clone()),
        (
            l.t("Plano", "Floor plan").to_owned(),
            format!(
                "{w:.1} × {h:.1} m  ({} × {} px, {})",
                project.plan.width_px,
                project.plan.height_px,
                match project.plan.calibration {
                    Some(c) => format!("{:.1} px/m", c.px_per_m()),
                    None => l
                        .t("escala sin calibrar", "scale not calibrated")
                        .to_owned(),
                }
            ),
        ),
        (
            l.t("Puntos de acceso", "Access points").to_owned(),
            project.aps.len().to_string(),
        ),
        (
            l.t("Paredes", "Walls").to_owned(),
            project.walls.len().to_string(),
        ),
        (
            l.t("Requisitos", "Requirements").to_owned(),
            format!(
                "{} ≥ {:.0} dBm, SNR ≥ {:.0} dB, CCA {:.0} dBm",
                l.t("señal", "signal"),
                project.requirements.min_rssi.0,
                project.requirements.min_snr.0,
                project.requirements.cca_threshold.0
            ),
        ),
        (
            l.t("Modelo de propagación", "Propagation model").to_owned(),
            match &project.settings.model {
                PathLossModel::FreeSpace => l.t("espacio libre", "free space").to_owned(),
                PathLossModel::LogDistance { exponent, .. } => format!(
                    "log-distance, n = {:.1} / {:.1} / {:.1} (2.4 / 5 / 6 GHz)",
                    exponent.g2_4, exponent.g5, exponent.g6
                ),
            } + &format!(
                ", {} {:.1} m",
                l.t("altura del cliente", "client height"),
                project.settings.receiver_height_m
            ),
        ),
    ];
    if !opts.author.trim().is_empty() {
        kv.insert(1, (l.t("Autor", "Author").to_owned(), opts.author.clone()));
    }
    doc.key_values(&kv);

    doc.heading(l.t("Resultados por banda", "Results by band"), 13.0);
    if results.is_empty() {
        doc.callout(
            l.t(
                "El proyecto no tiene APs con radios activas.",
                "The project has no AP with an enabled radio.",
            ),
            WARN,
        );
    } else {
        let header = vec![
            l.t("Banda", "Band").to_owned(),
            l.t("Radios", "Radios").to_owned(),
            l.t("Cobertura", "Coverage").to_owned(),
            l.t("Zonas sin cobertura", "Coverage gaps").to_owned(),
            l.t("Solapamiento ≥ 2", "Overlap ≥ 2").to_owned(),
        ];
        let rows: Vec<Vec<String>> = results
            .iter()
            .map(|r| {
                vec![
                    r.band.label().to_owned(),
                    radios_in(project, r.band).to_string(),
                    pct(r.analysis.coverage_ratio),
                    r.analysis.gaps.len().to_string(),
                    pct(r.analysis.overlap_share),
                ]
            })
            .collect();
        doc.table(&header, &rows, &[1.0, 1.0, 1.2, 1.4, 1.4]);
    }

    // ---- per band ----
    for r in &results {
        doc.new_page();
        let area = r.analysis.analysed_cells as f32 * r.metrics.grid.cell_m * r.metrics.grid.cell_m;
        doc.section(
            &format!("{} {}", l.t("Banda", "Band"), r.band.label()),
            &format!(
                "{} {}",
                l.t("Mapas de calor ·", "Heat maps ·"),
                r.band.label()
            ),
            &format!(
                "{} ({area:.0} m²).",
                l.t(
                    "Área analizada: interior del contorno de paredes",
                    "Analysed area: inside the outline of the walls"
                )
            ),
        );
        doc.kpis(&[
            Kpi {
                value: pct(r.analysis.coverage_ratio),
                label: l
                    .t("cumple los requisitos", "meets the requirements")
                    .into(),
                color: coverage_color(r.analysis.coverage_ratio),
            },
            Kpi {
                value: r.analysis.gaps.len().to_string(),
                label: l.t("zonas sin cobertura", "coverage gaps").into(),
                color: if r.analysis.gaps.is_empty() { OK } else { WARN },
            },
            Kpi {
                value: pct(r.analysis.overlap_share),
                label: l
                    .t("con 2 o más radios solapadas", "with 2+ overlapping radios")
                    .into(),
                color: if r.analysis.overlap_share > 0.2 {
                    WARN
                } else {
                    INK
                },
            },
            Kpi {
                value: radios_in(project, r.band).to_string(),
                label: l.t("radios activas", "enabled radios").into(),
                color: INK,
            },
        ]);
        for (kind, img) in &r.maps {
            let (title, caption) = metric_text(*kind, project, l);
            doc.figure_card(&title, &caption, img, 192.0);
        }

        if !r.analysis.gaps.is_empty() {
            doc.heading(
                l.t("Zonas con cobertura deficiente", "Areas with poor coverage"),
                13.0,
            );
            let header = vec![
                "#".to_owned(),
                l.t("Área", "Area").to_owned(),
                l.t("Centro (x, y)", "Center (x, y)").to_owned(),
                l.t("Mejor señal", "Best signal").to_owned(),
            ];
            let rows: Vec<Vec<String>> = r
                .analysis
                .gaps
                .iter()
                .take(25)
                .map(|z| {
                    vec![
                        z.number.to_string(),
                        format!("{:.1} m²", z.area_m2),
                        format!("{:.1}, {:.1} m", z.centroid.x, z.centroid.y),
                        z.best_rssi
                            .map_or(l.t("sin señal", "no signal").to_owned(), |v| {
                                format!("{:.0} dBm", v.0)
                            }),
                    ]
                })
                .collect();
            doc.table(&header, &rows, &[0.5, 1.0, 1.6, 1.2]);
            if r.analysis.gaps.len() > 25 {
                doc.text(
                    &format!(
                        "{} {} {}",
                        l.t("…y", "…and"),
                        r.analysis.gaps.len() - 25,
                        l.t("zonas más pequeñas.", "smaller areas.")
                    ),
                    9.0,
                    false,
                    MUTED,
                    0.0,
                );
            }
        }

        doc.heading(l.t("Recomendaciones", "Recommendations"), 13.0);
        for f in &r.analysis.findings {
            doc.callout(&finding_text(f, l), severity_color(&f.severity()));
        }
    }

    // ---- AP table ----
    doc.new_page();
    doc.section(
        l.t("Inventario", "Inventory"),
        l.t("Puntos de acceso", "Access points"),
        &format!(
            "{} {}",
            project.aps.len(),
            l.t(
                "puntos de acceso con su modelo, montaje y radios.",
                "access points with their model, mounting and radios."
            )
        ),
    );
    let header = vec![
        l.t("Nombre", "Name").to_owned(),
        l.t("Modelo", "Model").to_owned(),
        l.t("Posición", "Position").to_owned(),
        l.t("Montaje", "Mounting").to_owned(),
        l.t(
            "Radios (canal/ancho, Tx + ganancia = EIRP)",
            "Radios (channel/width, Tx + gain = EIRP)",
        )
        .to_owned(),
    ];
    let rows: Vec<Vec<String>> = project
        .aps
        .iter()
        .map(|a| {
            let model = format!("{} {}", a.vendor, a.model).trim().to_owned();
            let radios = a
                .radios
                .iter()
                .map(|r| {
                    format!(
                        "{} ch {}/{}: {:.0}+{:.1}={:.1} dBm{}",
                        r.channel.band.label(),
                        r.channel.number,
                        r.channel.width.mhz(),
                        r.tx_power.0,
                        r.antenna_gain.0,
                        r.eirp().0,
                        if r.enabled {
                            ""
                        } else {
                            l.t(" (apagada)", " (off)")
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            vec![
                a.name.clone(),
                if model.is_empty() {
                    "—".into()
                } else {
                    model
                },
                format!("{:.1}, {:.1} m", a.position.x, a.position.y),
                match a.mount {
                    Mount::Ceiling => format!("{} {:.1} m", l.t("Techo", "Ceiling"), a.height_m),
                    Mount::Wall => format!(
                        "{} {:.1} m, {:.0}°",
                        l.t("Pared", "Wall"),
                        a.height_m,
                        a.orientation_deg
                    ),
                },
                radios,
            ]
        })
        .collect();
    doc.table(&header, &rows, &[1.0, 1.4, 1.1, 1.0, 3.0]);

    // ---- method ----
    doc.section(
        l.t("Metodología", "Method"),
        l.t("Cómo se calculó", "How it was computed"),
        "",
    );
    doc.paragraph(l.t(
        "Predicción con pérdida de trayecto log-distance o de espacio libre más la atenuación de cada pared atravesada por la línea directa entre el AP y cada punto (modelo 2D de un piso). No se modelan reflexiones ni difracción. Los APs en techo se consideran omnidireccionales; los APs en pared usan un patrón horizontal simplificado (apertura del haz y pérdida trasera) según su dirección. La interferencia (SINR) solo considera radios en el mismo canal o en canales que comparten espectro, con la fracción de espectro solapada; los APs en canales distintos no se interfieren. Los valores de potencia y ganancia de los modelos de catálogo provienen de las fichas de cada fabricante; los límites regulatorios locales pueden ser menores. Las mediciones de campo, si se incluyen, permiten calibrar el modelo.",
        "Prediction with log-distance or free-space path loss plus the loss of every wall crossed by the straight path between the AP and each point (2D, single floor). Reflection and diffraction are not modelled. Ceiling APs are treated as omnidirectional; wall APs use a simplified horizontal pattern (beamwidth and front-to-back loss) along their pointing direction. Interference (SINR) only counts radios on the same channel or on channels sharing spectrum, by their overlapping fraction; APs on different channels do not interfere. Power and gain of catalog models come from vendor datasheets; local regulatory limits may be lower. Field measurements, when included, allow calibrating the model.",
    ));
    doc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::ap::{AccessPoint, ApId, Radio};
    use ondalux_core::channel::{Channel, ChannelWidth};
    use ondalux_core::geometry::{Segment, Vec2};
    use ondalux_core::material::{MaterialId, Wall};
    use ondalux_core::units::{Db, Dbm};

    fn project() -> Project {
        let mut p = Project::new("Oficina demo", 800, 400); // 40 × 20 m
        p.grid_cell_m = 0.5;
        let c = [
            Vec2::new(0.0, 0.0),
            Vec2::new(40.0, 0.0),
            Vec2::new(40.0, 20.0),
            Vec2::new(0.0, 20.0),
        ];
        for k in 0..4 {
            p.walls.push(Wall {
                segment: Segment::new(c[k], c[(k + 1) % 4]),
                material: MaterialId(5),
            });
        }
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(20.0, 0.0), Vec2::new(20.0, 20.0)),
            material: MaterialId(6),
        });
        p.aps.push(AccessPoint {
            id: ApId(1),
            name: "AP-1".into(),
            vendor: "Ubiquiti".into(),
            model: "U6 Pro".into(),
            position: Vec2::new(8.0, 10.0),
            height_m: 3.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![
                Radio {
                    channel: Channel::new(Band::G2_4, 3, ChannelWidth::Mhz20).unwrap(),
                    tx_power: Dbm(17.0),
                    antenna_gain: Db(4.0),
                    enabled: true,
                },
                Radio {
                    channel: Channel::new(Band::G5, 36, ChannelWidth::Mhz40).unwrap(),
                    tx_power: Dbm(20.0),
                    antenna_gain: Db(5.0),
                    enabled: true,
                },
            ],
        });
        p
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(20_739), "2026-10-13");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    #[test]
    fn report_contains_bands_gaps_and_recommendations() {
        let p = project();
        let results = compute_bands(&p, None, &bands_in_use(&p), Lang::Es, 600);
        assert_eq!(results.len(), 2);
        let g5 = results.iter().find(|r| r.band == Band::G5).unwrap();
        // The metal wall at x = 20 leaves the east half poorly covered.
        assert!(!g5.analysis.gaps.is_empty());
        assert!(g5.analysis.gaps[0].centroid.x > 20.0);
        let g24 = results.iter().find(|r| r.band == Band::G2_4).unwrap();
        assert!(g24
            .analysis
            .findings
            .iter()
            .any(|f| matches!(f, Finding::NonStandard24Channels { .. })));
        assert_eq!(g5.maps.len(), MetricKind::ALL.len());
        assert_eq!(g5.map(MetricKind::Rssi).unwrap().width(), 600);
    }

    #[test]
    fn pdf_renders_in_both_languages() {
        let p = project();
        for lang in [Lang::Es, Lang::En] {
            let bytes = build_pdf(
                &p,
                None,
                &ReportOptions {
                    lang,
                    author: "Equipo de redes".into(),
                    image_width_px: 600,
                    date: Some("2026-10-13".into()),
                    ..Default::default()
                },
            );
            assert!(bytes.starts_with(b"%PDF-"));
            let pdf = hayro::hayro_syntax::Pdf::new(bytes).expect("valid PDF");
            // Summary + 2 bands + AP table.
            assert!(pdf.pages().len() >= 4, "{}", pdf.pages().len());
        }
    }

    #[test]
    fn findings_are_localised() {
        let f = Finding::NonStandard24Channels {
            aps: vec!["AP-1".into()],
        };
        assert!(finding_text(&f, Lang::Es).contains("canales 1/6/11"));
        assert!(finding_text(&f, Lang::En).contains("channels 1/6/11"));
    }
}
