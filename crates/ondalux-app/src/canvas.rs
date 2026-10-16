//! The floor plan canvas: drawing and tool interaction.

use crate::app::{OndaluxApp, Selection, Tool};
use crate::heatmap::{self, Scale, Source};
use crate::i18n::t;
use crate::tr;
use crate::view::View;
use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
};
use ondalux_core::ap::{AccessPoint, ApId, Mount, Radio};
use ondalux_core::channel::{Band, Channel, ChannelWidth};
use ondalux_core::geometry::{Segment, Vec2 as WVec2};
use ondalux_core::material::Wall;
use ondalux_core::metrics::{MetricKind, Requirements};
use ondalux_core::project::{FloorPlan, Project};
use ondalux_core::units::{Db, Dbm};

/// Hit-test tolerances, in screen points.
const AP_HIT: f32 = 12.0;
const HANDLE_HIT: f32 = 8.0;
const WALL_HIT: f32 = 6.0;
const SNAP: f32 = 10.0;
const POINT_HIT: f32 = 9.0;
/// Distance in screen pixels from a wall AP to its rotation handle.
const ROTATE_HANDLE: f32 = 38.0;

#[derive(Debug, Clone, Copy)]
enum Drag {
    Ap { id: ApId, offset: WVec2 },
    Rotate { id: ApId },
    Measurement { id: u32, offset: WVec2 },
    WallEnd { index: usize, end_b: bool },
    Pan,
}

#[derive(Default)]
pub struct CanvasState {
    /// Last point of the wall being drawn, in meters.
    wall_draft: Option<WVec2>,
    /// First calibration point, in image pixels.
    calib_first: Option<WVec2>,
    /// Both calibration points (pixels) and the distance being typed.
    pub calib_pending: Option<(WVec2, WVec2, String)>,
    drag: Option<Drag>,
    /// Cursor readout for the status bar, refreshed every frame.
    pub hover_text: Option<String>,
    /// Keep the plan fitted while the canvas resizes (e.g. the window being
    /// maximized after load) until the user zooms or pans.
    auto_fit: bool,
    last_rect: Option<Rect>,
}

impl CanvasState {
    /// The user zoomed: stop refitting the plan when the canvas resizes.
    pub fn stop_auto_fit(&mut self) {
        self.auto_fit = false;
    }

    pub fn cancel(&mut self) {
        self.wall_draft = None;
        self.calib_first = None;
        self.drag = None;
    }

    pub fn is_drafting(&self) -> bool {
        self.wall_draft.is_some() || self.calib_first.is_some()
    }
}

/// Screen mapping for one frame (meters ↔ image pixels ↔ screen points).
#[derive(Clone, Copy)]
struct Xf<'a> {
    view: View,
    plan: &'a FloorPlan,
}

impl Xf<'_> {
    fn to_screen(self, m: WVec2) -> Pos2 {
        self.view.to_screen(self.plan.m_to_px(m))
    }

    fn to_m(self, s: Pos2) -> WVec2 {
        self.plan.px_to_m(self.view.to_image(s))
    }

    /// Screen points per meter.
    fn scale(self) -> f32 {
        self.view.zoom * self.plan.px_per_m()
    }
}

