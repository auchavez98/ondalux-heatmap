//! Right panel (properties, guided steps) and editor windows.
//!
//! Editors work on a copy of the data; if the copy changed, the document
//! takes an undo checkpoint and the copy is written back.

use crate::app::{OndaluxApp, Selection, Tool};
use crate::catalog_ui;
use crate::i18n::{self, t};
use crate::style::{self, icon};
use crate::tr;
use eframe::egui::{self, Color32, DragValue, RichText};
use ondalux_core::ap::{AntennaPattern, ApId, Mount, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth, PerBand};
use ondalux_core::material::MaterialLibrary;
use ondalux_core::metrics::Requirements;
use ondalux_core::project::Calibration;
use ondalux_core::propagation::{PathLossModel, PropagationSettings};
use ondalux_core::units::{Db, Dbm};

/// Inspector heading: an icon in an accent tile and the title.
pub fn inspector_title(ui: &mut egui::Ui, glyph: &str, title: &str) {
    let p = style::palette(ui);
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(p.accent_soft)
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(7, 4))
            .show(ui, |ui| {
                ui.label(RichText::new(glyph).size(17.0).color(p.accent));
            });
        ui.label(style::bold(title).size(18.0));
    });
    ui.add_space(4.0);
}

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
    inspector_title(ui, icon::WALL, t("Pared", "Wall"));
    let mut wall = orig.clone();
    egui::Grid::new("wall-grid").num_columns(2).show(ui, |ui| {
        ui.label(t("Material", "Material"));
        material_combo(
            ui,
            "wall-mat",
            &doc.project.materials,
            &mut wall.material,
            170.0,
        );
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
    if style::danger_button(
        ui,
        &format!("{}  {}", icon::TRASH, t("Eliminar pared", "Delete wall")),
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
    let p = style::palette(ui);
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(10, 8))
        .corner_radius(10)
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(mark.0).size(20.0).color(mark.1));
                // Button first (right-aligned) so the text wraps in the space left.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(a) = action {
                        clicked = ui.button(a).clicked();
                    }
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.label(style::bold(title));
                        if !detail.is_empty() {
                            ui.label(RichText::new(detail).size(12.0).color(p.muted));
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

    let pal = style::palette(ui);
    style::section(ui, t("Pasos", "Steps"));
    ui.spacing_mut().item_spacing.y = 6.0;
    let done = (icon::CHECK_CIRCLE, pal.success);
    let todo = (icon::CIRCLE_DASHED, pal.muted);
    let warn = (icon::WARNING_CIRCLE, pal.warning);
    step(
        ui,
        done,
        t("Plano importado", "Floor plan imported"),
        &format!("{w:.1} × {h:.1} m"),
        None,
    );
    if step(
        ui,
        if calibrated { done } else { warn },
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
            if c >= 0.95 { done } else { warn },
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
                    format!(
                        "{}  {}{model}   ch {}",
                        icon::BROADCAST,
                        ap.name,
                        channels.join("/")
                    ),
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

/// Round dial showing where a wall AP points on the plan; click or drag to
/// aim it (15° steps, the number field next to it allows any angle).
fn direction_dial(ui: &mut egui::Ui, deg: &mut f32) {
    let size = 58.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
    if response.clicked() || response.dragged() {
        if let Some(p) = response.interact_pointer_pos() {
            let v = p - rect.center();
            if v.length() > 4.0 {
                let a = (v.y.atan2(v.x).to_degrees() / 15.0).round() * 15.0;
                *deg = a.rem_euclid(360.0);
            }
        }
    }
    let painter = ui.painter_at(rect.expand(1.0));
    let c = rect.center();
    let r = size / 2.0 - 2.0;
    let v = ui.visuals();
    painter.circle(
        c,
        r,
        v.extreme_bg_color,
        egui::Stroke::new(1.0, v.widgets.inactive.bg_stroke.color.gamma_multiply(2.0)),
    );
    for i in 0..8 {
        let a = (i as f32 * 45.0).to_radians();
        let d = egui::vec2(a.cos(), a.sin());
        let inner = if i % 2 == 0 { r - 6.0 } else { r - 3.0 };
        painter.line_segment(
            [c + d * inner, c + d * r],
            egui::Stroke::new(1.0, v.weak_text_color()),
        );
    }
    let a = deg.to_radians();
    let d = egui::vec2(a.cos(), a.sin());
    let tip = c + d * (r - 5.0);
    painter.line_segment([c, tip], egui::Stroke::new(3.0, style::ACCENT));
    painter.circle_filled(tip, 4.5, style::ACCENT);
    painter.circle_filled(c, 3.0, v.text_color());
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(t(
            "Haz clic o arrastra para elegir hacia dónde apunta. También puedes girarlo en el plano con el punto naranja.",
            "Click or drag to aim it. You can also rotate it on the plan with the orange handle.",
        ));
}

/// Plain-language direction on the plan (screen up = "arriba").
fn direction_name(deg: f32) -> &'static str {
    const NAMES: [(&str, &str); 8] = [
        ("Derecha", "Right"),
        ("Abajo a la derecha", "Down-right"),
        ("Abajo", "Down"),
        ("Abajo a la izquierda", "Down-left"),
        ("Izquierda", "Left"),
        ("Arriba a la izquierda", "Up-left"),
        ("Arriba", "Up"),
        ("Arriba a la derecha", "Up-right"),
    ];
    let i = ((deg.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8;
    t(NAMES[i].0, NAMES[i].1)
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
    inspector_title(ui, icon::BROADCAST, t("Punto de acceso", "Access point"));

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
            ui.label(t("Montaje", "Mounting"));
            ui.horizontal(|ui| {
                for (m, label) in [
                    (Mount::Ceiling, t("Techo", "Ceiling")),
                    (Mount::Wall, t("Pared", "Wall")),
                ] {
                    if ui
                        .add(egui::Button::new(label).selected(ap.mount == m))
                        .clicked()
                        && ap.mount != m
                    {
                        ap.set_mount(m);
                    }
                }
            });
            ui.end_row();
            if ap.mount == Mount::Wall {
                ui.label(t("Apunta hacia", "Points to"));
                ui.horizontal(|ui| {
                    direction_dial(ui, &mut ap.orientation_deg);
                    ui.vertical(|ui| {
                        ui.label(direction_name(ap.orientation_deg));
                        ui.add(
                            DragValue::new(&mut ap.orientation_deg)
                                .range(0.0..=359.0)
                                .speed(1.0)
                                .suffix("°"),
                        );
                    });
                });
                ui.end_row();
                ui.label(t("Antena", "Antenna"));
                let presets = [
                    (AntennaPattern::WALL, t("Pared (180°)", "Wall (180°)")),
                    (
                        AntennaPattern::DIRECTIONAL,
                        t("Direccional (65°)", "Directional (65°)"),
                    ),
                ];
                let current = presets
                    .iter()
                    .find(|(p, _)| *p == ap.pattern)
                    .map_or(t("Personalizada", "Custom"), |(_, l)| *l);
                egui::ComboBox::from_id_salt("ap-antenna")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (p, l) in presets {
                            ui.selectable_value(&mut ap.pattern, p, l);
                        }
                    });
                ui.end_row();
            }
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
                if ap.mount == Mount::Wall {
                    ui.label(t("Apertura del haz", "Beamwidth"));
                    ui.add(
                        DragValue::new(&mut ap.pattern.beamwidth_deg)
                            .range(20.0..=360.0)
                            .speed(1.0)
                            .suffix("°"),
                    )
                    .on_hover_text(t(
                        "Ángulo en el que la señal cae menos de 3 dB.",
                        "Angle within which the signal drops less than 3 dB.",
                    ));
                    ui.end_row();
                    ui.label(t("Pérdida trasera", "Front-to-back"));
                    ui.add(
                        DragValue::new(&mut ap.pattern.front_to_back_db)
                            .range(0.0..=40.0)
                            .speed(0.5)
                            .suffix(" dB"),
                    )
                    .on_hover_text(t(
                        "Cuánto menos radia hacia atrás (hacia la pared).",
                        "How much less it radiates backwards (into the wall).",
                    ));
                    ui.end_row();
                }
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
    if style::danger_button(
        ui,
        &format!(
            "{}  {}",
            icon::TRASH,
            t("Eliminar punto de acceso", "Delete access point")
        ),
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
    let p = style::palette(ui);
    egui::Frame::new()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(10)
        .inner_margin(10.0)
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
        .default_width(600.0)
        .min_width(600.0)
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
                    // Built-in names are shown translated; typing stores the new name.
                    let mut name = i18n::material_name(&m.name).to_owned();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut name)
                                .id_salt(("material-name", id.0))
                                .desired_width(150.0)
                                .min_size(egui::vec2(170.0, 0.0)),
                        )
                        .changed()
                    {
                        m.name = name;
                    }
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
            let missing: Vec<_> = MaterialLibrary::default()
                .iter()
                .filter(|d| !lib.iter().any(|m| m.name == d.name))
                .cloned()
                .collect();
            ui.horizontal(|ui| {
                if ui.button(t("Agregar material", "Add material")).clicked() {
                    lib.add(
                        t("Nuevo material", "New material"),
                        [220, 120, 220, 255],
                        PerBand::new(Db(5.0), Db(7.0), Db(8.0)),
                    );
                }
                if !missing.is_empty()
                    && ui
                        .button(tr!(
                            "Agregar {} materiales comunes que faltan",
                            "Add {} missing common materials",
                            missing.len()
                        ))
                        .on_hover_text(t(
                            "Puertas, block, vidrio Low-E, elevador, mamparas…",
                            "Doors, concrete block, Low-E glass, elevator, cubicles…",
                        ))
                        .clicked()
                {
                    for m in missing {
                        lib.add(m.name, m.color, m.attenuation);
                    }
                }
            });
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

/// Material picker: color swatch, translated name and the loss per band.
pub fn material_combo(
    ui: &mut egui::Ui,
    id_salt: &str,
    lib: &MaterialLibrary,
    selected: &mut ondalux_core::material::MaterialId,
    width: f32,
) {
    fn swatch(ui: &mut egui::Ui, color: [u8; 4]) {
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
        ui.painter().rect_filled(r, 3.0, c);
    }
    let current = lib.get(*selected);
    egui::ComboBox::from_id_salt(id_salt)
        .width(width)
        .selected_text(current.map_or("—", |m| i18n::material_name(&m.name)))
        .height(420.0)
        .show_ui(ui, |ui| {
            for m in lib.iter() {
                ui.horizontal(|ui| {
                    swatch(ui, m.color);
                    let a = m.attenuation;
                    ui.selectable_value(selected, m.id, i18n::material_name(&m.name))
                        .on_hover_text(format!(
                            "{}: {:.0} / {:.0} / {:.0} dB (2.4 / 5 / 6 GHz)",
                            t("Pérdida", "Loss"),
                            a.g2_4.0,
                            a.g5.0,
                            a.g6.0
                        ));
                    ui.weak(format!("{:.0} dB", a.g5.0));
                });
            }
        });
}
