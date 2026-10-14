//! Right panel (properties, guided steps) and editor windows.
//!
//! Editors work on a copy of the data; if the copy changed, the document
//! takes an undo checkpoint and the copy is written back.

use crate::app::{OndaluxApp, Selection, Tool};
use crate::catalog_ui;
use crate::i18n::t;
use crate::style;
use eframe::egui::{self, Color32, DragValue, RichText};
use ondalux_core::ap::{ApId, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth, PerBand};
use ondalux_core::material::MaterialLibrary;
use ondalux_core::metrics::Requirements;
use ondalux_core::project::Calibration;
use ondalux_core::propagation::{PathLossModel, PropagationSettings};
use ondalux_core::units::{Db, Dbm};

const OK: Color32 = Color32::from_rgb(34, 160, 90);
const TODO: Color32 = Color32::from_rgb(150, 150, 150);
const WARN: Color32 = Color32::from_rgb(220, 140, 30);

pub fn inspector(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    match app.selection {
        Selection::Ap(id) => ap_inspector(app, ui, id),
        Selection::Wall(i) => wall_inspector(app, ui, i),
        Selection::Measurement(id) => crate::survey_ui::measurement_inspector(app, ui, id),
        Selection::None => project_steps(app, ui),
    }
}

fn wall_inspector(app: &mut OndaluxApp, ui: &mut egui::Ui, i: usize) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let Some(orig) = doc.project.walls.get(i).cloned() else {
        app.selection = Selection::None;
        return;
    };
    ui.heading(t("Pared", "Wall"));
    let mut wall = orig.clone();
    egui::Grid::new("wall-grid").num_columns(2).show(ui, |ui| {
        ui.label(t("Material", "Material"));
        let name = doc
            .project
            .materials
            .get(wall.material)
            .map_or("—".to_owned(), |m| m.name.clone());
        egui::ComboBox::from_id_salt("wall-mat")
            .selected_text(name)
            .show_ui(ui, |ui| {
                for m in doc.project.materials.iter() {
                    ui.selectable_value(&mut wall.material, m.id, &m.name);
                }
            });
        ui.end_row();
        ui.label(t("Largo", "Length"));
        ui.label(format!("{:.2} m", wall.segment.length()));
        ui.end_row();
        if let Some(m) = doc.project.materials.get(wall.material) {
            ui.label(t("Atenuación", "Loss"));
            ui.label(format!(
                "{:.0} / {:.0} / {:.0} dB",
                m.attenuation.g2_4.0, m.attenuation.g5.0, m.attenuation.g6.0
            ))
            .on_hover_text(t(
                "Pérdida de señal al atravesar la pared, en 2.4 / 5 / 6 GHz.",
                "Signal loss when crossing the wall, at 2.4 / 5 / 6 GHz.",
            ));
            ui.end_row();
        }
    });
    if wall != orig {
        doc.checkpoint();
        doc.project.walls[i] = wall;
    }
    ui.add_space(10.0);
    if ui
        .button(
            RichText::new(t("Eliminar pared", "Delete wall")).color(Color32::from_rgb(200, 60, 60)),
        )
        .clicked()
    {
        app.delete_selection();
    }
}

/// One line of the guided steps: status mark, title, detail and action.
fn step(
    ui: &mut egui::Ui,
    mark: (&str, Color32),
    title: &str,
    detail: &str,
    action: Option<&str>,
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(6, 5))
        .corner_radius(6)
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(mark.0).strong().size(16.0).color(mark.1));
                // Button first (right-aligned) so the text wraps in the space left.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(a) = action {
                        clicked = ui.button(a).clicked();
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.label(RichText::new(title).strong());
                        if !detail.is_empty() {
                            ui.label(RichText::new(detail).small().weak());
                        }
                    });
                });
            });
        });
    clicked
}