pub fn show(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let painter = ui.painter_at(rect);

    let (w_px, h_px) = {
        let plan = &app
            .doc
            .as_ref()
            .expect("canvas needs a document")
            .project
            .plan;
        (plan.width_px as f32, plan.height_px as f32)
    };
    let size = egui::vec2(w_px, h_px);
    if app.fit_pending || (app.canvas.auto_fit && app.canvas.last_rect != Some(rect)) {
        app.view = View::fit(rect, size);
        app.fit_pending = false;
        app.canvas.auto_fit = true;
    }
    app.canvas.last_rect = Some(rect);
    let view_before = (app.view.origin, app.view.zoom);

    // Zoom with the wheel (or pinch / Ctrl+wheel), pan with the middle button.
    if response.hovered() {
        let (scroll, zoom, pos) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.zoom_delta(),
                i.pointer.hover_pos(),
            )
        });
        if let Some(p) = pos {
            if zoom != 1.0 {
                app.view.zoom_around(p, zoom);
            } else if scroll != 0.0 {
                app.view.zoom_around(p, (scroll * 0.002).exp());
            }
        }
    }
    if response.dragged_by(PointerButton::Middle) {
        app.view.origin += response.drag_delta();
    }

    let pointer = response.hover_pos().or(response.interact_pointer_pos());
    let shift = ui.input(|i| i.modifiers.shift);
    let selection_before = app.selection;
    handle_tool(app, &response, pointer, shift);
    // Side panels were drawn before the canvas this frame; repaint so they
    // reflect what the tool just changed.
    if app.selection != selection_before
        || response.clicked()
        || response.secondary_clicked()
        || response.drag_stopped()
    {
        ui.ctx().request_repaint();
    }
    if (app.view.origin, app.view.zoom) != view_before {
        app.canvas.auto_fit = false;
    }

    // ---- drawing ----
    let doc = app.doc.as_ref().expect("canvas needs a document");
    let project = &doc.project;
    let xf = Xf {
        view: app.view,
        plan: &project.plan,
    };
    let plan_rect = app.view.image_rect(size);
    // Dotted backdrop and a soft shadow under the plan.
    let pal = if ui.visuals().dark_mode {
        crate::style::DARK
    } else {
        crate::style::LIGHT
    };
    let dot = pal.border;
    let step = 22.0;
    let mut y = rect.top() + (step - (rect.top() % step));
    while y < rect.bottom() {
        let mut x = rect.left() + (step - (rect.left() % step));
        while x < rect.right() {
            painter.circle_filled(Pos2::new(x, y), 1.0, dot);
            x += step;
        }
        y += step;
    }
    painter.add(
        egui::Shadow {
            offset: [0, 8],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha(if ui.visuals().dark_mode { 140 } else { 45 }),
        }
        .as_shape(plan_rect, 4.0),
    );
    match &app.plan_texture {
        Some(tex) => {
            painter.image(
                tex.id(),
                plan_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(plan_rect, 0.0, Color32::from_gray(235));
        }
    }

    if app.heat.show {
        if let (Some(tex), Some(f)) = (&app.heat.texture, &app.heat.field) {
            let g = f.grid;
            let grid_rect = Rect::from_min_max(
                xf.to_screen(g.origin),
                xf.to_screen(WVec2::new(
                    g.origin.x + g.cols as f32 * g.cell_m,
                    g.origin.y + g.rows as f32 * g.cell_m,
                )),
            );
            let alpha = (app.heat.opacity * 255.0).round() as u8;
            painter.with_clip_rect(plan_rect.intersect(rect)).image(
                tex.id(),
                grid_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::from_white_alpha(alpha),
            );
        }
    }

    draw_walls(&painter, project, xf, app.selection);
    crate::walls_ui::draw_preview(app, &painter);
    draw_aps(&painter, project, xf, app.selection, app.heat.band);
    draw_measurements(&painter, app, xf, ui.input(|i| i.time));
    draw_tool_overlay(app, &painter, xf, pointer, shift);

    if let (true, Some(field)) = (app.heat.show, &app.heat.field) {
        draw_legend(
            &painter,
            rect,
            field.scale,
            field.source,
            project.requirements,
            ui.visuals(),
        );
    }
    if app.survey.scanning_at().is_some() {
        ui.ctx().request_repaint();
    }

    // ---- status bar readout ----
    app.canvas.hover_text = pointer.filter(|p| rect.contains(*p)).map(|p| {
        let m = xf.to_m(p);
        let mut s = format!("x {:.2} m  y {:.2} m", m.x, m.y);
        if let Some(field) = &app.heat.field {
            let g = field.grid;
            let (c, r) = ((m.x - g.origin.x) / g.cell_m, (m.y - g.origin.y) / g.cell_m);
            if c >= 0.0 && r >= 0.0 {
                let kind = app.heat.metric;
                let text = match (field.value_at(c as usize, r as usize), field.scale) {
                    (Some(v), Scale::Difference) => format!("{v:+.1}"),
                    (Some(v), _) => format_value(kind, v),
                    (None, _) => t("sin datos", "no data").to_owned(),
                };
                let prefix = match field.source {
                    Source::Predicted => "",
                    Source::Measured => t("medido ", "measured "),
                    Source::Difference => "Δ ",
                };
                s.push_str(&format!(
                    "   {prefix}{}: {text}",
                    crate::app::metric_label(kind)
                ));
            }
        }
        s
    });
}

fn format_value(kind: MetricKind, v: f32) -> String {
    match kind {
        MetricKind::Rssi | MetricKind::Snr | MetricKind::Sinr => {
            format!("{v:.1} {}", kind.unit())
        }
        MetricKind::Coverage => if v > 0.5 {
            t("cumple requisitos", "meets requirements")
        } else {
            t("bajo requisitos", "below requirements")
        }
        .into(),
        _ => format!("{v:.0}"),
    }
}

