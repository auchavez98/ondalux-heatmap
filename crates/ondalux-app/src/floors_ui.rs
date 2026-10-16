//! Floors of the building: the floor selector over the plan, adding and
//! removing floors, and the floor settings window.

use crate::app::{OndaluxApp, Selection};
use crate::i18n::t;
use crate::style::{self, icon};
use crate::tr;
use eframe::egui;
use ondalux_core::project::{Floor, FloorInfo, Project};
use ondalux_io::{import_plan, FloorAssets};
use std::sync::Arc;

#[derive(Default)]
pub struct FloorState {
    pub settings_open: bool,
    /// Which floor's plan the textures show: (floor index, plan files).
    shown: Option<(usize, usize)>,
}

/// Display name of floor `i`: its name, or "Floor n".
pub fn label(project: &Project, i: usize) -> String {
    let name = project.floor_info(i).name.trim();
    if name.is_empty() {
        tr!("Piso {}", "Floor {}", i + 1)
    } else {
        name.to_owned()
    }
}

/// Keeps the on-screen plan in line with the active floor (after switching
/// floors, undo, adding or removing floors) and resets per-floor state.
pub fn sync(app: &mut OndaluxApp, ctx: &egui::Context) {
    let Some(doc) = &app.doc else {
        app.floors.shown = None;
        return;
    };
    let key = (doc.project.floor_index, Arc::as_ptr(doc.assets()) as usize);
    if app.floors.shown == Some(key) {
        return;
    }
    let first = app.floors.shown.is_none();
    let old_size = app.plan_texture.as_ref().map(|t| t.size());
    app.floors.shown = Some(key);
    app.load_plan_texture(ctx);
    if first {
        return;
    }
    let new_size = app.plan_texture.as_ref().map(|t| t.size());
    if new_size != old_size {
        app.fit_pending = true;
    }
    app.selection = Selection::None;
    app.walls = Default::default();
    app.canvas = Default::default();
    app.heat.texture = None;
    app.heat.field = None;
    app.heat.comparison = None;
    app.survey.active = 0;
}

/// The floors of the building, in the tools panel: one row per floor (top
/// floor first), add a floor and its settings.
pub fn panel(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let Some(doc) = &app.doc else {
        return;
    };
    let project = &doc.project;
    let n = project.floor_count();
    let current = project.floor_index;
    let labels: Vec<String> = (0..n).map(|i| label(project, i)).collect();
    let mut pick = None;
    let mut add: Option<(bool, bool)> = None;
    ui.add_space(6.0);
    style::section(ui, t("Edificio", "Building"));
    ui.spacing_mut().item_spacing.y = 4.0;
    // Top floor first, like a building.
    for i in (0..n).rev() {
        let r = style::tool_button(ui, icon::STACK, &labels[i], "", i == current).on_hover_text(t(
            "Piso que estás editando (Re Pág / Av Pág)",
            "Floor being edited (Page Up / Page Down)",
        ));
        if r.clicked() {
            pick = Some(i);
        }
    }
    ui.horizontal(|ui| {
        ui.menu_button(
            format!("{}  {}", icon::PLUS, t("Agregar piso", "Add floor")),
            |ui| {
                for (above, same, es, en) in [
                    (true, true, "Arriba, mismo plano", "Above, same plan"),
                    (true, false, "Arriba, otro plano…", "Above, another plan…"),
                    (false, true, "Abajo, mismo plano", "Below, same plan"),
                    (false, false, "Abajo, otro plano…", "Below, another plan…"),
                ] {
                    if ui.button(t(es, en)).clicked() {
                        add = Some((above, same));
                    }
                }
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if style::icon_button(ui, icon::GEAR_SIX, t("Configurar piso", "Floor settings"))
                .clicked()
            {
                app.floors.settings_open = !app.floors.settings_open;
            }
        });
    });
    if let Some(i) = pick {
        switch(app, i);
    }
    if let Some((above, same_plan)) = add {
        add_floor(app, above, same_plan);
    }
}

/// Makes floor `i` active. Not an undo step.
pub fn switch(app: &mut OndaluxApp, i: usize) {
    if let Some(doc) = &mut app.doc {
        if i < doc.project.floor_count() && i != doc.project.floor_index {
            doc.project.set_active_floor(i);
            doc.dirty = true;
        }
    }
}

