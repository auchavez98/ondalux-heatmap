//! Side panel inspector and editor windows.
//!
//! Editors work on a copy of the data; if the copy changed, the document
//! takes an undo checkpoint and the copy is written back.

use crate::app::{OndaluxApp, Selection, Tool};
use eframe::egui::{self, DragValue};
use ondalux_core::ap::{AccessPoint, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth, PerBand};
use ondalux_core::material::MaterialLibrary;
use ondalux_core::metrics::Requirements;
use ondalux_core::project::Calibration;
use ondalux_core::propagation::{PathLossModel, PropagationSettings};
use ondalux_core::units::{Db, Dbm};

pub fn inspector(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    match app.selection {
        Selection::Ap(id) => {
            let Some(orig) = doc.project.ap(id).cloned() else {
                app.selection = Selection::None;
                return;
            };
            ui.heading("Access point");
            let mut ap = orig.clone();
            let delete = ap_editor(ui, &mut ap);
            if ap != orig {
                doc.checkpoint_coalesced("ap-edit");
                if let Some(slot) = doc.project.ap_mut(id) {
                    *slot = ap;
                }
            }
            if delete {
                app.delete_selection();
            }
        }
        Selection::Wall(i) => {
            let Some(orig) = doc.project.walls.get(i).cloned() else {
                app.selection = Selection::None;
                return;
            };
            ui.heading("Wall");
            let mut wall = orig.clone();
            egui::Grid::new("wall-grid").num_columns(2).show(ui, |ui| {
                ui.label("Material");
                let name = doc
                    .project
                    .materials
                    .get(wall.material)
                    .map_or("(missing)".to_owned(), |m| m.name.clone());
                egui::ComboBox::from_id_salt("wall-mat")
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for m in doc.project.materials.iter() {
                            ui.selectable_value(&mut wall.material, m.id, &m.name);
                        }
                    });
                ui.end_row();
                ui.label("Length");
                ui.label(format!("{:.2} m", wall.segment.length()));
                ui.end_row();
                if let Some(m) = doc.project.materials.get(wall.material) {
                    ui.label("Loss");
                    ui.label(format!(
                        "{:.1} / {:.1} / {:.1} dB",
                        m.attenuation.g2_4.0, m.attenuation.g5.0, m.attenuation.g6.0
                    ));
                    ui.end_row();
                }
            });
            ui.weak("Loss per crossing at 2.4 / 5 / 6 GHz.");
            if wall != orig {
                doc.checkpoint();
                doc.project.walls[i] = wall;
            }
            ui.add_space(8.0);
            if ui.button("Delete wall").clicked() {
                app.delete_selection();
            }
        }
        Selection::None => project_summary(app, ui),
    }
}

fn project_summary(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    ui.heading("Project");
    let mut name = doc.project.name.clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut name);
    });
    if name != doc.project.name {
        doc.checkpoint_coalesced("project-name");
        doc.project.name = name;
    }
    let (w, h) = doc.project.plan.size_m();
    ui.label(format!(
        "Plan: {} × {} px  ({:.1} × {:.1} m)",
        doc.project.plan.width_px, doc.project.plan.height_px, w, h
    ));
    match doc.project.plan.calibration {
        Some(c) => {
            ui.label(format!("Scale: {:.1} px/m", c.px_per_m()));
        }
        None => {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Scale not calibrated (assuming 20 px/m).",
            );
            if ui.button("Calibrate now").clicked() {
                app.tool = Tool::Calibrate;
            }
        }
    }
    let doc = app.doc.as_ref().expect("checked above");
    ui.label(format!(
        "{} walls · {} access points",
        doc.project.walls.len(),
        doc.project.aps.len()
    ));
    ui.separator();
    ui.strong("Access points");
    if doc.project.aps.is_empty() {
        ui.weak("None yet. Use the AP tool (A) to place one.");
    }
    let mut clicked = None;
    for ap in &doc.project.aps {
        let radios: Vec<String> = ap
            .enabled_radios()
            .map(|(_, r)| format!("{}", r.channel.number))
            .collect();
        if ui
            .selectable_label(false, format!("{}   ch {}", ap.name, radios.join(" / ")))
            .clicked()
        {
            clicked = Some(ap.id);
        }
    }
    if let Some(id) = clicked {
        app.selection = Selection::Ap(id);
    }
}