/// Screen position of the rotation handle of a wall-mounted AP.
fn rotate_handle(ap: &AccessPoint, xf: Xf) -> Option<Pos2> {
    (ap.mount == Mount::Wall).then(|| {
        let d = ap.direction();
        xf.to_screen(ap.position) + egui::vec2(d.x, d.y) * ROTATE_HANDLE
    })
}

/// Plan angle (0 = right, clockwise on screen) from `center` to `p`, snapped
/// to 15° steps when `snap` is set.
fn angle_to(center: Pos2, p: Pos2, snap: bool) -> f32 {
    let v = p - center;
    let mut a = v.y.atan2(v.x).to_degrees();
    if snap {
        a = (a / 15.0).round() * 15.0;
    }
    a.rem_euclid(360.0)
}

fn hit_ap(project: &Project, xf: Xf, p: Pos2) -> Option<ApId> {
    project
        .aps
        .iter()
        .map(|a| (a.id, xf.to_screen(a.position).distance(p)))
        .filter(|(_, d)| *d <= AP_HIT)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn hit_measurement(app: &OndaluxApp, xf: Xf, p: Pos2) -> Option<u32> {
    let survey = app.doc.as_ref()?.project.surveys.get(app.survey.active)?;
    survey
        .measurements
        .iter()
        .map(|m| (m.id, xf.to_screen(m.position).distance(p)))
        .filter(|(_, d)| *d <= POINT_HIT)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn hit_wall_end(project: &Project, xf: Xf, p: Pos2, only: Option<usize>) -> Option<(usize, bool)> {
    project
        .walls
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .flat_map(|(i, w)| [(i, false, w.segment.a), (i, true, w.segment.b)])
        .map(|(i, b, m)| (i, b, xf.to_screen(m).distance(p)))
        .filter(|(_, _, d)| *d <= HANDLE_HIT)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(i, b, _)| (i, b))
}

fn hit_wall(project: &Project, xf: Xf, p: Pos2) -> Option<usize> {
    let m = xf.to_m(p);
    let tol = WALL_HIT / xf.scale();
    project
        .walls
        .iter()
        .enumerate()
        .map(|(i, w)| (i, w.segment.distance_to_point(m)))
        .filter(|(_, d)| *d <= tol)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// Snaps to existing wall endpoints, or to 45° steps from `anchor` with Shift.
fn snap(
    project: &Project,
    xf: Xf,
    p: Pos2,
    anchor: Option<WVec2>,
    shift: bool,
    skip: Option<usize>,
) -> WVec2 {
    let nearest = project
        .walls
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != skip)
        .flat_map(|(_, w)| [w.segment.a, w.segment.b])
        .map(|m| (m, xf.to_screen(m).distance(p)))
        .filter(|(_, d)| *d <= SNAP)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((m, _)) = nearest {
        return m;
    }
    let m = xf.to_m(p);
    match anchor {
        Some(a) if shift => {
            let d = m - a;
            let len = d.length();
            let step = std::f32::consts::FRAC_PI_4;
            let ang = (d.y.atan2(d.x) / step).round() * step;
            WVec2::new(a.x + len * ang.cos(), a.y + len * ang.sin())
        }
        _ => m,
    }
}

fn clamp_to_plan(plan: &FloorPlan, m: WVec2) -> WVec2 {
    let (w, h) = plan.size_m();
    WVec2::new(m.x.clamp(0.0, w), m.y.clamp(0.0, h))
}

fn handle_tool(
    app: &mut OndaluxApp,
    response: &egui::Response,
    pointer: Option<Pos2>,
    shift: bool,
) {
    let view = app.view;
    let tool = app.tool;
    let active_survey = app.survey.active;
    let press_pos = response
        .ctx
        .input(|i| i.pointer.press_origin())
        .or(response.interact_pointer_pos());
    let (point_at_press, point_at_click) = {
        let Some(doc) = app.doc.as_ref() else {
            return;
        };
        let xf = Xf {
            view,
            plan: &doc.project.plan,
        };
        (
            press_pos.and_then(|p| hit_measurement(app, xf, p)),
            response
                .interact_pointer_pos()
                .and_then(|p| hit_measurement(app, xf, p)),
        )
    };
    let mut scan_error = None;
    let Some(doc) = app.doc.as_mut() else {
        return;
    };

    match tool {
        Tool::Select => {
            if response.drag_started_by(PointerButton::Primary) {
                // Hit-test where the button went down: a drag is only
                // recognized after the pointer has already moved.
                let p = response
                    .ctx
                    .input(|i| i.pointer.press_origin())
                    .or(response.interact_pointer_pos())
                    .unwrap_or_default();
                let xf = Xf {
                    view,
                    plan: &doc.project.plan,
                };
                let selected_wall = match app.selection {
                    Selection::Wall(i) => Some(i),
                    _ => None,
                };
                let rotating = match app.selection {
                    Selection::Ap(id) => doc
                        .project
                        .ap(id)
                        .and_then(|a| rotate_handle(a, xf))
                        .is_some_and(|h| h.distance(p) <= HANDLE_HIT + 3.0)
                        .then_some(id),
                    _ => None,
                };
                if let Some(id) = rotating {
                    doc.checkpoint();
                    app.canvas.drag = Some(Drag::Rotate { id });
                } else if let Some(id) = hit_ap(&doc.project, xf, p) {
                    let pos = doc.project.ap(id).map(|a| a.position).unwrap_or_default();
                    let offset = pos - xf.to_m(p);
                    doc.checkpoint();
                    app.canvas.drag = Some(Drag::Ap { id, offset });
                    app.selection = Selection::Ap(id);
                } else if let Some(id) = point_at_press {
                    let pos = doc
                        .project
                        .surveys
                        .get(active_survey)
                        .and_then(|s| s.measurements.iter().find(|m| m.id == id))
                        .map(|m| m.position)
                        .unwrap_or_default();
                    let offset = pos - xf.to_m(p);
                    doc.checkpoint();
                    app.canvas.drag = Some(Drag::Measurement { id, offset });
                    app.selection = Selection::Measurement(id);
                } else if let Some((index, end_b)) =
                    hit_wall_end(&doc.project, xf, p, selected_wall)
                {
                    doc.checkpoint();
                    app.canvas.drag = Some(Drag::WallEnd { index, end_b });
                    app.selection = Selection::Wall(index);
                } else {
                    app.canvas.drag = Some(Drag::Pan);
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                match app.canvas.drag {
                    Some(Drag::Rotate { id }) => {
                        if let Some(p) = pointer {
                            let xf = Xf {
                                view,
                                plan: &doc.project.plan,
                            };
                            let center = doc.project.ap(id).map(|a| xf.to_screen(a.position));
                            if let (Some(c), Some(ap)) = (center, doc.project.ap_mut(id)) {
                                ap.orientation_deg = angle_to(c, p, shift);
                            }
                        }
                    }
                    Some(Drag::Ap { id, offset }) => {
                        if let Some(p) = pointer {
                            let xf = Xf {
                                view,
                                plan: &doc.project.plan,
                            };
                            let target = clamp_to_plan(&doc.project.plan, xf.to_m(p) + offset);
                            if let Some(ap) = doc.project.ap_mut(id) {
                                ap.position = target;
                            }
                        }
                    }
                    Some(Drag::WallEnd { index, end_b }) => {
                        if let Some(p) = pointer {
                            let xf = Xf {
                                view,
                                plan: &doc.project.plan,
                            };
                            let other = doc.project.walls.get(index).map(|w| {
                                if end_b {
                                    w.segment.a
                                } else {
                                    w.segment.b
                                }
                            });
                            let m = snap(&doc.project, xf, p, other, shift, Some(index));
                            if let Some(w) = doc.project.walls.get_mut(index) {
                                if end_b {
                                    w.segment.b = m;
                                } else {
                                    w.segment.a = m;
                                }
                            }
                        }
                    }
                    Some(Drag::Measurement { id, offset }) => {
                        if let Some(p) = pointer {
                            let xf = Xf {
                                view,
                                plan: &doc.project.plan,
                            };
                            let target = clamp_to_plan(&doc.project.plan, xf.to_m(p) + offset);
                            if let Some(m) = doc
                                .project
                                .surveys
                                .get_mut(active_survey)
                                .and_then(|s| s.measurements.iter_mut().find(|m| m.id == id))
                            {
                                m.position = target;
                            }
                        }
                    }
                    Some(Drag::Pan) => app.view.origin += response.drag_delta(),
                    None => {}
                }
            }
            if response.drag_stopped() {
                app.canvas.drag = None;
            }
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let xf = Xf {
                        view,
                        plan: &doc.project.plan,
                    };
                    app.selection = if let Some(id) = hit_ap(&doc.project, xf, p) {
                        Selection::Ap(id)
                    } else if let Some(id) = point_at_click {
                        Selection::Measurement(id)
                    } else if let Some(i) = hit_wall(&doc.project, xf, p) {
                        Selection::Wall(i)
                    } else {
                        Selection::None
                    };
                }
            }
        }
        Tool::Wall => {
            if response.double_clicked() || response.secondary_clicked() {
                app.canvas.wall_draft = None;
            } else if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let xf = Xf {
                        view,
                        plan: &doc.project.plan,
                    };
                    let m = snap(&doc.project, xf, p, app.canvas.wall_draft, shift, None);
                    if let Some(a) = app.canvas.wall_draft {
                        if a.distance(m) > 1e-3 {
                            doc.checkpoint();
                            doc.project.walls.push(Wall {
                                segment: Segment::new(a, m),
                                material: app.wall_material,
                                thickness_cm: app.wall_thickness_cm,
                            });
                        }
                    }
                    app.canvas.wall_draft = Some(m);
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                app.view.origin += response.drag_delta();
            }
        }
        Tool::Ap => {
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let xf = Xf {
                        view,
                        plan: &doc.project.plan,
                    };
                    let m = clamp_to_plan(&doc.project.plan, xf.to_m(p));
                    doc.checkpoint();
                    let mut ap = new_ap(&doc.project, m);
                    if let Some(model) = app.catalog.new_ap_model() {
                        ap.radios.clear();
                        model.apply_to(&mut ap, doc.project.aps.len());
                    }
                    app.selection = Selection::Ap(ap.id);
                    doc.project.aps.push(ap);
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                app.view.origin += response.drag_delta();
            }
        }
        Tool::Survey => {
            if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let xf = Xf {
                        view,
                        plan: &doc.project.plan,
                    };
                    let m = clamp_to_plan(&doc.project.plan, xf.to_m(p));
                    if let Err(e) = app.survey.start_scan(m, &doc.project, &response.ctx) {
                        scan_error = Some(e);
                    }
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                app.view.origin += response.drag_delta();
            }
        }
        Tool::Calibrate => {
            if response.secondary_clicked() {
                app.canvas.calib_first = None;
            } else if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let px = view.to_image(p);
                    match app.canvas.calib_first.take() {
                        None => app.canvas.calib_first = Some(px),
                        Some(first) => {
                            app.canvas.calib_pending = Some((first, px, String::new()));
                        }
                    }
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                app.view.origin += response.drag_delta();
            }
        }
    }
    if let Some(e) = scan_error {
        app.error(e);
    }
}

