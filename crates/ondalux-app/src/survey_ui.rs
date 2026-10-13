//! Field survey: background scans, survey management and the survey panel.

use crate::app::{OndaluxApp, Selection, Tool};
use crate::heatmap::Source;
use eframe::egui::{self, DragValue};
use ondalux_core::geometry::Vec2;
use ondalux_core::interpolate::InterpolationMethod;
use ondalux_core::measured::MeasuredSettings;
use ondalux_core::project::Project;
use ondalux_core::survey::{BssReading, Measurement, Survey};
use ondalux_io::survey_csv::{read_survey_csv, write_survey_csv};
use ondalux_scan::{platform_scanner, ScanError, SimulatedScanner, WifiScanner};
use std::sync::mpsc::{self, Receiver};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerChoice {
    /// The OS WiFi adapter.
    Native,
    /// Generated from the prediction (demo / training).
    Simulated,
}

type ScanResult = (
    Option<Box<dyn WifiScanner>>,
    Result<Vec<BssReading>, ScanError>,
);

struct ScanJob {
    position: Vec2,
    rx: Receiver<ScanResult>,
    started: Instant,
    source: String,
}

pub struct SurveyState {
    /// Index of the active survey in `project.surveys`.
    pub active: usize,
    pub scanner: ScannerChoice,
    native: Option<Box<dyn WifiScanner>>,
    native_error: Option<String>,
    job: Option<ScanJob>,
    pub csv_warnings: Vec<String>,
}