/// Returns true if the user asked to delete the AP.
fn ap_editor(ui: &mut egui::Ui, ap: &mut AccessPoint) -> bool {
    egui::Grid::new("ap-grid").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut ap.name);
        ui.end_row();
        ui.label("Model");
        ui.text_edit_singleline(&mut ap.model);
        ui.end_row();
        ui.label("Position");
        ui.horizontal(|ui| {
            ui.add(
                DragValue::new(&mut ap.position.x)
                    .speed(0.05)
                    .suffix(" m")
                    .prefix("x "),
            );
            ui.add(
                DragValue::new(&mut ap.position.y)
                    .speed(0.05)
                    .suffix(" m")
                    .prefix("y "),
            );
        });
        ui.end_row();
        ui.label("Height");
        ui.add(
            DragValue::new(&mut ap.height_m)
                .range(0.0..=30.0)
                .speed(0.05)
                .suffix(" m"),
        );
        ui.end_row();
        ui.label("Orientation");
        ui.horizontal(|ui| {
            ui.add(
                DragValue::new(&mut ap.orientation_deg)
                    .range(0.0..=359.0)
                    .suffix("°"),
            );
            ui.weak("(not used yet: omni antennas only)");
        });
        ui.end_row();
    });

    ui.separator();
    ui.strong("Radios");
    let mut remove = None;
    for (i, radio) in ap.radios.iter_mut().enumerate() {
        ui.push_id(i, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut radio.enabled, format!("Radio {}", i + 1));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
                radio_editor(ui, radio);
            });
        });
    }
    if let Some(i) = remove {
        ap.radios.remove(i);
    }
    if ui.button("Add radio").clicked() {
        ap.radios.push(Radio {
            channel: Channel::new(Band::G5, 36, ChannelWidth::Mhz20).expect("valid channel"),
            tx_power: Dbm(17.0),
            antenna_gain: Db(5.0),
            enabled: true,
        });
    }
    ui.add_space(8.0);
    ui.button("Delete access point").clicked()
}

fn radio_editor(ui: &mut egui::Ui, radio: &mut Radio) {
    let mut band = radio.channel.band;
    let mut number = radio.channel.number;
    let mut width = radio.channel.width;
    egui::Grid::new("radio-grid").num_columns(2).show(ui, |ui| {
        ui.label("Band");
        egui::ComboBox::from_id_salt("radio-band")
            .selected_text(band.label())
            .show_ui(ui, |ui| {
                for b in Band::ALL {
                    ui.selectable_value(&mut band, b, b.label());
                }
            });
        ui.end_row();
        if band != radio.channel.band {
            number = band.channels()[0];
            width = ChannelWidth::Mhz20;
        }
        ui.label("Channel");
        egui::ComboBox::from_id_salt("radio-channel")
            .selected_text(number.to_string())
            .height(300.0)
            .show_ui(ui, |ui| {
                for c in band.channels() {
                    ui.selectable_value(&mut number, c, c.to_string());
                }
            });
        ui.end_row();
        ui.label("Width");
        egui::ComboBox::from_id_salt("radio-width")
            .selected_text(format!("{} MHz", width.mhz()))
            .show_ui(ui, |ui| {
                for w in band.widths() {
                    ui.selectable_value(&mut width, *w, format!("{} MHz", w.mhz()));
                }
            });
        ui.end_row();
        ui.label("Tx power");
        ui.add(
            DragValue::new(&mut radio.tx_power.0)
                .range(-10.0..=36.0)
                .speed(0.25)
                .suffix(" dBm"),
        );
        ui.end_row();
        ui.label("Antenna gain");
        ui.add(
            DragValue::new(&mut radio.antenna_gain.0)
                .range(-5.0..=25.0)
                .speed(0.1)
                .suffix(" dBi"),
        );
        ui.end_row();
        ui.label("EIRP");
        ui.label(format!("{:.1} dBm", radio.eirp().0));
        ui.end_row();
    });
    if let Ok(ch) = Channel::new(band, number, width) {
        radio.channel = ch;
    }
    let r = radio.channel.range();
    ui.weak(format!("Occupies {:.0}–{:.0} MHz", r.lo_mhz, r.hi_mhz));
}