/// A new AP with a 2.4 GHz and a 5 GHz radio on rotating channels, and a
/// disabled 6 GHz radio.
pub fn new_ap(project: &Project, position: WVec2) -> AccessPoint {
    let id = project.next_ap_id();
    let n = project.aps.len();
    let radio = |band, ch, width, tx, gain, enabled| Radio {
        channel: Channel::new(band, ch, width).expect("default channels are valid"),
        tx_power: Dbm(tx),
        antenna_gain: Db(gain),
        enabled,
    };
    AccessPoint {
        id,
        name: format!("AP-{}", id.0),
        vendor: String::new(),
        model: "Generic".into(),
        position,
        height_m: 3.0,
        orientation_deg: 0.0,
        mount: Default::default(),
        pattern: Default::default(),
        radios: vec![
            radio(
                Band::G2_4,
                [1, 6, 11][n % 3],
                ChannelWidth::Mhz20,
                14.0,
                3.0,
                true,
            ),
            radio(
                Band::G5,
                [36, 52, 100, 149][n % 4],
                ChannelWidth::Mhz40,
                17.0,
                5.0,
                true,
            ),
            radio(
                Band::G6,
                [5, 21, 37, 53][n % 4],
                ChannelWidth::Mhz80,
                17.0,
                5.0,
                false,
            ),
        ],
    }
}