fn project_steps(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let mut name = doc.project.name.clone();
    ui.add(
        egui::TextEdit::singleline(&mut name)
            .font(egui::TextStyle::Heading)
            .frame(egui::Frame::NONE)
            .hint_text(t("Nombre del proyecto", "Project name")),
    );
    if name != doc.project.name {
        doc.checkpoint_coalesced("project-name");
        doc.project.name = name;
    }
    let p = &doc.project;
    let (w, h) = p.plan.size_m();
    let calibrated = p.plan.calibration.is_some();
    let walls = p.walls.len();
    let aps = p.aps.len();
    let coverage = app.heat.field.as_ref().and_then(|f| f.coverage);
    let req = p.requirements;

    style::section(ui, t("Pasos", "Steps"));
    let done = ("✔", OK);
    let todo = ("○", TODO);
    step(
        ui,
        done,
        t("Plano importado", "Floor plan imported"),
        &format!("{w:.1} × {h:.1} m"),
        None,
    );
    if step(
        ui,
        if calibrated { done } else { ("!", WARN) },
        t("Escala calibrada", "Scale calibrated"),
        if calibrated {
            t("Las medidas son reales.", "Distances are real.")
        } else {
            t(
                "Sin calibrar: las distancias son aproximadas.",
                "Not calibrated: distances are approximate.",
            )
        },
        Some(if calibrated {
            t("Recalibrar", "Redo")
        } else {
            t("Calibrar", "Calibrate")
        }),
    ) {
        app.set_tool(Tool::Calibrate);
    }
    let walls_detail = format!("{walls} {}", t("paredes", "walls"));
    if step(
        ui,
        if walls > 0 { done } else { todo },
        t("Paredes dibujadas", "Walls drawn"),
        &walls_detail,
        Some(t("Dibujar", "Draw")),
    ) {
        app.set_tool(Tool::Wall);
    }
    let aps_detail = format!("{aps} {}", t("puntos de acceso", "access points"));
    if step(
        ui,
        if aps > 0 { done } else { todo },
        t("Puntos de acceso colocados", "Access points placed"),
        &aps_detail,
        Some(t("Colocar", "Place")),
    ) {
        app.set_tool(Tool::Ap);
    }
    let (cov_mark, cov_detail) = match coverage {
        Some(c) if aps > 0 => (
            if c >= 0.95 { done } else { ("!", WARN) },
            format!(
                "{:.0} % {} {:.0} dBm / SNR {:.0} dB ({})",
                c * 100.0,
                t("cumple", "meets"),
                req.min_rssi.0,
                req.min_snr.0,
                app.heat.band.label()
            ),
        ),
        _ => (
            todo,
            t(
                "Coloca APs para ver la cobertura.",
                "Place APs to see coverage.",
            )
            .to_owned(),
        ),
    };
    if step(
        ui,
        cov_mark,
        t("Cobertura revisada", "Coverage checked"),
        &cov_detail,
        Some(t("Requisitos", "Requirements")),
    ) {
        app.show_settings = true;
    }
    if step(
        ui,
        todo,
        t("Reporte", "Report"),
        t(
            "PDF con mapas, zonas sin cobertura y recomendaciones.",
            "PDF with maps, coverage gaps and recommendations.",
        ),
        Some(t("Exportar", "Export")),
    ) {
        app.export.report_open = true;
    }

    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    if !doc.project.aps.is_empty() {
        style::section(ui, t("Puntos de acceso", "Access points"));
        let mut clicked = None;
        for ap in &doc.project.aps {
            let channels: Vec<String> = ap
                .enabled_radios()
                .map(|(_, r)| r.channel.number.to_string())
                .collect();
            let model = if ap.model.is_empty() {
                String::new()
            } else {
                format!(" · {}", ap.model)
            };
            if ui
                .selectable_label(
                    false,
                    format!("📡  {}{model}   ch {}", ap.name, channels.join("/")),
                )
                .clicked()
            {
                clicked = Some(ap.id);
            }
        }
        if let Some(id) = clicked {
            app.selection = Selection::Ap(id);
        }
    }
}