fn add_floor(app: &mut OndaluxApp, above: bool, same_plan: bool) {
    let Some(doc) = &app.doc else {
        return;
    };
    let current = doc.project.floor_index;
    let index = if above { current + 1 } else { current };
    let (mut floor, assets) = if same_plan {
        let mut f = Floor::new(FloorInfo::default(), 0, 0);
        f.plan = doc.project.plan.clone();
        (f, (**doc.assets()).clone())
    } else {
        let Some(path) = rfd::FileDialog::new()
            .set_title(t("Plano del nuevo piso", "Floor plan of the new floor"))
            .add_filter(t("Planos", "Floor plans"), &["png", "jpg", "jpeg", "pdf"])
            .pick_file()
        else {
            return;
        };
        match import_plan(&path) {
            Ok(imported) => {
                let mut f = Floor::new(
                    FloorInfo::default(),
                    imported.decoded.width,
                    imported.decoded.height,
                );
                // Same drawing size: most likely the same scale.
                let cur = &doc.project.plan;
                if (cur.width_px, cur.height_px) == (f.plan.width_px, f.plan.height_px) {
                    f.plan.calibration = cur.calibration;
                }
                let assets = FloorAssets {
                    plan_image: Some(imported.image),
                    plan_source: imported.source_pdf,
                };
                (f, assets)
            }
            Err(e) => {
                app.error(tr!(
                    "No se pudo importar {}: {e}",
                    "Could not import {}: {e}",
                    path.display()
                ));
                return;
            }
        }
    };
    let Some(doc) = &mut app.doc else {
        return;
    };
    // Name floors so APs seen from other floors say where they are.
    let n = doc.project.floor_count();
    if doc.project.floor.name.trim().is_empty() && n == 1 {
        doc.project.floor.name = label(&doc.project, 0);
    }
    floor.info.height_m = doc.project.floor.height_m;
    floor.info.slab_loss_db = doc.project.floor.slab_loss_db;
    floor.info.offset_m = doc.project.floor.offset_m;
    let calibrated = floor.plan.calibration.is_some();
    doc.add_floor(index, floor, assets);
    // Default name from its position.
    doc.project.floor.name = tr!("Piso {}", "Floor {}", index + 1);
    app.info(if calibrated {
        t(
            "Piso agregado. Dibuja sus muros y coloca sus APs.",
            "Floor added. Draw its walls and place its APs.",
        )
    } else {
        t(
            "Piso agregado. Calibra la escala de su plano.",
            "Floor added. Calibrate the scale of its plan.",
        )
    });
    if !calibrated {
        app.tool = crate::app::Tool::Calibrate;
    }
}