pub fn calibration_dialog(app: &mut OndaluxApp, ctx: &egui::Context) {
    let Some((a, b, mut text)) = app.canvas.calib_pending.clone() else {
        return;
    };
    let mut done = None;
    egui::Window::new("Calibrate scale")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(format!("The two points are {:.0} px apart.", a.distance(b)));
            ui.horizontal(|ui| {
                ui.label("Real distance:");
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0));
                r.request_focus();
                ui.label("m");
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
            });
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
    match done {
        Some(true) => {
            let parsed = text.trim().replace(',', ".").parse::<f32>().ok();
            match parsed.and_then(|d| Calibration::new(a, b, d)) {
                Some(c) => {
                    if let Some(doc) = app.doc.as_mut() {
                        doc.checkpoint();
                        doc.project.set_calibration(Some(c));
                    }
                    app.canvas.calib_pending = None;
                    app.tool = Tool::Select;
                    app.info(format!("Scale set to {:.1} px/m.", c.px_per_m()));
                }
                None => {
                    app.error("Enter a positive distance in meters (points must be apart).");
                    app.canvas.calib_pending = Some((a, b, text));
                }
            }
        }
        Some(false) => app.canvas.calib_pending = None,
        None => app.canvas.calib_pending = Some((a, b, text)),
    }
}

pub fn materials_window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.show_materials {
        return;
    }
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let orig: MaterialLibrary = doc.project.materials.clone();
    let mut lib = orig.clone();
    let mut open = true;
    egui::Window::new("Materials")
        .open(&mut open)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.weak("Loss per wall crossing, in dB. Values are typical figures; adjust them to your building.");
            ui.add_space(4.0);
            let ids: Vec<_> = lib.iter().map(|m| m.id).collect();
            let mut remove = None;
            egui::Grid::new("materials").striped(true).num_columns(6).show(ui, |ui| {
                ui.strong("Name");
                ui.strong("Color");
                ui.strong("2.4 GHz");
                ui.strong("5 GHz");
                ui.strong("6 GHz");
                ui.label("");
                ui.end_row();
                for id in ids {
                    let in_use = doc.project.material_in_use(id);
                    let Some(m) = lib.get_mut(id) else { continue };
                    ui.add(
                        egui::TextEdit::singleline(&mut m.name)
                            .id_salt(("material-name", id.0))
                            .desired_width(120.0),
                    );
                    let mut c = egui::Color32::from_rgba_unmultiplied(m.color[0], m.color[1], m.color[2], m.color[3]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        m.color = c.to_srgba_unmultiplied();
                    }
                    for band in Band::ALL {
                        let v = m.attenuation.get_mut(band);
                        ui.add(DragValue::new(&mut v.0).range(0.0..=80.0).speed(0.1).suffix(" dB"));
                    }
                    let del = ui
                        .add_enabled(!in_use, egui::Button::new("Delete"))
                        .on_disabled_hover_text("Used by walls");
                    if del.clicked() {
                        remove = Some(id);
                    }
                    ui.end_row();
                }
            });
            if let Some(id) = remove {
                lib.remove(id);
            }
            if ui.button("Add material").clicked() {
                lib.add(
                    "New material",
                    [220, 120, 220, 255],
                    PerBand::new(Db(5.0), Db(7.0), Db(8.0)),
                );
            }
        });
    if lib != orig {
        doc.checkpoint_coalesced("materials");
        doc.project.materials = lib;
        if doc.project.materials.get(app.wall_material).is_none() {
            if let Some(m) = doc.project.materials.iter().next() {
                app.wall_material = m.id;
            }
        }
    }
    app.show_materials = open;
}