fn ap_inspector(app: &mut OndaluxApp, ui: &mut egui::Ui, id: ApId) {
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    let Some(index) = doc.project.aps.iter().position(|a| a.id == id) else {
        app.selection = Selection::None;
        return;
    };
    let orig = doc.project.aps[index].clone();
    ui.heading(t("Punto de acceso", "Access point"));

    // Catalog model: picking one fills the radios.
    let catalog = &app.catalog.catalog;
    let mut selected = catalog
        .find(&orig.vendor, &orig.model)
        .map(|m| (m.vendor.clone(), m.model.clone()));
    let mut ap = orig.clone();
    let mut delete = false;
    egui::Grid::new("ap-grid")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label(t("Nombre", "Name"));
            ui.text_edit_singleline(&mut ap.name);
            ui.end_row();
            ui.label(t("Modelo", "Model"));
            if catalog_ui::model_combo(
                ui,
                "ap-model",
                catalog,
                &mut selected,
                t("Personalizado", "Custom"),
            ) {
                if let Some(m) = selected.as_ref().and_then(|(v, m)| catalog.find(v, m)) {
                    m.apply_to(&mut ap, index);
                }
            }
            ui.end_row();
            ui.label(t("Altura", "Height"));
            ui.add(
                DragValue::new(&mut ap.height_m)
                    .range(0.0..=30.0)
                    .speed(0.05)
                    .suffix(" m"),
            )
            .on_hover_text(t(
                "Altura de montaje sobre el piso.",
                "Mounting height above the floor.",
            ));
            ui.end_row();
        });

    style::section(ui, t("Radios", "Radios"));
    for (i, radio) in ap.radios.iter_mut().enumerate() {
        ui.push_id(i, |ui| radio_card(ui, radio));
    }

    egui::CollapsingHeader::new(t("Más detalles", "More details"))
        .id_salt("ap-details")
        .show(ui, |ui| {
            egui::Grid::new("ap-adv").num_columns(2).show(ui, |ui| {
                ui.label(t("Posición", "Position"));
                ui.horizontal(|ui| {
                    ui.add(
                        DragValue::new(&mut ap.position.x)
                            .speed(0.05)
                            .prefix("x ")
                            .suffix(" m"),
                    );
                    ui.add(
                        DragValue::new(&mut ap.position.y)
                            .speed(0.05)
                            .prefix("y ")
                            .suffix(" m"),
                    );
                });
                ui.end_row();
                ui.label(t("Orientación", "Orientation"));
                ui.add(
                    DragValue::new(&mut ap.orientation_deg)
                        .range(0.0..=359.0)
                        .suffix("°"),
                )
                .on_hover_text(t(
                    "Aún no se usa: el motor modela antenas omnidireccionales.",
                    "Not used yet: the engine models omnidirectional antennas.",
                ));
                ui.end_row();
            });
            let mut remove = None;
            for (i, radio) in ap.radios.iter_mut().enumerate() {
                ui.push_id(("adv", i), |ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.strong(format!("{} {}", t("Radio", "Radio"), i + 1));
                        if ui.small_button(t("Quitar", "Remove")).clicked() {
                            remove = Some(i);
                        }
                    });
                    radio_advanced(ui, radio);
                });
            }
            if let Some(i) = remove {
                ap.radios.remove(i);
            }
            if ui.button(t("Agregar radio", "Add radio")).clicked() {
                ap.radios.push(Radio {
                    channel: Channel::new(Band::G5, 36, ChannelWidth::Mhz20)
                        .expect("valid channel"),
                    tx_power: Dbm(17.0),
                    antenna_gain: Db(5.0),
                    enabled: true,
                });
            }
            if let Some(m) = selected.as_ref().and_then(|(v, m)| catalog.find(v, m)) {
                ui.add_space(4.0);
                ui.hyperlink_to(t("Ficha técnica del modelo", "Model datasheet"), &m.source);
            }
        });

    ui.add_space(10.0);
    if ui
        .button(
            RichText::new(t("Eliminar punto de acceso", "Delete access point"))
                .color(Color32::from_rgb(200, 60, 60)),
        )
        .clicked()
    {
        delete = true;
    }

    if ap != orig {
        // A manual edit of the radios no longer matches the catalog model.
        if ap.radios != orig.radios
            && selected
                .as_ref()
                .is_some_and(|(v, m)| (v, m) == (&orig.vendor, &orig.model))
        {
            let still_model = catalog
                .find(&orig.vendor, &orig.model)
                .is_some_and(|m| m.radios.len() == ap.radios.len());
            if !still_model {
                ap.vendor.clear();
            }
        }
        doc.checkpoint_coalesced("ap-edit");
        doc.project.aps[index] = ap;
    }
    if delete {
        app.delete_selection();
    }
}

