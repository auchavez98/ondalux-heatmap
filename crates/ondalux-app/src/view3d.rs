//! 3D view: the floor plan and heat map on the floor, walls extruded with
//! their material color and access points at their mounting height, seen
//! through an orbiting perspective camera.
//!
//! Drawn with egui meshes (no 3D engine): points are projected on the CPU
//! and shapes are painted back to front. Walls are semi-transparent so the
//! coverage behind them stays visible.

use crate::app::OndaluxApp;
use crate::i18n::t;
use eframe::egui::{self, Color32, Mesh, PointerButton, Pos2, Rect, Sense, Shape, Stroke};
use ondalux_core::ap::Mount;
use ondalux_core::geometry::Vec2 as WVec2;

/// Wall height when the project has no access points yet, in meters.
pub const DEFAULT_WALL_HEIGHT_M: f32 = 3.0;
/// Walls are never drawn lower than this.
const MIN_WALL_HEIGHT_M: f32 = 2.5;

/// Height walls are extruded to: the ceiling, taken as the highest AP
/// mounting height (ceiling APs then sit on it), at least 2.5 m.
pub fn wall_height(project: &ondalux_core::project::Project) -> f32 {
    project
        .aps
        .iter()
        .map(|a| a.height_m)
        .fold(None, |m: Option<f32>, h| Some(m.map_or(h, |m| m.max(h))))
        .map_or(DEFAULT_WALL_HEIGHT_M, |h| h.max(MIN_WALL_HEIGHT_M))
}
const FOV_Y: f32 = 50.0;
const NEAR: f32 = 0.05;

#[derive(Debug, Clone, Copy)]
pub struct View3dState {
    pub enabled: bool,
    /// Rotation around the vertical axis, radians (0 = looking north).
    pub yaw: f32,
    /// Angle above the floor, radians.
    pub pitch: f32,
    /// Distance from the camera to the target, meters.
    pub distance: f32,
    /// Point the camera orbits around (meters); `None` = plan center.
    pub target: Option<[f32; 3]>,
}

impl Default for View3dState {
    fn default() -> Self {
        Self {
            enabled: false,
            yaw: 0.35,
            pitch: 0.85,
            distance: 0.0,
            target: None,
        }
    }
}

impl View3dState {
    /// Back to the default camera, keeping the 3D view on.
    pub fn reset(&mut self) {
        *self = Self {
            enabled: self.enabled,
            ..Self::default()
        };
    }

    /// Zoom by a factor (< 1 = closer).
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).max(1.0);
    }
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// World (x right, y down the plan, z up, meters) to screen.
struct Camera {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    focal_px: f32,
    center: Pos2,
}

impl Camera {
    fn new(st: &View3dState, target: V3, rect: Rect) -> Self {
        let (cy, sy) = (st.yaw.cos(), st.yaw.sin());
        let (cp, sp) = (st.pitch.cos(), st.pitch.sin());
        let eye = [
            target[0] + st.distance * cp * sy,
            target[1] + st.distance * cp * cy,
            target[2] + st.distance * sp,
        ];
        let fwd = norm(sub(target, eye));
        let right = norm(cross([0.0, 0.0, 1.0], fwd));
        let up = cross(fwd, right);
        let focal_px = rect.height() * 0.5 / (FOV_Y.to_radians() * 0.5).tan();
        Self {
            eye,
            right,
            up,
            fwd,
            focal_px,
            center: rect.center(),
        }
    }

    /// Screen position and depth, or `None` behind the camera.
    fn project(&self, p: V3) -> Option<(Pos2, f32)> {
        let v = sub(p, self.eye);
        let z = dot(v, self.fwd);
        if z < NEAR {
            return None;
        }
        let x = dot(v, self.right) / z * self.focal_px;
        let y = dot(v, self.up) / z * self.focal_px;
        Some((self.center + egui::vec2(x, -y), z))
    }

    fn depth(&self, p: V3) -> f32 {
        dot(sub(p, self.eye), self.fwd)
    }
}