/// Floor settings window.
pub fn settings_window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.floors.settings_open {
        return;
    }
    let Some(doc) = &mut app.doc else {
        return;
    };
    let mut open = true;
    let index = doc.project.floor_index;
    let title = format!("{}  {}", icon::STACK, label(&doc.project, index));
    let mut info = doc.project.floor.clone();
    let mut cross = doc.project.cross_floor;
    let mut remove = false;
    let mut replace_plan = false;
    egui::Window::new(title)
        .id(egui::Id::new("floor-settings"))
        .open(&mut open)
        .resizable(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            egui::Grid::new("floor-grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(t("Nombre", "Name"));
                    ui.add(egui::TextEdit::singleline(&mut info.name).desired_width(180.0));
                    ui.end_row();

                    ui.label(t("Altura de piso a piso", "Floor-to-floor height"))
                        .on_hover_text(t(
                            "De este nivel al siguiente. Los muros en 3D tienen esta altura.",
                            "From this level to the next one. Walls in 3D are this tall.",
                        ));
                    ui.add(
                        egui::DragValue::new(&mut info.height_m)
                            .range(2.0..=12.0)
                            .speed(0.05)
                            .suffix(" m"),
                    );
                    ui.end_row();

                    ui.label(t("Losa bajo este piso", "Slab under this floor"))
                        .on_hover_text(t(
                            "Pérdida de señal al atravesar la losa entre este piso y el de abajo.",
                            "Signal loss through the slab between this floor and the one below.",
                        ));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut info.slab_loss_db)
                                .range(0.0..=40.0)
                                .speed(0.2)
                                .suffix(" dB"),
                        );
                        egui::ComboBox::from_id_salt("slab-preset")
                            .selected_text(t("Típicos", "Typical"))
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for (es, en, db) in [
                                    ("Losa de concreto", "Concrete slab", 15.0),
                                    ("Concreto grueso", "Thick concrete", 20.0),
                                    ("Losacero / metal", "Metal deck", 25.0),
                                    ("Entrepiso de madera", "Wooden floor", 6.0),
                                ] {
                                    if ui.button(format!("{}  ·  {db:.0} dB", t(es, en))).clicked() {
                                        info.slab_loss_db = db;
                                    }
                                }
                            });
                    });
                    ui.end_row();

                    ui.label(t("Desplazamiento del plano", "Plan offset"))
                        .on_hover_text(t(
                            "Mueve este plano respecto a los demás pisos para que coincidan (columnas, escaleras).",
                            "Moves this plan relative to the other floors so they line up (columns, stairs).",
                        ));
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut info.offset_m.x).speed(0.05).prefix("X ").suffix(" m"));
                        ui.add(egui::DragValue::new(&mut info.offset_m.y).speed(0.05).prefix("Y ").suffix(" m"));
                    });
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.checkbox(
                &mut cross,
                t(
                    "Incluir la señal de los APs de otros pisos",
                    "Include the signal of APs on other floors",
                ),
            )
            .on_hover_text(t(
                "Hasta dos pisos arriba y abajo, restando la pérdida de cada losa.",
                "Up to two floors above and below, minus the loss of each slab.",
            ));
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button(format!("{}  {}", icon::IMAGE, t("Cambiar plano…", "Replace plan…")))
                    .clicked()
                {
                    replace_plan = true;
                }
                if doc.project.floor_count() > 1
                    && style::danger_button(
                        ui,
                        &format!("{}  {}", icon::TRASH, t("Eliminar piso", "Delete floor")),
                    )
                    .clicked()
                {
                    remove = true;
                }
            });
        });
    if info != doc.project.floor {
        doc.checkpoint_coalesced("floor-settings");
        doc.project.floor = info;
    }
    if cross != doc.project.cross_floor {
        doc.checkpoint();
        doc.project.cross_floor = cross;
    }
    if remove {
        let name = label(&doc.project, index);
        doc.remove_floor(index);
        app.info(tr!(
            "Piso «{name}» eliminado (Ctrl+Z para deshacer).",
            "Floor “{name}” deleted (Ctrl+Z to undo)."
        ));
    }
    if replace_plan {
        replace_floor_plan(app);
    }
    app.floors.settings_open = open;
}

/// Replaces the active floor's plan image. Walls and APs keep their
/// position in meters.
fn replace_floor_plan(app: &mut OndaluxApp) {
    let Some(path) = rfd::FileDialog::new()
        .set_title(t("Cambiar plano", "Replace plan"))
        .add_filter(t("Planos", "Floor plans"), &["png", "jpg", "jpeg", "pdf"])
        .pick_file()
    else {
        return;
    };
    match import_plan(&path) {
        Ok(imported) => {
            let Some(doc) = &mut app.doc else {
                return;
            };
            doc.checkpoint();
            let plan = &mut doc.project.plan;
            let same_size = (plan.width_px, plan.height_px)
                == (imported.decoded.width, imported.decoded.height);
            plan.width_px = imported.decoded.width;
            plan.height_px = imported.decoded.height;
            if !same_size {
                plan.calibration = None;
            }
            doc.set_assets(FloorAssets {
                plan_image: Some(imported.image),
                plan_source: imported.source_pdf,
            });
            if same_size {
                app.info(t("Plano reemplazado.", "Plan replaced."));
            } else {
                app.tool = crate::app::Tool::Calibrate;
                app.info(t(
                    "Plano reemplazado. Calibra su escala.",
                    "Plan replaced. Calibrate its scale.",
                ));
            }
        }
        Err(e) => app.error(tr!(
            "No se pudo importar {}: {e}",
            "Could not import {}: {e}",
            path.display()
        )),
    }
}