fn draw_walls(painter: &egui::Painter, project: &Project, xf: Xf, selection: Selection) {
    for (i, w) in project.walls.iter().enumerate() {
        let color = project
            .materials
            .get(w.material)
            .map_or(Color32::GRAY, |m| {
                Color32::from_rgba_unmultiplied(m.color[0], m.color[1], m.color[2], m.color[3])
            });
        let (a, b) = (xf.to_screen(w.segment.a), xf.to_screen(w.segment.b));
        let selected = selection == Selection::Wall(i);
        // At scale, but never thinner than 3 px.
        let width = (w.effective_thickness_cm(&project.materials) * 0.01 * xf.scale()).max(3.0);
        if selected {
            painter.line_segment(
                [a, b],
                Stroke::new(width + 4.0, Color32::from_rgb(80, 160, 255)),
            );
        }
        painter.line_segment([a, b], Stroke::new(width, color));
        if selected {
            for p in [a, b] {
                painter.circle(p, 5.0, Color32::WHITE, Stroke::new(1.5, Color32::BLACK));
            }
        }
    }
}

/// APs of the floors right above and below, faint, where they would be on
/// this plan.
fn draw_other_floor_aps(painter: &egui::Painter, project: &Project, xf: Xf) {
    let f = project.floor_index;
    let color = Color32::from_rgba_unmultiplied(110, 120, 140, 150);
    for g in [f.wrapping_sub(1), f + 1] {
        if g >= project.floor_count() {
            continue;
        }
        let shift = project.floor_info(g).offset_m - project.floor.offset_m;
        let arrow = if g > f {
            crate::style::icon::ARROW_UP
        } else {
            crate::style::icon::ARROW_DOWN
        };
        for ap in project.floor_aps(g) {
            let p = xf.to_screen(ap.position + shift);
            painter.circle_stroke(p, 7.0, Stroke::new(1.5, color));
            painter.text(
                p + egui::vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{arrow} {}", ap.name),
                egui::FontId::proportional(11.0),
                color,
            );
        }
    }
}