/// A textured quad on the floor, subdivided so the perspective looks right.
#[allow(clippy::too_many_arguments)]
fn floor_mesh(
    cam: &Camera,
    texture: egui::TextureId,
    min: WVec2,
    max: WVec2,
    tint: Color32,
    z: f32,
    steps: usize,
) -> Mesh {
    let mut mesh = Mesh::with_texture(texture);
    let n = steps.max(2);
    let mut idx = vec![None; (n + 1) * (n + 1)];
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            let p = [min.x + (max.x - min.x) * u, min.y + (max.y - min.y) * v, z];
            if let Some((s, _)) = cam.project(p) {
                idx[j * (n + 1) + i] = Some(mesh.vertices.len() as u32);
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: s,
                    uv: egui::pos2(u, v),
                    color: tint,
                });
            }
        }
    }
    for j in 0..n {
        for i in 0..n {
            let a = idx[j * (n + 1) + i];
            let b = idx[j * (n + 1) + i + 1];
            let c = idx[(j + 1) * (n + 1) + i + 1];
            let d = idx[(j + 1) * (n + 1) + i];
            if let (Some(a), Some(b), Some(c), Some(d)) = (a, b, c, d) {
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(a, c, d);
            }
        }
    }
    mesh
}

enum Item {
    Wall {
        corners: [Pos2; 4],
        fill: Color32,
        edge: Color32,
    },
    Ap {
        top: Pos2,
        floor: Pos2,
        radius: f32,
        label: String,
        fill: Color32,
        aim: Option<Pos2>,
    },
}