/// Everyday radio settings: on/off, channel, width and power.
fn radio_card(ui: &mut egui::Ui, radio: &mut Radio) {
    egui::Frame::group(ui.style())
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.checkbox(
                &mut radio.enabled,
                RichText::new(radio.channel.band.label()).strong(),
            );
            ui.add_enabled_ui(radio.enabled, |ui| {
                let band = radio.channel.band;
                let mut number = radio.channel.number;
                let mut width = radio.channel.width;
                egui::Grid::new("radio-grid").num_columns(2).show(ui, |ui| {
                    ui.label(t("Canal", "Channel"));
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("radio-channel")
                            .selected_text(number.to_string())
                            .width(70.0)
                            .height(300.0)
                            .show_ui(ui, |ui| {
                                for c in band.channels() {
                                    ui.selectable_value(&mut number, c, c.to_string());
                                }
                            });
                        egui::ComboBox::from_id_salt("radio-width")
                            .selected_text(format!("{} MHz", width.mhz()))
                            .width(80.0)
                            .show_ui(ui, |ui| {
                                for w in band.widths() {
                                    ui.selectable_value(&mut width, *w, format!("{} MHz", w.mhz()));
                                }
                            })
                            .response
                            .on_hover_text(t("Ancho de canal", "Channel width"));
                    });
                    ui.end_row();
                    ui.label(t("Potencia", "Power"));
                    ui.add(
                        DragValue::new(&mut radio.tx_power.0)
                            .range(-10.0..=36.0)
                            .speed(0.25)
                            .suffix(" dBm"),
                    )
                    .on_hover_text(t(
                        "Potencia de transmisión configurada en el AP.",
                        "Transmit power configured on the AP.",
                    ));
                    ui.end_row();
                });
                if let Ok(ch) = Channel::new(band, number, width) {
                    radio.channel = ch;
                }
            });
        });
}

/// Rarely changed radio settings.
fn radio_advanced(ui: &mut egui::Ui, radio: &mut Radio) {
    let mut band = radio.channel.band;
    egui::Grid::new("radio-adv").num_columns(2).show(ui, |ui| {
        ui.label(t("Banda", "Band"));
        egui::ComboBox::from_id_salt("radio-band")
            .selected_text(band.label())
            .show_ui(ui, |ui| {
                for b in Band::ALL {
                    ui.selectable_value(&mut band, b, b.label());
                }
            });
        ui.end_row();
        ui.label(t("Ganancia de antena", "Antenna gain"));
        ui.add(
            DragValue::new(&mut radio.antenna_gain.0)
                .range(-5.0..=25.0)
                .speed(0.1)
                .suffix(" dBi"),
        );
        ui.end_row();
        ui.label("EIRP");
        ui.label(format!("{:.1} dBm", radio.eirp().0))
            .on_hover_text(t("Potencia + ganancia de antena.", "Power + antenna gain."));
        ui.end_row();
    });
    if band != radio.channel.band {
        if let Ok(ch) = Channel::new(band, band.channels()[0], ChannelWidth::Mhz20) {
            radio.channel = ch;
        }
    }
}