pub fn settings_window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.show_settings {
        return;
    }
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let orig = (
        doc.project.settings.clone(),
        doc.project.requirements,
        doc.project.grid_cell_m,
    );
    let (mut settings, mut req, mut cell): (PropagationSettings, Requirements, f32) = orig.clone();
    let mut open = true;
    egui::Window::new("Prediction settings")
        .open(&mut open)
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.strong("Path loss model");
            let is_fs = matches!(settings.model, PathLossModel::FreeSpace);
            ui.horizontal(|ui| {
                if ui.radio(is_fs, "Free space").clicked() && !is_fs {
                    settings.model = PathLossModel::FreeSpace;
                }
                if ui.radio(!is_fs, "Log-distance").clicked() && is_fs {
                    settings.model = PathLossModel::default();
                }
            });
            if let PathLossModel::LogDistance {
                exponent,
                ref_distance_m,
            } = &mut settings.model
            {
                egui::Grid::new("ld").num_columns(2).show(ui, |ui| {
                    for band in Band::ALL {
                        ui.label(format!("Exponent n ({band})"));
                        ui.add(DragValue::new(exponent.get_mut(band)).range(1.5..=6.0).speed(0.01));
                        ui.end_row();
                    }
                    ui.label("Reference distance");
                    ui.add(DragValue::new(ref_distance_m).range(0.1..=10.0).speed(0.01).suffix(" m"));
                    ui.end_row();
                });
                ui.weak("n = 2 is free space. Walls are modelled separately, so values of 2.0–2.5 are typical.");
            }
            ui.separator();
            ui.strong("Client device");
            egui::Grid::new("rx").num_columns(2).show(ui, |ui| {
                ui.label("Height");
                ui.add(DragValue::new(&mut settings.receiver_height_m).range(0.0..=10.0).speed(0.05).suffix(" m"));
                ui.end_row();
                ui.label("Antenna gain");
                ui.add(DragValue::new(&mut settings.receiver_gain.0).range(-10.0..=10.0).speed(0.1).suffix(" dBi"));
                ui.end_row();
                ui.label("Noise figure");
                ui.add(DragValue::new(&mut settings.noise_figure.0).range(0.0..=20.0).speed(0.1).suffix(" dB"));
                ui.end_row();
            });
            ui.separator();
            ui.strong("Requirements");
            egui::Grid::new("req").num_columns(2).show(ui, |ui| {
                ui.label("Minimum signal");
                ui.add(DragValue::new(&mut req.min_rssi.0).range(-100.0..=-30.0).speed(0.5).suffix(" dBm"));
                ui.end_row();
                ui.label("Minimum SNR");
                ui.add(DragValue::new(&mut req.min_snr.0).range(0.0..=60.0).speed(0.5).suffix(" dB"));
                ui.end_row();
                ui.label("Channel overlap threshold");
                ui.add(DragValue::new(&mut req.cca_threshold.0).range(-100.0..=-40.0).speed(0.5).suffix(" dBm"));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                ui.label("Presets:");
                if ui.small_button("Voice / video (−67 dBm, 25 dB)").clicked() {
                    req.min_rssi = Dbm(-67.0);
                    req.min_snr = Db(25.0);
                }
                if ui.small_button("Data (−72 dBm, 20 dB)").clicked() {
                    req.min_rssi = Dbm(-72.0);
                    req.min_snr = Db(20.0);
                }
            });
            ui.separator();
            ui.strong("Heat map resolution");
            ui.add(DragValue::new(&mut cell).range(0.05..=2.0).speed(0.01).suffix(" m per sample"));
            ui.weak("Smaller is sharper but slower.");
        });
    if (settings.clone(), req, cell) != orig {
        doc.checkpoint_coalesced("settings");
        doc.project.settings = settings;
        doc.project.requirements = req;
        doc.project.grid_cell_m = cell;
    }
    app.show_settings = open;
}