pub fn show(app: &mut OndaluxApp, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let Some(doc) = app.doc.as_ref() else {
        return;
    };
    let project = &doc.project;
    let (w, h) = project.plan.size_m();
    let span = w.max(h).max(1.0);
    let wall_h = wall_height(project);
    let st = &mut app.view3d;
    if st.distance <= 0.0 {
        st.distance = span * 1.25;
    }
    let mut target = st.target.unwrap_or([w * 0.5, h * 0.5, 0.0]);

    // ---- interaction ----
    if response.dragged_by(PointerButton::Primary) {
        let d = response.drag_delta();
        st.yaw -= d.x * 0.008;
        st.pitch = (st.pitch + d.y * 0.006).clamp(0.12, 1.53);
    }
    if response.dragged_by(PointerButton::Secondary) || response.dragged_by(PointerButton::Middle) {
        let d = response.drag_delta();
        let cam = Camera::new(st, target, rect);
        let k = st.distance / cam.focal_px;
        // Move on the floor plane: right and "forward" projected to the floor.
        let fwd_floor = norm([cam.fwd[0], cam.fwd[1], 0.0]);
        target[0] -= (cam.right[0] * d.x - fwd_floor[0] * d.y) * k;
        target[1] -= (cam.right[1] * d.x - fwd_floor[1] * d.y) * k;
        st.target = Some(target);
    }
    if response.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        if zoom != 1.0 {
            st.zoom(1.0 / zoom);
        } else if scroll != 0.0 {
            st.zoom((-scroll * 0.0015).exp());
        }
        st.distance = st.distance.clamp(1.0, span * 6.0);
    }
    if response.double_clicked() {
        st.reset();
        st.distance = span * 1.25;
        target = [w * 0.5, h * 0.5, 0.0];
    }
    let cam = Camera::new(st, target, rect);

    // ---- drawing ----
    let painter = ui.painter_at(rect);
    let pal = if ui.visuals().dark_mode {
        crate::style::DARK
    } else {
        crate::style::LIGHT
    };
    painter.rect_filled(rect, 0.0, pal.backdrop);
    let steps = 36;
    // Floor: the plan image, then the heat map on top.
    match &app.plan_texture {
        Some(tex) => {
            painter.add(Shape::mesh(floor_mesh(
                &cam,
                tex.id(),
                WVec2::new(0.0, 0.0),
                WVec2::new(w, h),
                Color32::WHITE,
                0.0,
                steps,
            )));
        }
        None => {
            painter.add(Shape::mesh(floor_mesh(
                &cam,
                egui::TextureId::default(),
                WVec2::new(0.0, 0.0),
                WVec2::new(w, h),
                Color32::from_gray(225),
                0.0,
                steps,
            )));
        }
    }
    if app.heat.show {
        if let (Some(tex), Some(f)) = (&app.heat.texture, &app.heat.field) {
            let g = f.grid;
            let alpha = (app.heat.opacity * 255.0).round() as u8;
            painter.add(Shape::mesh(floor_mesh(
                &cam,
                tex.id(),
                g.origin,
                WVec2::new(
                    g.origin.x + g.cols as f32 * g.cell_m,
                    g.origin.y + g.rows as f32 * g.cell_m,
                ),
                Color32::from_white_alpha(alpha),
                0.01,
                steps,
            )));
        }
    }

    // Walls and APs, back to front.
    let light = norm([0.45, -0.75, 0.0]);
    let mut items: Vec<(f32, Item)> = Vec::new();
    for wall in &project.walls {
        let (a, b) = (wall.segment.a, wall.segment.b);
        let pts = [
            [a.x, a.y, 0.0],
            [b.x, b.y, 0.0],
            [b.x, b.y, wall_h],
            [a.x, a.y, wall_h],
        ];
        let proj: Vec<_> = pts.iter().filter_map(|p| cam.project(*p)).collect();
        if proj.len() < 4 {
            continue;
        }
        let c = project
            .materials
            .get(wall.material)
            .map_or([150, 150, 150, 255], |m| m.color);
        let n = norm([-(b.y - a.y), b.x - a.x, 0.0]);
        let shade = 0.6 + 0.4 * dot(n, light).abs();
        let rgb = |v: u8| (v as f32 * shade).round() as u8;
        let fill = Color32::from_rgba_unmultiplied(rgb(c[0]), rgb(c[1]), rgb(c[2]), 165);
        let edge = Color32::from_rgba_unmultiplied(
            (c[0] as f32 * 0.45) as u8,
            (c[1] as f32 * 0.45) as u8,
            (c[2] as f32 * 0.45) as u8,
            230,
        );
        let mid = [(a.x + b.x) * 0.5, (a.y + b.y) * 0.5, wall_h * 0.5];
        items.push((
            cam.depth(mid),
            Item::Wall {
                corners: [proj[0].0, proj[1].0, proj[2].0, proj[3].0],
                fill,
                edge,
            },
        ));
    }
    for ap in &project.aps {
        let top = [ap.position.x, ap.position.y, ap.height_m];
        let (Some((ts, depth)), Some((fs, _))) = (
            cam.project(top),
            cam.project([ap.position.x, ap.position.y, 0.0]),
        ) else {
            continue;
        };
        let has_band = ap
            .enabled_radios()
            .any(|(_, r)| r.channel.band == app.heat.band);
        let fill = if has_band {
            Color32::from_rgb(40, 110, 220)
        } else {
            Color32::from_gray(130)
        };
        let aim = (ap.mount == Mount::Wall)
            .then(|| {
                let d = ap.direction();
                cam.project([
                    ap.position.x + d.x * 1.2,
                    ap.position.y + d.y * 1.2,
                    ap.height_m,
                ])
            })
            .flatten()
            .map(|p| p.0);
        items.push((
            depth,
            Item::Ap {
                top: ts,
                floor: fs,
                radius: (0.25 * cam.focal_px / depth).clamp(4.0, 14.0),
                label: ap.name.clone(),
                fill,
                aim,
            },
        ));
    }
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, item) in items {
        match item {
            Item::Wall {
                corners,
                fill,
                edge,
            } => {
                let mut mesh = Mesh::default();
                for p in corners {
                    mesh.colored_vertex(p, fill);
                }
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(Shape::mesh(mesh));
                let stroke = Stroke::new(1.2, edge);
                painter.line_segment([corners[3], corners[2]], stroke);
                painter.line_segment([corners[0], corners[3]], stroke);
                painter.line_segment([corners[1], corners[2]], stroke);
                painter.line_segment([corners[0], corners[1]], Stroke::new(2.0, edge));
            }
            Item::Ap {
                top,
                floor,
                radius,
                label,
                fill,
                aim,
            } => {
                painter.add(Shape::dashed_line(
                    &[floor, top],
                    Stroke::new(1.5, Color32::from_white_alpha(200)),
                    4.0,
                    3.0,
                ));
                painter.circle_filled(floor, 3.0, Color32::from_black_alpha(140));
                if let Some(a) = aim {
                    painter
                        .line_segment([top, a], Stroke::new(2.5, Color32::from_rgb(255, 170, 40)));
                }
                painter.circle(top, radius, fill, Stroke::new(2.0, Color32::WHITE));
                let font = egui::FontId::proportional(12.0);
                let pos = top + egui::vec2(0.0, -radius - 4.0);
                let r = painter.text(
                    pos,
                    egui::Align2::CENTER_BOTTOM,
                    &label,
                    font.clone(),
                    Color32::WHITE,
                );
                painter.rect_filled(r.expand(2.0), 3.0, Color32::from_black_alpha(170));
                painter.text(
                    pos,
                    egui::Align2::CENTER_BOTTOM,
                    label,
                    font,
                    Color32::WHITE,
                );
            }
        }
    }

    if let (true, Some(field)) = (app.heat.show, &app.heat.field) {
        crate::canvas::draw_legend(
            &painter,
            rect,
            field.scale,
            field.source,
            project.requirements,
            ui.visuals(),
        );
    }
    app.canvas.hover_text = Some(
        t(
            "Vista 3D · arrastra para girar · clic derecho para mover · rueda para acercar · doble clic para reiniciar",
            "3D view · drag to rotate · right-drag to pan · wheel to zoom · double-click to reset",
        )
        .to_owned(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_keeps_plan_orientation() {
        // Looking from the south (yaw 0) at the center of a 20 × 10 m plan.
        let st = View3dState {
            enabled: true,
            yaw: 0.0,
            pitch: 0.8,
            distance: 30.0,
            target: None,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let cam = Camera::new(&st, [10.0, 5.0, 0.0], rect);
        let (left, _) = cam.project([0.0, 5.0, 0.0]).unwrap();
        let (right, _) = cam.project([20.0, 5.0, 0.0]).unwrap();
        let (far, d_far) = cam.project([10.0, 0.0, 0.0]).unwrap();
        let (near, d_near) = cam.project([10.0, 10.0, 0.0]).unwrap();
        let (high, _) = cam.project([10.0, 5.0, 3.0]).unwrap();
        // Not mirrored: +x to the right, the plan top (y = 0) farther away
        // and higher on screen, and up is up.
        assert!(left.x < right.x);
        assert!(far.y < near.y && d_far > d_near);
        assert!(high.y < cam.center.y);
        // The target is at the screen center.
        let (c, _) = cam.project([10.0, 5.0, 0.0]).unwrap();
        assert!((c - cam.center).length() < 0.5);
    }

    #[test]
    fn walls_reach_the_highest_ap() {
        use ondalux_core::project::Project;
        let mut p = Project::new("t", 100, 100);
        assert_eq!(wall_height(&p), DEFAULT_WALL_HEIGHT_M);
        let ap = crate::canvas::new_ap(&p, WVec2::new(1.0, 1.0));
        for h in [4.2, 2.0] {
            let mut a = ap.clone();
            a.height_m = h;
            p.aps.push(a);
        }
        assert_eq!(wall_height(&p), 4.2);
        p.aps.retain(|a| a.height_m < 3.0);
        // A low AP never makes the walls lower than 2.5 m.
        assert_eq!(wall_height(&p), 2.5);
    }

    #[test]
    fn points_behind_the_camera_are_rejected() {
        let st = View3dState {
            enabled: true,
            yaw: 0.0,
            pitch: 0.5,
            distance: 10.0,
            target: None,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let cam = Camera::new(&st, [0.0, 0.0, 0.0], rect);
        // Far behind the eye, which is south (+y) of the target.
        assert!(cam.project([0.0, 100.0, 0.0]).is_none());
    }
}