fn draw_aps(painter: &egui::Painter, project: &Project, xf: Xf, selection: Selection, band: Band) {
    draw_other_floor_aps(painter, project, xf);
    for ap in &project.aps {
        let p = xf.to_screen(ap.position);
        let selected = selection == Selection::Ap(ap.id);
        let has_band = ap.enabled_radios().any(|(_, r)| r.channel.band == band);
        let fill = if has_band {
            Color32::from_rgb(40, 110, 220)
        } else {
            Color32::from_gray(120)
        };
        if ap.mount == Mount::Wall {
            draw_wall_mount(painter, ap, p, fill, selected);
        }
        if selected {
            painter.circle_stroke(p, 14.0, Stroke::new(3.0, Color32::from_rgb(80, 160, 255)));
        }
        painter.circle(p, 9.0, fill, Stroke::new(2.0, Color32::WHITE));
        let channels: Vec<String> = ap
            .enabled_radios()
            .filter(|(_, r)| r.channel.band == band)
            .map(|(_, r)| r.channel.number.to_string())
            .collect();
        let label = if channels.is_empty() {
            ap.name.clone()
        } else {
            format!("{} · ch {}", ap.name, channels.join("/"))
        };
        // Wall APs pointing down keep their label above, off the beam.
        let above = ap.mount == Mount::Wall && ap.direction().y > 0.5;
        let (galley_pos, align) = if above {
            (p - egui::vec2(0.0, 16.0), Align2::CENTER_BOTTOM)
        } else {
            (p + egui::vec2(0.0, 13.0), Align2::CENTER_TOP)
        };
        let font = FontId::proportional(12.0);
        let text_rect = painter.text(galley_pos, align, &label, font.clone(), Color32::WHITE);
        painter.rect_filled(text_rect.expand(2.0), 3.0, Color32::from_black_alpha(170));
        painter.text(galley_pos, align, label, font, Color32::WHITE);
    }
}

/// Wall-mounted AP: the beam as a translucent fan, a bar for the wall behind
/// it and, when selected, the rotation handle.
fn draw_wall_mount(
    painter: &egui::Painter,
    ap: &AccessPoint,
    p: Pos2,
    fill: Color32,
    selected: bool,
) {
    let d = ap.direction();
    let dir = egui::vec2(d.x, d.y);
    let normal = egui::vec2(-dir.y, dir.x);
    // Fan of the half-power beam (drawn at most as a half disc).
    let half = (ap.pattern.beamwidth_deg.min(180.0) / 2.0).to_radians();
    let base = d.y.atan2(d.x);
    let radius = ROTATE_HANDLE - 6.0;
    let mut fan = vec![p];
    for i in 0..=16 {
        let a = base - half + 2.0 * half * i as f32 / 16.0;
        fan.push(p + egui::vec2(a.cos(), a.sin()) * radius);
    }
    let alpha = if selected { 110 } else { 70 };
    painter.add(Shape::convex_polygon(
        fan,
        Color32::from_rgba_unmultiplied(fill.r(), fill.g(), fill.b(), alpha),
        Stroke::NONE,
    ));
    // The wall the AP hangs on.
    let back = p - dir * 11.0;
    painter.line_segment(
        [back - normal * 11.0, back + normal * 11.0],
        Stroke::new(4.0, Color32::from_gray(235)),
    );
    if selected {
        let h = p + dir * ROTATE_HANDLE;
        let accent = Color32::from_rgb(255, 170, 40);
        painter.line_segment([p + dir * 10.0, h], Stroke::new(2.0, accent));
        painter.circle(h, 6.0, accent, Stroke::new(1.5, Color32::WHITE));
    }
}

