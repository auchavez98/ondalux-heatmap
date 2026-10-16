//! "Detect walls": builds the plan's ink mask in the background, shows the
//! detected walls as a preview over the plan, and adds them on request.

use crate::app::OndaluxApp;
use crate::i18n::{t, UiPrefs};
use crate::style::{self, icon};
use crate::tr;
use eframe::egui::{self, Color32, Stroke};
use ondalux_core::geometry::{Segment, Vec2 as WVec2};
use ondalux_core::material::{MaterialId, Wall};
use ondalux_core::walldetect::DetectedWall;
use ondalux_io::walls::{plan_mask, MaskSource, PlanMask, Tuning};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

/// Preview colors for thick and thin walls.
const THICK: Color32 = Color32::from_rgb(249, 115, 22);
const THIN: Color32 = Color32::from_rgb(14, 165, 233);

pub struct WallDetectState {
    pub open: bool,
    mask_job: Option<Receiver<Result<PlanMask, String>>>,
    mask: Option<Arc<PlanMask>>,
    detect_job: Option<(Tuning, Receiver<Vec<DetectedWall>>)>,
    error: Option<String>,
    pub tuning: Tuning,
    /// Tuning the current preview was computed with.
    shown: Option<Tuning>,
    /// Detected walls, in plan pixels.
    pub preview: Vec<DetectedWall>,
    pub thick_material: MaterialId,
    pub thin_material: MaterialId,
    pub replace_existing: bool,
}

impl Default for WallDetectState {
    fn default() -> Self {
        Self {
            open: false,
            mask_job: None,
            mask: None,
            detect_job: None,
            error: None,
            tuning: Tuning::default(),
            shown: None,
            preview: Vec::new(),
            thick_material: MaterialId(5),
            thin_material: MaterialId(1),
            replace_existing: false,
        }
    }
}

impl WallDetectState {
    fn busy(&self) -> bool {
        self.mask_job.is_some() || self.detect_job.is_some()
    }

    /// Thickness (plan px) above which a wall counts as thick: clearly
    /// thicker than the typical (median) wall.
    fn thick_threshold(&self) -> f32 {
        let mut t: Vec<f32> = self.preview.iter().map(|w| w.thickness_px).collect();
        if t.is_empty() {
            return f32::MAX;
        }
        t.sort_by(f32::total_cmp);
        t[t.len() / 2] * 1.35
    }
}