pub fn calibration_dialog(app: &mut OndaluxApp, ctx: &egui::Context) {
    let Some((a, b, mut text)) = app.canvas.calib_pending.clone() else {
        return;
    };
    let mut done = None;
    egui::Window::new(t("Calibrar escala", "Calibrate scale"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(t(
                "¿Cuál es la distancia real entre los dos puntos?",
                "What is the real distance between the two points?",
            ));
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0));
                r.request_focus();
                ui.label(t("metros", "meters"));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
            });
            ui.weak(format!(
                "({:.0} px {})",
                a.distance(b),
                t("en el plano", "on the plan")
            ));
            ui.horizontal(|ui| {
                if ui.button(t("Aplicar", "Apply")).clicked() {
                    done = Some(true);
                }
                if ui.button(t("Cancelar", "Cancel")).clicked() {
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
                    app.info(format!(
                        "{} {:.1} px/m.",
                        t("Escala calibrada:", "Scale set:"),
                        c.px_per_m()
                    ));
                }
                None => {
                    app.error(t(
                        "Escribe una distancia positiva en metros (los puntos deben estar separados).",
                        "Enter a positive distance in meters (points must be apart).",
                    ));
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
    egui::Window::new(t("Materiales de pared", "Wall materials"))
        .open(&mut open)
        .default_width(540.0)
        .show(ctx, |ui| {
            ui.weak(t(
                "Pérdida de señal al atravesar una pared de cada material, en dB. Son valores típicos: ajústalos a tu edificio.",
                "Signal loss when crossing a wall of each material, in dB. These are typical values: adjust them to your building.",
            ));
            ui.add_space(4.0);
            let ids: Vec<_> = lib.iter().map(|m| m.id).collect();
            let mut remove = None;
            egui::Grid::new("materials").striped(true).num_columns(6).show(ui, |ui| {
                ui.strong(t("Nombre", "Name"));
                ui.strong(t("Color", "Color"));
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
                    let mut c = Color32::from_rgba_unmultiplied(m.color[0], m.color[1], m.color[2], m.color[3]);
                    if ui.color_edit_button_srgba(&mut c).changed() {
                        m.color = c.to_srgba_unmultiplied();
                    }
                    for band in Band::ALL {
                        let v = m.attenuation.get_mut(band);
                        ui.add(DragValue::new(&mut v.0).range(0.0..=80.0).speed(0.1).suffix(" dB"));
                    }
                    let del = ui
                        .add_enabled(!in_use, egui::Button::new(t("Eliminar", "Delete")))
                        .on_disabled_hover_text(t("Hay paredes con este material", "Walls use this material"));
                    if del.clicked() {
                        remove = Some(id);
                    }
                    ui.end_row();
                }
            });
            if let Some(id) = remove {
                lib.remove(id);
            }
            if ui.button(t("Agregar material", "Add material")).clicked() {
                lib.add(
                    t("Nuevo material", "New material"),
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
    egui::Window::new(t("Requisitos y predicción", "Requirements and prediction"))
        .open(&mut open)
        .default_width(400.0)
        .show(ctx, |ui| {
            ui.strong(t("¿Qué cobertura necesitas?", "What coverage do you need?"));
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(req.min_rssi == Dbm(-67.0) && req.min_snr == Db(25.0), t("Voz y video", "Voice and video"))
                    .on_hover_text("−67 dBm, SNR 25 dB")
                    .clicked()
                {
                    req.min_rssi = Dbm(-67.0);
                    req.min_snr = Db(25.0);
                }
                if ui
                    .selectable_label(req.min_rssi == Dbm(-72.0) && req.min_snr == Db(20.0), t("Datos", "Data"))
                    .on_hover_text("−72 dBm, SNR 20 dB")
                    .clicked()
                {
                    req.min_rssi = Dbm(-72.0);
                    req.min_snr = Db(20.0);
                }
                if ui
                    .selectable_label(req.min_rssi == Dbm(-77.0) && req.min_snr == Db(15.0), t("Cobertura básica", "Basic coverage"))
                    .on_hover_text("−77 dBm, SNR 15 dB")
                    .clicked()
                {
                    req.min_rssi = Dbm(-77.0);
                    req.min_snr = Db(15.0);
                }
            });
            egui::Grid::new("req").num_columns(2).show(ui, |ui| {
                ui.label(t("Señal mínima", "Minimum signal"));
                ui.add(DragValue::new(&mut req.min_rssi.0).range(-100.0..=-30.0).speed(0.5).suffix(" dBm"));
                ui.end_row();
                ui.label(t("SNR mínima", "Minimum SNR"));
                ui.add(DragValue::new(&mut req.min_snr.0).range(0.0..=60.0).speed(0.5).suffix(" dB"));
                ui.end_row();
                ui.label(t("Altura del dispositivo", "Device height"));
                ui.add(DragValue::new(&mut settings.receiver_height_m).range(0.0..=10.0).speed(0.05).suffix(" m"));
                ui.end_row();
            });

            egui::CollapsingHeader::new(t("Avanzado", "Advanced"))
                .id_salt("settings-adv")
                .show(ui, |ui| {
                    ui.strong(t("Modelo de propagación", "Propagation model"));
                    let is_fs = matches!(settings.model, PathLossModel::FreeSpace);
                    ui.horizontal(|ui| {
                        if ui.radio(is_fs, t("Espacio libre", "Free space")).clicked() && !is_fs {
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
                                ui.label(format!("{} n ({band})", t("Exponente", "Exponent")));
                                ui.add(DragValue::new(exponent.get_mut(band)).range(1.5..=6.0).speed(0.01));
                                ui.end_row();
                            }
                            ui.label(t("Distancia de referencia", "Reference distance"));
                            ui.add(DragValue::new(ref_distance_m).range(0.1..=10.0).speed(0.01).suffix(" m"));
                            ui.end_row();
                        });
                        ui.weak(t(
                            "n = 2 es espacio libre. Como las paredes se modelan aparte, lo típico es 2.0–2.5.",
                            "n = 2 is free space. Walls are modelled separately, so 2.0–2.5 is typical.",
                        ));
                    }
                    egui::Grid::new("rx").num_columns(2).show(ui, |ui| {
                        ui.label(t("Ganancia del dispositivo", "Device antenna gain"));
                        ui.add(DragValue::new(&mut settings.receiver_gain.0).range(-10.0..=10.0).speed(0.1).suffix(" dBi"));
                        ui.end_row();
                        ui.label(t("Figura de ruido", "Noise figure"));
                        ui.add(DragValue::new(&mut settings.noise_figure.0).range(0.0..=20.0).speed(0.1).suffix(" dB"));
                        ui.end_row();
                        ui.label(t("Umbral de solapamiento (CCA)", "Overlap threshold (CCA)"));
                        ui.add(DragValue::new(&mut req.cca_threshold.0).range(-100.0..=-40.0).speed(0.5).suffix(" dBm"));
                        ui.end_row();
                        ui.label(t("Resolución del mapa", "Map resolution"));
                        ui.add(DragValue::new(&mut cell).range(0.05..=2.0).speed(0.01).suffix(" m"))
                            .on_hover_text(t("Más pequeño = más nítido pero más lento.", "Smaller = sharper but slower."));
                        ui.end_row();
                    });
                });
        });
    if (settings.clone(), req, cell) != orig {
        doc.checkpoint_coalesced("settings");
        doc.project.settings = settings;
        doc.project.requirements = req;
        doc.project.grid_cell_m = cell;
    }
    app.show_settings = open;
}
