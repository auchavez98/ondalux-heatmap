//! The floor plan canvas: drawing and tool interaction.

use crate::app::{OndaluxApp, Selection, Tool};
use crate::heatmap;
use crate::view::View;
use eframe::egui::{
    self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind,
};
use ondalux_core::ap::{AccessPoint, ApId, Radio};
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

#[derive(Debug, Clone, Copy)]
enum Drag {
    Ap { id: ApId, offset: WVec2 },
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
        if let (Some(tex), Some(m)) = (&app.heat.texture, &app.heat.metrics) {
            let g = m.grid;
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
    draw_aps(&painter, project, xf, app.selection, app.heat.band);
    draw_tool_overlay(app, &painter, xf, pointer, shift);

    if app.heat.show && project.aps.is_empty() {
        painter.text(
            rect.center_top() + egui::vec2(0.0, 16.0),
            Align2::CENTER_TOP,
            "Place access points with the AP tool (A) to see the heat map",
            FontId::proportional(14.0),
            ui.visuals().weak_text_color(),
        );
    }
    if app.heat.show && app.heat.texture.is_some() {
        draw_legend(
            &painter,
            rect,
            app.heat.metric,
            project.requirements,
            ui.visuals(),
        );
    }

    // ---- status bar readout ----
    app.canvas.hover_text = pointer.filter(|p| rect.contains(*p)).map(|p| {
        let m = xf.to_m(p);
        let mut s = format!("x {:.2} m  y {:.2} m", m.x, m.y);
        if let Some(metrics) = &app.heat.metrics {
            let g = metrics.grid;
            let (c, r) = ((m.x - g.origin.x) / g.cell_m, (m.y - g.origin.y) / g.cell_m);
            if c >= 0.0 && r >= 0.0 && (c as usize) < g.cols && (r as usize) < g.rows {
                let i = r as usize * g.cols + c as usize;
                let kind = app.heat.metric;
                let v = metrics.value(kind, i);
                s.push_str(&format!("   {}: {}", kind.label(), format_value(kind, v)));
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
            "meets requirements"
        } else {
            "below requirements"
        }
        .into(),
        _ => format!("{v:.0}"),
    }
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
                if let Some(id) = hit_ap(&doc.project, xf, p) {
                    let pos = doc.project.ap(id).map(|a| a.position).unwrap_or_default();
                    let offset = pos - xf.to_m(p);
                    doc.checkpoint();
                    app.canvas.drag = Some(Drag::Ap { id, offset });
                    app.selection = Selection::Ap(id);
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
                    let ap = new_ap(&doc.project, m);
                    app.selection = Selection::Ap(ap.id);
                    doc.project.aps.push(ap);
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
        model: "Generic".into(),
        position,
        height_m: 3.0,
        orientation_deg: 0.0,
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
        if selected {
            painter.line_segment([a, b], Stroke::new(7.0, Color32::from_rgb(80, 160, 255)));
        }
        painter.line_segment([a, b], Stroke::new(3.0, color));
        if selected {
            for p in [a, b] {
                painter.circle(p, 5.0, Color32::WHITE, Stroke::new(1.5, Color32::BLACK));
            }
        }
    }
}

fn draw_aps(painter: &egui::Painter, project: &Project, xf: Xf, selection: Selection, band: Band) {
    for ap in &project.aps {
        let p = xf.to_screen(ap.position);
        let selected = selection == Selection::Ap(ap.id);
        let has_band = ap.enabled_radios().any(|(_, r)| r.channel.band == band);
        let fill = if has_band {
            Color32::from_rgb(40, 110, 220)
        } else {
            Color32::from_gray(120)
        };
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
        let galley_pos = p + egui::vec2(0.0, 13.0);
        let font = FontId::proportional(12.0);
        let text_rect = painter.text(
            galley_pos,
            Align2::CENTER_TOP,
            &label,
            font.clone(),
            Color32::WHITE,
        );
        painter.rect_filled(text_rect.expand(2.0), 3.0, Color32::from_black_alpha(170));
        painter.text(galley_pos, Align2::CENTER_TOP, label, font, Color32::WHITE);
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

fn draw_legend(
    painter: &egui::Painter,
    canvas: Rect,
    kind: MetricKind,
    req: Requirements,
    visuals: &egui::Visuals,
) {
    let stops = heatmap::stops(kind);
    let width = 260.0;
    let height = 58.0;
    let rect = Rect::from_min_size(
        canvas.left_bottom() + egui::vec2(12.0, -height - 12.0),
        egui::vec2(width, height),
    );
    painter.rect(
        rect,
        6.0,
        visuals.window_fill.gamma_multiply(0.92),
        visuals.window_stroke,
        StrokeKind::Inside,
    );
    let title = match kind {
        MetricKind::Coverage => format!(
            "Coverage: ≥ {:.0} dBm and SNR ≥ {:.0} dB",
            req.min_rssi.0, req.min_snr.0
        ),
        MetricKind::ApCount => format!("APs heard at ≥ {:.0} dBm", req.min_rssi.0),
        MetricKind::ChannelOverlap => {
            format!("Overlapping radios above {:.0} dBm", req.cca_threshold.0)
        }
        _ => format!("{} ({})", kind.label(), kind.unit()),
    };
    painter.text(
        rect.min + egui::vec2(8.0, 6.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(12.0),
        visuals.text_color(),
    );
    let bar = Rect::from_min_size(
        rect.min + egui::vec2(8.0, 24.0),
        egui::vec2(width - 16.0, 12.0),
    );
    let (lo, hi) = (stops[0].0, stops[stops.len() - 1].0);
    if heatmap::is_discrete(kind) {
        let n = stops.len();
        for (i, (v, _)) in stops.iter().enumerate() {
            let x0 = bar.left() + bar.width() * i as f32 / n as f32;
            let x1 = bar.left() + bar.width() * (i + 1) as f32 / n as f32;
            let cell = Rect::from_min_max(Pos2::new(x0, bar.top()), Pos2::new(x1, bar.bottom()));
            painter.rect_filled(cell, 0.0, heatmap::color_at(kind, *v));
            let text = match kind {
                MetricKind::Coverage => if *v > 0.5 { "meets" } else { "fails" }.to_owned(),
                _ if i == n - 1 => format!("{v:.0}+"),
                _ => format!("{v:.0}"),
            };
            painter.text(
                Pos2::new(cell.center().x, bar.bottom() + 2.0),
                Align2::CENTER_TOP,
                text,
                FontId::proportional(11.0),
                visuals.text_color(),
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
            painter.rect_filled(cell, 0.0, heatmap::color_at(kind, v));
        }
        for (v, _) in stops {
            let x = bar.left() + bar.width() * (v - lo) / (hi - lo);
            painter.text(
                Pos2::new(x, bar.bottom() + 2.0),
                Align2::CENTER_TOP,
                format!("{v:.0}"),
                FontId::proportional(11.0),
                visuals.text_color(),
            );
        }
    }
}