impl Default for SurveyState {
    fn default() -> Self {
        Self {
            active: 0,
            scanner: ScannerChoice::Native,
            native: None,
            native_error: None,
            job: None,
            csv_warnings: Vec::new(),
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl SurveyState {
    pub fn scanning_at(&self) -> Option<Vec2> {
        self.job.as_ref().map(|j| j.position)
    }

    pub fn native_name(&self) -> String {
        match (&self.native, &self.native_error) {
            (Some(s), _) => s.name().to_owned(),
            (None, Some(e)) => format!("unavailable: {e}"),
            (None, None) => "OS WiFi adapter".to_owned(),
        }
    }

    /// Starts a scan at `position` in the background.
    pub fn start_scan(
        &mut self,
        position: Vec2,
        project: &Project,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err("A scan is already running; wait for it to finish.".into());
        }
        let mut scanner: Box<dyn WifiScanner> = match self.scanner {
            ScannerChoice::Native => match self.native.take() {
                Some(s) => s,
                None => match platform_scanner() {
                    Ok(s) => {
                        self.native_error = None;
                        s
                    }
                    Err(e) => {
                        self.native_error = Some(e.to_string());
                        return Err(e.to_string());
                    }
                },
            },
            ScannerChoice::Simulated => Box::new(SimulatedScanner::new(
                project.aps.clone(),
                &project.walls,
                &project.materials,
                project.settings.clone(),
                now_ms() as u64,
            )),
        };
        let keep = self.scanner == ScannerChoice::Native;
        let source = scanner.name().to_owned();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("wifi-scan".into())
            .spawn(move || {
                let result = scanner.scan(position);
                let _ = tx.send((keep.then_some(scanner), result));
                ctx.request_repaint();
            })
            .map_err(|e| e.to_string())?;
        self.job = Some(ScanJob {
            position,
            rx,
            started: Instant::now(),
            source,
        });
        Ok(())
    }
}

/// Collects a finished scan, if any, into the active survey.
pub fn poll(app: &mut OndaluxApp) {
    let Some(job) = &app.survey.job else {
        return;
    };
    let Ok((scanner, result)) = job.rx.try_recv() else {
        return;
    };
    let job = app.survey.job.take().expect("checked above");
    if let Some(s) = scanner {
        app.survey.native = Some(s);
    }
    let readings = match result {
        Ok(r) => r,
        Err(e) => {
            if job.source != ondalux_scan::simulated::NAME {
                app.survey.native_error = Some(e.to_string());
            }
            app.error(e.to_string());
            return;
        }
    };
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    doc.checkpoint();
    let surveys = &mut doc.project.surveys;
    if surveys.is_empty() {
        surveys.push(Survey::new("Survey 1", job.source.clone()));
        app.survey.active = 0;
    }
    let active = app.survey.active.min(surveys.len() - 1);
    let survey = &mut surveys[active];
    if survey.measurements.is_empty() {
        survey.source = job.source.clone();
    } else if survey.source != job.source && !survey.source.starts_with("Mixed") {
        survey.source = format!("Mixed ({} + {})", survey.source, job.source);
    }
    let id = survey.next_id();
    let n = readings.len();
    survey.measurements.push(Measurement {
        id,
        position: job.position,
        timestamp_ms: now_ms(),
        readings,
    });
    app.selection = Selection::Measurement(id);
    app.info(format!(
        "Point {id}: {n} networks heard in {:.1} s.",
        job.started.elapsed().as_secs_f32()
    ));
}

pub fn active_survey(app: &OndaluxApp) -> Option<&Survey> {
    let doc = app.doc.as_ref()?;
    doc.project.surveys.get(app.survey.active)
}

fn import_csv(app: &mut OndaluxApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import survey CSV")
        .add_filter("CSV", &["csv", "txt"])
        .pick_file()
    else {
        return;
    };
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let name = path.file_name().map_or_else(
        || "survey.csv".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let result = std::fs::File::open(&path)
        .map_err(ondalux_io::IoError::from)
        .and_then(|f| read_survey_csv(std::io::BufReader::new(f), &name, &doc.project.plan));
    match result {
        Ok(import) => {
            doc.checkpoint();
            let points = import.survey.measurements.len();
            doc.project.surveys.push(import.survey);
            app.survey.active = doc.project.surveys.len() - 1;
            let skipped = import.warnings.len();
            app.survey.csv_warnings = import.warnings;
            if skipped == 0 {
                app.info(format!("Imported {points} points from {name}."));
            } else {
                app.error(format!(
                    "Imported {points} points from {name}; {skipped} rows skipped (see Survey panel)."
                ));
            }
        }
        Err(e) => app.error(format!("Could not import {name}: {e}")),
    }
}

fn export_csv(app: &mut OndaluxApp) {
    let Some(survey) = active_survey(app).cloned() else {
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export survey CSV")
        .set_file_name(format!("{}.csv", survey.name))
        .add_filter("CSV", &["csv"])
        .save_file()
    else {
        return;
    };
    let result = std::fs::File::create(&path)
        .map_err(ondalux_io::IoError::from)
        .and_then(|f| write_survey_csv(std::io::BufWriter::new(f), &survey));
    match result {
        Ok(()) => app.info(format!("Exported {}.", path.display())),
        Err(e) => app.error(format!("Could not export: {e}")),
    }
}

pub fn panel(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    if app.doc.is_none() {
        return;
    }
    ui.heading("Field survey");

    // ---- survey selection ----
    let mut new_survey = false;
    let mut delete = false;
    {
        let doc = app.doc.as_ref().expect("checked above");
        let surveys = &doc.project.surveys;
        if app.survey.active >= surveys.len() {
            app.survey.active = surveys.len().saturating_sub(1);
        }
        ui.horizontal(|ui| {
            let current = surveys
                .get(app.survey.active)
                .map_or("(none)".to_owned(), |s| s.name.clone());
            egui::ComboBox::from_id_salt("active-survey")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (i, s) in surveys.iter().enumerate() {
                        ui.selectable_value(&mut app.survey.active, i, &s.name);
                    }
                });
            new_survey = ui.button("New").clicked();
            delete = ui
                .add_enabled(!surveys.is_empty(), egui::Button::new("Delete"))
                .clicked();
        });
    }
    let doc = app.doc.as_mut().expect("checked above");
    if new_survey {
        doc.checkpoint();
        let n = doc.project.surveys.len() + 1;
        doc.project
            .surveys
            .push(Survey::new(format!("Survey {n}"), ""));
        app.survey.active = n - 1;
    }
    if delete && app.survey.active < doc.project.surveys.len() {
        doc.checkpoint();
        doc.project.surveys.remove(app.survey.active);
        app.survey.active = app.survey.active.saturating_sub(1);
        app.selection = Selection::None;
    }
    if let Some(s) = doc.project.surveys.get(app.survey.active) {
        let mut name = s.name.clone();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut name);
        });
        if name != s.name {
            doc.checkpoint_coalesced("survey-name");
            doc.project.surveys[app.survey.active].name = name;
        }
        let s = &doc.project.surveys[app.survey.active];
        let bss = s.bss_summary();
        ui.weak(format!(
            "{} points · {} BSSIDs · source: {}",
            s.measurements.len(),
            bss.len(),
            if s.source.is_empty() {
                "—"
            } else {
                &s.source
            }
        ));
    }

    // ---- scanner ----
    ui.separator();
    ui.strong("Scanner");
    let native_label = format!("This computer ({})", app.survey.native_name());
    ui.radio_value(&mut app.survey.scanner, ScannerChoice::Native, native_label);
    ui.radio_value(
        &mut app.survey.scanner,
        ScannerChoice::Simulated,
        "Simulated from the prediction",
    );
    if app.survey.scanner == ScannerChoice::Simulated {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "Simulated readings are generated from the design plus random fading. They are not real measurements.",
        );
    }
    if let Some(at) = app.survey.scanning_at() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!(
                "Scanning at ({:.1}, {:.1}) m… stay still.",
                at.x, at.y
            ));
        });
    } else if app.tool == Tool::Survey {
        ui.weak("Click on the plan where you are standing to record a scan.");
    } else if ui.button("Start surveying (S)").clicked() {
        app.set_tool(Tool::Survey);
    }
    ui.horizontal(|ui| {
        if ui.button("Import CSV…").clicked() {
            import_csv(app);
        }
        if ui
            .add_enabled(
                active_survey(app).is_some(),
                egui::Button::new("Export CSV…"),
            )
            .clicked()
        {
            export_csv(app);
        }
    });
    if !app.survey.csv_warnings.is_empty() {
        egui::CollapsingHeader::new(format!(
            "{} CSV rows skipped",
            app.survey.csv_warnings.len()
        ))
        .show(ui, |ui| {
            for w in app.survey.csv_warnings.iter().take(50) {
                ui.weak(w);
            }
        });
    }

    // ---- networks and interpolation ----
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let orig: MeasuredSettings = doc.project.survey_settings.clone();
    let mut st = orig.clone();
    let ssids = doc
        .project
        .surveys
        .get(app.survey.active)
        .map(|s| s.ssids())
        .unwrap_or_default();
    ui.separator();
    ui.strong("Networks");
    let mut all = st.ssids.is_empty();
    if ui.checkbox(&mut all, "All networks heard").changed() {
        st.ssids = if all {
            Vec::new()
        } else {
            ssids.first().cloned().into_iter().collect()
        };
    }
    if !all {
        egui::ScrollArea::vertical()
            .id_salt("ssid-list")
            .max_height(140.0)
            .show(ui, |ui| {
                for ssid in &ssids {
                    let mut on = st.ssids.contains(ssid);
                    let label = if ssid.is_empty() {
                        "(hidden)"
                    } else {
                        ssid.as_str()
                    };
                    if ui.checkbox(&mut on, label).changed() {
                        if on {
                            st.ssids.push(ssid.clone());
                        } else {
                            st.ssids.retain(|s| s != ssid);
                        }
                    }
                }
            });
    }
    ui.separator();
    ui.strong("Interpolation");
    let ip = &mut st.interpolation;
    ui.horizontal(|ui| {
        ui.radio_value(&mut ip.method, InterpolationMethod::Idw, "IDW");
        ui.radio_value(&mut ip.method, InterpolationMethod::Kriging, "Kriging");
    });
    egui::Grid::new("interp").num_columns(2).show(ui, |ui| {
        if ip.method == InterpolationMethod::Idw {
            ui.label("Power");
            ui.add(
                DragValue::new(&mut ip.idw_power)
                    .range(1.0..=4.0)
                    .speed(0.05),
            );
            ui.end_row();
        }
        ui.label("Neighbors");
        ui.add(DragValue::new(&mut ip.neighbors).range(3..=32));
        ui.end_row();
        ui.label("Max distance");
        ui.add(
            DragValue::new(&mut ip.max_distance_m)
                .range(1.0..=50.0)
                .speed(0.1)
                .suffix(" m"),
        );
        ui.end_row();
        ui.label("Not heard =");
        ui.add(
            DragValue::new(&mut st.missing_rssi.0)
                .range(-120.0..=-80.0)
                .suffix(" dBm"),
        );
        ui.end_row();
    });
    if let Some(v) = &app.heat.variogram {
        ui.weak(format!(
            "Fitted variogram: nugget {:.1}, sill {:.1} dB², range {:.1} m",
            v.nugget,
            v.nugget + v.psill,
            v.range
        ));
    }
    if st != orig {
        doc.checkpoint_coalesced("survey-settings");
        doc.project.survey_settings = st;
    }

    // ---- comparison ----
    ui.separator();
    ui.strong("Measured vs predicted");
    match &app.heat.comparison {
        Some(c) if !c.points.is_empty() => {
            ui.label(format!(
                "{} points ({}): mean {:+.1} dB, RMSE {:.1} dB",
                c.points.len(),
                app.heat.band,
                c.mean,
                c.rmse
            ));
            ui.weak(if c.mean > 3.0 {
                "Measured signal is stronger than predicted: the model is pessimistic (walls or exponent too high)."
            } else if c.mean < -3.0 {
                "Measured signal is weaker than predicted: the model is optimistic (missing walls, losses too low, or AP settings differ)."
            } else {
                "On average the prediction matches the measurements within ±3 dB."
            });
        }
        _ => {
            ui.weak("Record or import measurements in this band to compare.");
        }
    }
    ui.horizontal(|ui| {
        ui.label("Show:");
        for s in Source::ALL {
            ui.selectable_value(&mut app.heat.source, s, s.label());
        }
    });
    if app.heat.source == Source::Difference {
        ui.add(
            egui::Slider::new(&mut app.heat.diff_threshold, 0.0..=15.0)
                .text("hide |Δ| below (dB)")
                .step_by(0.5),
        );
    }
}