fn draw_tool_overlay(
    app: &OndaluxApp,
    painter: &egui::Painter,
    xf: Xf,
    pointer: Option<Pos2>,
    shift: bool,
) {
    let accent = Color32::from_rgb(255, 170, 40);
    let project = &app.doc.as_ref().expect("document").project;
    match app.tool {
        Tool::Wall => {
            if let (Some(a), Some(p)) = (app.canvas.wall_draft, pointer) {
                let m = snap(project, xf, p, Some(a), shift, None);
                let (sa, sb) = (xf.to_screen(a), xf.to_screen(m));
                painter.add(Shape::dashed_line(
                    &[sa, sb],
                    Stroke::new(2.0, accent),
                    6.0,
                    4.0,
                ));
                label_box(
                    painter,
                    sb + egui::vec2(12.0, -12.0),
                    format!("{:.2} m", a.distance(m)),
                );
            }
        }
        Tool::Calibrate => {
            let current = project.plan.calibration;
            if let Some(c) = current.filter(|_| app.canvas.calib_first.is_none()) {
                let (a, b) = (xf.view.to_screen(c.a_px), xf.view.to_screen(c.b_px));
                painter.line_segment([a, b], Stroke::new(2.0, accent.gamma_multiply(0.6)));
                label_box(painter, a.lerp(b, 0.5), format!("{:.2} m", c.distance_m));
            }
            if let Some(first) = app.canvas.calib_first {
                let a = xf.view.to_screen(first);
                painter.circle_filled(a, 4.0, accent);
                if let Some(p) = pointer {
                    painter.line_segment([a, p], Stroke::new(2.0, accent));
                }
            }
            if let Some((a, b, _)) = &app.canvas.calib_pending {
                let (a, b) = (xf.view.to_screen(*a), xf.view.to_screen(*b));
                painter.line_segment([a, b], Stroke::new(2.0, accent));
                painter.circle_filled(a, 4.0, accent);
                painter.circle_filled(b, 4.0, accent);
            }
        }
        _ => {}
    }
}

fn label_box(painter: &egui::Painter, pos: Pos2, text: String) {
    let font = FontId::proportional(12.0);
    let r = painter.text(
        pos,
        Align2::LEFT_BOTTOM,
        &text,
        font.clone(),
        Color32::WHITE,
    );
    painter.rect_filled(r.expand(3.0), 3.0, Color32::from_black_alpha(190));
    painter.text(pos, Align2::LEFT_BOTTOM, text, font, Color32::WHITE);
}

fn draw_measurements(painter: &egui::Painter, app: &OndaluxApp, xf: Xf, time: f64) {
    let Some(survey) = app
        .doc
        .as_ref()
        .and_then(|d| d.project.surveys.get(app.survey.active))
    else {
        return;
    };
    let errors = app.heat.comparison.as_ref().map(|c| &c.points);
    let show_errors = app.heat.source == Source::Difference;
    for m in &survey.measurements {
        let p = xf.to_screen(m.position);
        let selected = app.selection == Selection::Measurement(m.id);
        let err = errors.and_then(|e| e.iter().find(|(id, _)| *id == m.id).map(|(_, v)| *v));
        let fill = match (show_errors, err) {
            (true, Some(e)) => heatmap::color_at(Scale::Difference, e),
            _ => Color32::WHITE,
        };
        if selected {
            painter.circle_stroke(p, 9.0, Stroke::new(2.5, Color32::from_rgb(80, 160, 255)));
        }
        painter.circle(p, 4.5, fill, Stroke::new(1.5, Color32::from_gray(30)));
        if let (true, Some(e)) = (show_errors, err) {
            label_box(painter, p + egui::vec2(7.0, -5.0), format!("{e:+.0}"));
        }
    }
    if let Some(at) = app.survey.scanning_at() {
        let p = xf.to_screen(at);
        let pulse = 8.0 + 6.0 * ((time * 4.0).sin() as f32 * 0.5 + 0.5);
        let orange = Color32::from_rgb(255, 170, 40);
        painter.circle_stroke(p, pulse, Stroke::new(2.0, orange));
        painter.circle_filled(p, 4.0, orange);
        label_box(
            painter,
            p + egui::vec2(14.0, -8.0),
            t("Escaneando…", "Scanning…").to_owned(),
        );
    }
}