/// Opens the window and starts reading the plan.
pub fn start(app: &mut OndaluxApp, ctx: &egui::Context) {
    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    let Some(image) = doc.plan_image.clone() else {
        app.error(t(
            "El proyecto no tiene plano.",
            "The project has no floor plan.",
        ));
        return;
    };
    let source = doc.plan_source.clone();
    let st = &mut app.walls;
    *st = WallDetectState {
        open: true,
        thick_material: st.thick_material,
        thin_material: st.thin_material,
        replace_existing: !doc.project.walls.is_empty(),
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("wall-mask".into())
        .spawn(move || {
            let result = ondalux_io::decode_plan(&image)
                .and_then(|decoded| plan_mask(&decoded, source.as_deref()))
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => app.walls.mask_job = Some(rx),
        Err(e) => app.walls.error = Some(e.to_string()),
    }
}

/// Collects background results and re-runs detection when the tuning
/// changed.
pub fn poll(app: &mut OndaluxApp, ctx: &egui::Context) {
    let st = &mut app.walls;
    if let Some(rx) = &st.mask_job {
        if let Ok(result) = rx.try_recv() {
            st.mask_job = None;
            match result {
                Ok(mask) => st.mask = Some(Arc::new(mask)),
                Err(e) => st.error = Some(e),
            }
        }
    }
    if let Some((tuning, rx)) = &st.detect_job {
        if let Ok(walls) = rx.try_recv() {
            st.shown = Some(*tuning);
            st.preview = walls;
            st.detect_job = None;
        }
    }
    if !st.open || st.detect_job.is_some() || st.shown == Some(st.tuning) {
        return;
    }
    let Some(mask) = st.mask.clone() else {
        return;
    };
    let tuning = st.tuning;
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    if std::thread::Builder::new()
        .name("wall-detect".into())
        .spawn(move || {
            let _ = tx.send(mask.detect(tuning));
            ctx.request_repaint();
        })
        .is_ok()
    {
        st.detect_job = Some((tuning, rx));
    }
}

/// Draws the proposal over the plan while the window is open.
pub fn draw_preview(app: &OndaluxApp, painter: &egui::Painter) {
    let st = &app.walls;
    if !st.open {
        return;
    }
    let threshold = st.thick_threshold();
    for w in &st.preview {
        let a = app.view.to_screen(WVec2::new(w.a.x, w.a.y));
        let b = app.view.to_screen(WVec2::new(w.b.x, w.b.y));
        let color = if w.thickness_px >= threshold {
            THICK
        } else {
            THIN
        };
        let width = (w.thickness_px * app.view.zoom).max(3.0);
        painter.line_segment([a, b], Stroke::new(width, color.gamma_multiply(0.75)));
        painter.circle_filled(a, 2.5, color);
        painter.circle_filled(b, 2.5, color);
    }
}

fn apply(app: &mut OndaluxApp) {
    let st = &app.walls;
    let threshold = st.thick_threshold();
    let walls: Vec<(DetectedWall, MaterialId)> = st
        .preview
        .iter()
        .map(|w| {
            let m = if w.thickness_px >= threshold {
                st.thick_material
            } else {
                st.thin_material
            };
            (*w, m)
        })
        .collect();
    let replace = st.replace_existing;
    let Some(doc) = app.doc.as_mut() else {
        return;
    };
    doc.checkpoint();
    if replace {
        doc.project.walls.clear();
    }
    let plan = doc.project.plan.clone();
    let n = walls.len();
    for (w, material) in walls {
        let a = plan.px_to_m(WVec2::new(w.a.x, w.a.y));
        let b = plan.px_to_m(WVec2::new(w.b.x, w.b.y));
        doc.project.walls.push(Wall {
            segment: Segment::new(a, b),
            material,
        });
    }
    app.walls.open = false;
    app.walls.preview.clear();
    app.info(tr!(
        "{n} muros agregados. Revísalos: puedes cambiar su material, moverlos o borrarlos (Ctrl+Z deshace).",
        "{n} walls added. Review them: you can change their material, move or delete them (Ctrl+Z undoes)."
    ));
}

pub fn window(app: &mut OndaluxApp, ctx: &egui::Context) {
    if !app.walls.open {
        return;
    }
    let Some(doc) = app.doc.as_ref() else {
        app.walls.open = false;
        return;
    };
    let materials = doc.project.materials.clone();
    let has_walls = !doc.project.walls.is_empty();
    let mut open = true;
    let mut accept = false;
    let mut cancel = false;
    egui::Window::new(format!("{}  {}", icon::MAGIC_WAND, t("Detectar muros", "Detect walls")))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(330.0)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-720.0, 70.0))
        .show(ctx, |ui| {
            let p = style::palette(ui);
            let st = &mut app.walls;
            let prefs = &mut app.prefs;
            ui.horizontal(|ui| {
                beta_badge(ui);
                ui.label(
                    egui::RichText::new(t("Función en prueba", "Feature in testing"))
                        .size(12.0)
                        .color(p.muted),
                );
            });
            ui.set_width(320.0);
            if let Some(e) = &st.error {
                ui.colored_label(p.danger, tr!("No se pudo leer el plano: {e}", "Could not read the plan: {e}"));
                return;
            }
            match &st.mask {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(t("Analizando el plano…", "Analyzing the plan…"));
                    });
                    return;
                }
                Some(m) => {
                    let (glyph, text) = match m.source {
                        MaskSource::PdfVectors => (
                            icon::FILE_PDF,
                            t("Líneas vectoriales del PDF", "PDF vector line art"),
                        ),
                        MaskSource::Image => (icon::IMAGE, t("Imagen del plano", "Plan image")),
                    };
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(glyph).color(p.muted));
                        ui.label(egui::RichText::new(text).color(p.muted));
                    });
                }
            }

            // Result summary.
            let threshold = st.thick_threshold();
            let thick = st.preview.iter().filter(|w| w.thickness_px >= threshold).count();
            let thin = st.preview.len() - thick;
            ui.horizontal(|ui| {
                ui.label(style::bold(st.preview.len().to_string()).size(26.0));
                ui.label(egui::RichText::new(t("muros encontrados", "walls found")).color(p.muted));
                if st.busy() {
                    ui.spinner();
                }
            });

            style::section(ui, t("Sensibilidad", "Sensitivity"));
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t("Estricta", "Strict")).size(12.0).color(p.muted));
                ui.spacing_mut().slider_width = 170.0;
                ui.add(egui::Slider::new(&mut st.tuning.sensitivity, 0.0..=1.0).show_value(false))
                    .on_hover_text(t(
                        "Más permisiva encuentra muros cortos o delgados, pero también puede tomar muebles o cotas.",
                        "More permissive finds short or thin walls, but may also pick up furniture or dimensions.",
                    ));
                ui.label(egui::RichText::new(t("Permisiva", "Permissive")).size(12.0).color(p.muted));
            });

            style::section(ui, t("Materiales", "Materials"));
            egui::Grid::new("detect-materials").num_columns(2).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 4.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2.0, THICK);
                    ui.label(tr!("Gruesos ({thick})", "Thick ({thick})"));
                });
                crate::panels::material_combo(ui, "detect-thick", &materials, &mut st.thick_material, 170.0);
                ui.end_row();
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 4.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2.0, THIN);
                    ui.label(tr!("Delgados ({thin})", "Thin ({thin})"));
                });
                crate::panels::material_combo(ui, "detect-thin", &materials, &mut st.thin_material, 170.0);
                ui.end_row();
            });
            if has_walls {
                ui.checkbox(
                    &mut st.replace_existing,
                    t("Reemplazar los muros que ya existen", "Replace the existing walls"),
                );
            }
            auto_detect_checkbox(ui, prefs);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t(
                    "Detecta muros rectos (también diagonales) y aproxima los curvos con tramos rectos. Es una propuesta: revísala antes de confiar en el mapa de calor.",
                    "Finds straight walls (diagonal ones too) and approximates curved walls with straight pieces. It is a proposal: review it before trusting the heat map.",
                ))
                .size(12.0)
                .color(p.muted),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = tr!("Agregar {} muros", "Add {} walls", st.preview.len());
                if ui
                    .add_enabled(
                        !st.preview.is_empty() && !st.busy(),
                        egui::Button::new(style::bold(label).color(Color32::WHITE))
                            .fill(p.accent)
                            .corner_radius(8)
                            .min_size(egui::vec2(170.0, 34.0)),
                    )
                    .clicked()
                {
                    accept = true;
                }
                if ui
                    .add(egui::Button::new(t("Cancelar", "Cancel")).min_size(egui::vec2(100.0, 34.0)))
                    .clicked()
                {
                    cancel = true;
                }
            });
        });
    if accept {
        apply(app);
    } else if cancel || !open {
        app.walls.open = false;
        app.walls.preview.clear();
    }
}

/// "BETA" pill next to a feature name.
pub fn beta_badge(ui: &mut egui::Ui) {
    let p = style::palette(ui);
    style::badge(ui, "BETA", p.warning).on_hover_text(t(
        "Función en prueba: los resultados pueden necesitar correcciones a mano.",
        "Feature in testing: results may need manual corrections.",
    ));
}

/// Checkbox for the beta option "detect walls when importing a plan";
/// saves the preference when it changes.
pub fn auto_detect_checkbox(ui: &mut egui::Ui, prefs: &mut UiPrefs) {
    ui.horizontal(|ui| {
        let changed = ui
            .checkbox(
                &mut prefs.auto_detect_walls,
                t(
                    "Detectar muros automáticamente al importar un plano",
                    "Detect walls automatically when importing a plan",
                ),
            )
            .on_hover_text(t(
                "Al importar un plano se propone el trazado de muros para que lo revises antes de agregarlo. Función beta: detecta muros rectos, diagonales y curvos.",
                "When a plan is imported, walls are proposed for you to review before adding them. Beta feature: detects straight, diagonal and curved walls.",
            ))
            .changed();
        beta_badge(ui);
        if changed {
            prefs.save();
        }
    });
}
