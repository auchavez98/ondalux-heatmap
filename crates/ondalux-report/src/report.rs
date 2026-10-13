//! Builds the PDF design report: summary, heat maps per band, coverage
//! gaps, recommendations, AP table and (optionally) survey comparison.

use crate::map::{render_map, GapOverlay, HeatLayer, MapOptions};
use crate::pdfdoc::{Doc, MUTED, TEXT};
use image::RgbaImage;
use ondalux_core::analysis::{analyze_band, BandAnalysis, Finding, Severity, TARGET_COVERAGE};
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
    pub signal_map: RgbaImage,
    pub coverage_map: RgbaImage,
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
            let signal: Vec<f32> = (0..grid.len())
                .map(|i| {
                    if metrics.serving[i] == NO_SERVER || metrics.rssi[i] < RSSI_CUTOFF {
                        f32::NAN
                    } else {
                        metrics.rssi[i]
                    }
                })
                .collect();
            let inside = ondalux_core::analysis::footprint_mask(project, &grid);
            let coverage: Vec<f32> = (0..grid.len())
                .map(|i| {
                    if inside[i] {
                        f32::from(u8::from(metrics.meets_requirements(i)))
                    } else {
                        f32::NAN
                    }
                })
                .collect();
            let signal_map = render_map(
                project,
                plan,
                &MapOptions {
                    max_width_px: image_width_px,
                    heat: Some(HeatLayer {
                        grid,
                        values: &signal,
                        scale: ColorScale::Metric(MetricKind::Rssi),
                        opacity: 0.6,
                    }),
                    legend_title: Some(format!(
                        "{} {} (dBm)",
                        lang.t("Señal", "Signal"),
                        band.label()
                    )),
                    band: Some(band),
                    ..Default::default()
                },
            );
            let coverage_map = render_map(
                project,
                plan,
                &MapOptions {
                    max_width_px: image_width_px,
                    heat: Some(HeatLayer {
                        grid,
                        values: &coverage,
                        scale: ColorScale::Metric(MetricKind::Coverage),
                        opacity: 0.45,
                    }),
                    gaps: Some(GapOverlay {
                        grid,
                        labels: &analysis.gap_labels,
                        zones: &analysis.gaps,
                    }),
                    legend_title: Some(format!(
                        "{} ≥ {:.0} dBm, SNR ≥ {:.0} dB",
                        lang.t("Cobertura", "Coverage"),
                        project.requirements.min_rssi.0,
                        project.requirements.min_snr.0
                    )),
                    band: Some(band),
                    ..Default::default()
                },
            );
            BandResult {
                band,
                metrics,
                analysis,
                signal_map,
                coverage_map,
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
        Finding::AllGood => l
            .t(
                "La banda cumple los requisitos en toda el área analizada.",
                "The band meets the requirements across the analysed area.",
            )
            .into(),
    }
}

fn severity_color(s: &Severity) -> [f32; 3] {
    match s {
        Severity::Critical => [0.75, 0.1, 0.1],
        Severity::Warning => [0.85, 0.5, 0.05],
        Severity::Info => [0.15, 0.45, 0.75],
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

    let title = format!(
        "{} — {}",
        l.t("Informe de diseño WiFi", "WiFi design report"),
        project.name
    );
    let mut doc = Doc::new(
        &title,
        &format!("Ondalux Heatmap · {} · {date}", project.name),
    );

    // ---- summary ----
    doc.text(
        l.t("Informe de diseño WiFi", "WiFi design report"),
        22.0,
        true,
        TEXT,
        0.0,
    );
    doc.text(&project.name, 15.0, false, MUTED, 0.0);
    doc.space(6.0);
    let (w, h) = project.plan.size_m();
    let mut kv = vec![
        (l.t("Fecha", "Date").to_owned(), date.clone()),
        (
            l.t("Plano", "Floor plan").to_owned(),
            format!(
                "{} × {} px, {w:.1} × {h:.1} m ({})",
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

    doc.heading(l.t("Resumen por banda", "Summary by band"), 14.0);
    if results.is_empty() {
        doc.paragraph(l.t(
            "El proyecto no tiene APs con radios activas.",
            "The project has no AP with an enabled radio.",
        ));
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
                let radios = project
                    .aps
                    .iter()
                    .flat_map(|a| a.enabled_radios())
                    .filter(|(_, x)| x.channel.band == r.band)
                    .count();
                vec![
                    r.band.label().to_owned(),
                    radios.to_string(),
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
        doc.heading(
            &format!("{} {}", l.t("Banda", "Band"), r.band.label()),
            16.0,
        );
        let area = r.analysis.analysed_cells as f32 * r.metrics.grid.cell_m * r.metrics.grid.cell_m;
        doc.paragraph(&format!(
            "{} {}. {} ({area:.0} m²).",
            l.t(
                "Cobertura que cumple los requisitos:",
                "Coverage meeting the requirements:"
            ),
            pct(r.analysis.coverage_ratio),
            l.t(
                "Área analizada: interior del contorno de paredes",
                "Analysed area: inside the outline of the walls"
            ),
        ));
        doc.text(
            l.t("Intensidad de señal", "Signal strength"),
            11.0,
            true,
            TEXT,
            0.0,
        );
        doc.image(&r.signal_map, 330.0);
        doc.text(
            l.t(
                "Cobertura según requisitos (zonas sin cobertura numeradas)",
                "Coverage against requirements (gaps numbered)",
            ),
            11.0,
            true,
            TEXT,
            0.0,
        );
        doc.image(&r.coverage_map, 330.0);

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
            doc.bullet(&finding_text(f, l), severity_color(&f.severity()));
        }
    }

    // ---- AP table ----
    doc.new_page();
    doc.heading(l.t("Puntos de acceso", "Access points"), 16.0);
    let header = vec![
        l.t("Nombre", "Name").to_owned(),
        l.t("Modelo", "Model").to_owned(),
        l.t("Posición", "Position").to_owned(),
        l.t("Altura", "Height").to_owned(),
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
                format!("{:.1} m", a.height_m),
                radios,
            ]
        })
        .collect();
    doc.table(&header, &rows, &[1.0, 1.4, 1.1, 0.7, 3.2]);

    // ---- method ----
    doc.heading(
        l.t("Metodología y limitaciones", "Method and limitations"),
        13.0,
    );
    doc.paragraph(l.t(
        "Predicción con pérdida de trayecto log-distance o de espacio libre más la atenuación de cada pared atravesada por la línea directa entre el AP y cada punto (modelo 2D de un piso). No se modelan reflexiones, difracción ni antenas direccionales. La interferencia considera canales solapados con su fracción de espectro. Los valores de potencia y ganancia de los modelos de catálogo provienen de las fichas de cada fabricante; los límites regulatorios locales pueden ser menores. Las mediciones de campo, si se incluyen, permiten calibrar el modelo.",
        "Prediction with log-distance or free-space path loss plus the loss of every wall crossed by the straight path between the AP and each point (2D, single floor). Reflection, diffraction and directional antennas are not modelled. Interference accounts for overlapping channels by their spectral overlap. Power and gain of catalog models come from vendor datasheets; local regulatory limits may be lower. Field measurements, when included, allow calibrating the model.",
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
        assert_eq!(g5.signal_map.width(), 600);
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