pub fn measurement_inspector(app: &mut OndaluxApp, ui: &mut egui::Ui, id: u32) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let Some(survey) = doc.project.surveys.get(app.survey.active) else {
        app.selection = Selection::None;
        return;
    };
    let Some(m) = survey.measurements.iter().find(|m| m.id == id).cloned() else {
        app.selection = Selection::None;
        return;
    };
    ui.heading(format!("Survey point {id}"));
    ui.label(format!(
        "Position: x {:.2} m, y {:.2} m",
        m.position.x, m.position.y
    ));
    if m.timestamp_ms > 0 {
        let secs = m.timestamp_ms / 1000;
        ui.weak(format!("Recorded at Unix time {secs}"));
    }
    if let Some(err) = app
        .heat
        .comparison
        .as_ref()
        .and_then(|c| c.points.iter().find(|(pid, _)| *pid == id))
    {
        ui.label(format!(
            "Measured − predicted ({}): {:+.1} dB",
            app.heat.band, err.1
        ));
    }
    ui.label(format!("{} networks heard:", m.readings.len()));
    let mut readings = m.readings.clone();
    readings.sort_by(|a, b| b.rssi.0.total_cmp(&a.rssi.0));
    egui::ScrollArea::vertical()
        .id_salt("readings")
        .max_height(360.0)
        .show(ui, |ui| {
            egui::Grid::new("readings-grid")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    ui.strong("SSID");
                    ui.strong("dBm");
                    ui.strong("Ch");
                    ui.strong("Security");
                    ui.end_row();
                    for r in &readings {
                        let ssid = if r.ssid.is_empty() {
                            "(hidden)"
                        } else {
                            &r.ssid
                        };
                        ui.label(ssid).on_hover_text(&r.bssid);
                        ui.label(format!("{:.0}", r.rssi.0));
                        ui.label(r.channel().map_or("?".into(), |(_, n)| n.to_string()));
                        ui.label(r.security.label());
                        ui.end_row();
                    }
                });
        });
    if ui.button("Delete point").clicked() {
        doc.checkpoint();
        if let Some(s) = doc.project.surveys.get_mut(app.survey.active) {
            s.measurements.retain(|x| x.id != id);
        }
        app.selection = Selection::None;
    }
}