pub(crate) fn draw_legend(
    painter: &egui::Painter,
    canvas: Rect,
    scale: Scale,
    source: Source,
    req: Requirements,
    visuals: &egui::Visuals,
) {
    let stops = heatmap::stops(scale);
    let pal = if visuals.dark_mode {
        crate::style::DARK
    } else {
        crate::style::LIGHT
    };
    let width = 300.0;
    let height = 66.0;
    let rect = Rect::from_min_size(
        canvas.left_bottom() + egui::vec2(12.0, -height - 12.0),
        egui::vec2(width, height),
    );
    painter.add(visuals.popup_shadow.as_shape(rect, 12.0));
    painter.rect(
        rect,
        12.0,
        pal.panel,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );
    let title = match scale {
        Scale::Difference => t(
            "Medido − predicho (dB): rojo = más débil",
            "Measured − predicted (dB): red = weaker",
        )
        .to_owned(),
        Scale::Metric(MetricKind::Coverage) => tr!(
            "Cobertura: ≥ {:.0} dBm y SNR ≥ {:.0} dB",
            "Coverage: ≥ {:.0} dBm and SNR ≥ {:.0} dB",
            req.min_rssi.0,
            req.min_snr.0
        ),
        Scale::Metric(MetricKind::ApCount) => tr!(
            "APs que se oyen a ≥ {:.0} dBm",
            "APs heard at ≥ {:.0} dBm",
            req.min_rssi.0
        ),
        Scale::Metric(MetricKind::ChannelOverlap) => {
            tr!(
                "Radios solapadas sobre {:.0} dBm",
                "Overlapping radios above {:.0} dBm",
                req.cca_threshold.0
            )
        }
        Scale::Metric(kind) => format!("{} ({})", crate::app::metric_label(kind), kind.unit()),
    };
    let title = match source {
        Source::Measured => tr!("Medido · {title}", "Measured · {title}"),
        _ => title,
    };
    painter.text(
        rect.min + egui::vec2(12.0, 9.0),
        Align2::LEFT_TOP,
        title,
        FontId::new(12.5, crate::style::semibold()),
        pal.text,
    );
    let bar = Rect::from_min_size(
        rect.min + egui::vec2(12.0, 30.0),
        egui::vec2(width - 24.0, 10.0),
    );
    let (lo, hi) = (stops[0].0, stops[stops.len() - 1].0);
    if heatmap::is_discrete(scale) {
        let n = stops.len();
        for (i, (v, _)) in stops.iter().enumerate() {
            let x0 = bar.left() + bar.width() * i as f32 / n as f32;
            let x1 = bar.left() + bar.width() * (i + 1) as f32 / n as f32;
            let cell = Rect::from_min_max(Pos2::new(x0, bar.top()), Pos2::new(x1, bar.bottom()));
            painter.rect_filled(cell, 0.0, heatmap::color_at(scale, *v));
            let text = match scale {
                Scale::Metric(MetricKind::Coverage) => if *v > 0.5 {
                    t("cumple", "meets")
                } else {
                    t("no cumple", "fails")
                }
                .to_owned(),
                _ if i == n - 1 => format!("{v:.0}+"),
                _ => format!("{v:.0}"),
            };
            painter.text(
                Pos2::new(cell.center().x, bar.bottom() + 2.0),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(11.0),
                pal.muted,
            );
        }
    } else {
        let steps = 48;
        for s in 0..steps {
            let t0 = s as f32 / steps as f32;
            let t1 = (s + 1) as f32 / steps as f32;
            let v = lo + (hi - lo) * (t0 + t1) * 0.5;
            let cell = Rect::from_min_max(
                Pos2::new(bar.left() + bar.width() * t0, bar.top()),
                Pos2::new(bar.left() + bar.width() * t1 + 0.5, bar.bottom()),
            );
            painter.rect_filled(cell, 0.0, heatmap::color_at(scale, v));
        }
        for (v, _) in stops {
            let x = bar.left() + bar.width() * (v - lo) / (hi - lo);
            let text = if scale == Scale::Difference && *v > 0.0 {
                format!("+{v:.0}")
            } else {
                format!("{v:.0}")
            };
            painter.text(
                Pos2::new(x, bar.bottom() + 2.0),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(11.0),
                pal.muted,
            );
        }
    }
}
