//! 3D view: the floor plan and heat map on the floor, walls extruded with
//! their material color and access points at their mounting height, seen
//! through an orbiting perspective camera. Other floors of the building are
//! stacked above and below, faint.
//!
//! Drawn with egui meshes (no 3D engine): points are projected on the CPU
//! and shapes are painted back to front. Walls are semi-transparent so the
//! coverage behind them stays visible.

use crate::app::OndaluxApp;
use crate::i18n::t;
use eframe::egui::{self, Color32, Mesh, PointerButton, Pos2, Rect, Sense, Shape, Stroke};
use ondalux_core::ap::{AccessPoint, Mount};
use ondalux_core::geometry::Vec2 as WVec2;
use ondalux_core::material::{MaterialLibrary, Wall};
use ondalux_core::project::FloorInfo;

/// Height walls are extruded to: the floor-to-floor height, raised to the
/// highest AP mounting height so ceiling APs never float above the walls.
pub fn floor_wall_height(info: &FloorInfo, aps: &[AccessPoint]) -> f32 {
    aps.iter().map(|a| a.height_m).fold(info.height_m, f32::max)
}

/// Wall height of the active floor.
pub fn wall_height(project: &ondalux_core::project::Project) -> f32 {
    floor_wall_height(&project.floor, &project.aps)
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
    /// Draw the other floors of the building.
    pub other_floors: bool,
}

impl Default for View3dState {
    fn default() -> Self {
        Self {
            enabled: false,
            yaw: 0.35,
            pitch: 0.85,
            distance: 0.0,
            target: None,
            other_floors: true,
        }
    }
}

impl View3dState {
    /// Back to the default camera, keeping the 3D view on.
    pub fn reset(&mut self) {
        *self = Self {
            enabled: self.enabled,
            other_floors: self.other_floors,
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
    /// A flat polygon (slab of another floor).
    Quad {
        corners: [Pos2; 4],
        fill: Color32,
        edge: Color32,
    },
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
        ghost: bool,
    },
}

/// One floor to draw: its content in its own plan coordinates, where it
/// sits in the building, and whether it is drawn faint.
struct FloorView<'a> {
    walls: &'a [Wall],
    aps: &'a [AccessPoint],
    offset: WVec2,
    base_z: f32,
    wall_h: f32,
    ghost: bool,
}

fn push_floor_items(
    items: &mut Vec<(f32, Item)>,
    cam: &Camera,
    fl: &FloorView,
    materials: &MaterialLibrary,
    band: ondalux_core::channel::Band,
) {
    let light = norm([0.45, -0.75, 0.0]);
    let (z0, z1) = (fl.base_z, fl.base_z + fl.wall_h);
    for wall in fl.walls {
        let (a, b) = (wall.segment.a + fl.offset, wall.segment.b + fl.offset);
        let len = a.distance(b);
        if len < 1e-4 {
            continue;
        }
        let c = materials
            .get(wall.material)
            .map_or([150, 150, 150, 255], |m| m.color);
        // A box as thick as the wall: two sides, the top and both ends.
        let half = wall.effective_thickness_cm(materials) * 0.005;
        let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
        let (nx, ny) = (-uy * half, ux * half);
        let a1 = [a.x + nx, a.y + ny];
        let a2 = [a.x - nx, a.y - ny];
        let b1 = [b.x + nx, b.y + ny];
        let b2 = [b.x - nx, b.y - ny];
        let lo = |p: [f32; 2]| [p[0], p[1], z0];
        let hi = |p: [f32; 2]| [p[0], p[1], z1];
        let side_n = norm([-uy, ux, 0.0]);
        let end_n = [ux, uy, 0.0];
        let faces: [([V3; 4], V3, bool); 5] = [
            ([lo(a1), lo(b1), hi(b1), hi(a1)], side_n, true),
            ([lo(a2), lo(b2), hi(b2), hi(a2)], side_n, true),
            ([hi(a1), hi(b1), hi(b2), hi(a2)], [0.0, 0.0, 1.0], false),
            ([lo(a1), lo(a2), hi(a2), hi(a1)], end_n, false),
            ([lo(b1), lo(b2), hi(b2), hi(b1)], end_n, false),
        ];
        for (pts, n, side) in faces {
            let proj: Vec<_> = pts.iter().filter_map(|p| cam.project(*p)).collect();
            if proj.len() < 4 {
                continue;
            }
            let (fill, edge) = if fl.ghost {
                if !side {
                    continue;
                }
                (
                    Color32::from_rgba_unmultiplied(150, 160, 175, 38),
                    Color32::from_rgba_unmultiplied(120, 130, 150, 90),
                )
            } else {
                // The top is lit the most.
                let shade = if n[2] > 0.5 {
                    1.0
                } else {
                    0.6 + 0.4 * dot(n, light).abs()
                };
                let rgb = |v: u8| (v as f32 * shade).round() as u8;
                (
                    Color32::from_rgba_unmultiplied(rgb(c[0]), rgb(c[1]), rgb(c[2]), 165),
                    Color32::from_rgba_unmultiplied(
                        (c[0] as f32 * 0.45) as u8,
                        (c[1] as f32 * 0.45) as u8,
                        (c[2] as f32 * 0.45) as u8,
                        230,
                    ),
                )
            };
            let mid = [
                pts.iter().map(|p| p[0]).sum::<f32>() * 0.25,
                pts.iter().map(|p| p[1]).sum::<f32>() * 0.25,
                pts.iter().map(|p| p[2]).sum::<f32>() * 0.25,
            ];
            items.push((
                cam.depth(mid),
                Item::Wall {
                    corners: [proj[0].0, proj[1].0, proj[2].0, proj[3].0],
                    fill,
                    edge,
                },
            ));
        }
    }
    for ap in fl.aps {
        let p = ap.position + fl.offset;
        let top = [p.x, p.y, z0 + ap.height_m];
        let (Some((ts, depth)), Some((fs, _))) = (cam.project(top), cam.project([p.x, p.y, z0]))
        else {
            continue;
        };
        let has_band = ap.enabled_radios().any(|(_, r)| r.channel.band == band);
        let fill = match (fl.ghost, has_band) {
            (true, _) => Color32::from_rgba_unmultiplied(110, 130, 170, 140),
            (false, true) => Color32::from_rgb(40, 110, 220),
            (false, false) => Color32::from_gray(130),
        };
        let aim = (ap.mount == Mount::Wall && !fl.ghost)
            .then(|| {
                let d = ap.direction();
                cam.project([p.x + d.x * 1.2, p.y + d.y * 1.2, top[2]])
            })
            .flatten()
            .map(|p| p.0);
        let r = (0.25 * cam.focal_px / depth).clamp(4.0, 14.0);
        items.push((
            depth,
            Item::Ap {
                top: ts,
                floor: fs,
                radius: if fl.ghost { r * 0.7 } else { r },
                label: ap.name.clone(),
                fill,
                aim,
                ghost: fl.ghost,
            },
        ));
    }
}

/// The outline of another floor's plan, as a faint slab.
fn push_slab(items: &mut Vec<(f32, Item)>, cam: &Camera, min: WVec2, max: WVec2, z: f32) {
    let pts = [
        [min.x, min.y, z],
        [max.x, min.y, z],
        [max.x, max.y, z],
        [min.x, max.y, z],
    ];
    let proj: Vec<_> = pts.iter().filter_map(|p| cam.project(*p)).collect();
    if proj.len() < 4 {
        return;
    }
    let center = [(min.x + max.x) * 0.5, (min.y + max.y) * 0.5, z];
    items.push((
        // Behind everything standing on it.
        cam.depth(center) + (max.x - min.x).max(max.y - min.y),
        Item::Quad {
            corners: [proj[0].0, proj[1].0, proj[2].0, proj[3].0],
            fill: Color32::from_rgba_unmultiplied(170, 180, 195, 22),
            edge: Color32::from_rgba_unmultiplied(120, 130, 150, 110),
        },
    ));
}

fn paint_items(painter: &egui::Painter, mut items: Vec<(f32, Item)>) {
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, item) in items {
        match item {
            Item::Quad {
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
                let stroke = Stroke::new(1.0, edge);
                for i in 0..4 {
                    painter.line_segment([corners[i], corners[(i + 1) % 4]], stroke);
                }
            }
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
                ghost,
            } => {
                let line = if ghost {
                    Color32::from_white_alpha(90)
                } else {
                    Color32::from_white_alpha(200)
                };
                painter.add(Shape::dashed_line(
                    &[floor, top],
                    Stroke::new(1.5, line),
                    4.0,
                    3.0,
                ));
                painter.circle_filled(floor, 3.0, Color32::from_black_alpha(140));
                if let Some(a) = aim {
                    painter
                        .line_segment([top, a], Stroke::new(2.5, Color32::from_rgb(255, 170, 40)));
                }
                painter.circle(top, radius, fill, Stroke::new(2.0, line));
                if ghost {
                    continue;
                }
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
    // The active floor in building coordinates.
    let fi = project.floor_index;
    let origin = project.floor.offset_m;
    let base_z = project.floor_elevation_m(fi);
    let st = &mut app.view3d;
    if st.distance <= 0.0 {
        st.distance = span * 1.25;
    }
    let home = [origin.x + w * 0.5, origin.y + h * 0.5, base_z];
    let mut target = st.target.unwrap_or(home);

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
        target = home;
    }
    // Switching floors moves the camera with them.
    if (target[2] - base_z).abs() > 1e-3 {
        target[2] = base_z;
        if st.target.is_some() {
            st.target = Some(target);
        }
    }
    let show_others = st.other_floors;
    let cam = Camera::new(st, target, rect);

    // ---- drawing ----
    let painter = ui.painter_at(rect);
    let pal = if ui.visuals().dark_mode {
        crate::style::DARK
    } else {
        crate::style::LIGHT
    };
    painter.rect_filled(rect, 0.0, pal.backdrop);
    let band = app.heat.band;

    // Floors below first: the active floor's plan covers them.
    let mut below: Vec<(f32, Item)> = Vec::new();
    let mut above: Vec<(f32, Item)> = Vec::new();
    if show_others && project.floor_count() > 1 {
        let floors = project.floors();
        for (g, floor) in floors.iter().enumerate() {
            if g == fi {
                continue;
            }
            let z = project.floor_elevation_m(g);
            let off = floor.info.offset_m;
            let (fw, fh) = floor.plan.size_m();
            let items = if g < fi { &mut below } else { &mut above };
            push_slab(items, &cam, off, off + WVec2::new(fw, fh), z);
            let view = FloorView {
                walls: &floor.walls,
                aps: &floor.aps,
                offset: off,
                base_z: z,
                wall_h: floor_wall_height(&floor.info, &floor.aps),
                ghost: true,
            };
            push_floor_items(items, &cam, &view, &project.materials, band);
        }
    }
    paint_items(&painter, below);

    // The active floor: the plan image, then the heat map on top.
    let steps = 36;
    let (tex, tint) = match &app.plan_texture {
        Some(tex) => (tex.id(), Color32::WHITE),
        None => (egui::TextureId::default(), Color32::from_gray(225)),
    };
    painter.add(Shape::mesh(floor_mesh(
        &cam,
        tex,
        origin,
        origin + WVec2::new(w, h),
        tint,
        base_z,
        steps,
    )));
    if app.heat.show {
        if let (Some(tex), Some(f)) = (&app.heat.texture, &app.heat.field) {
            let g = f.grid;
            let alpha = (app.heat.opacity * 255.0).round() as u8;
            painter.add(Shape::mesh(floor_mesh(
                &cam,
                tex.id(),
                origin + g.origin,
                origin
                    + WVec2::new(
                        g.origin.x + g.cols as f32 * g.cell_m,
                        g.origin.y + g.rows as f32 * g.cell_m,
                    ),
                Color32::from_white_alpha(alpha),
                base_z + 0.01,
                steps,
            )));
        }
    }

    // Walls and APs of the active floor and the floors above, back to front.
    let view = FloorView {
        walls: &project.walls,
        aps: &project.aps,
        offset: origin,
        base_z,
        wall_h,
        ghost: false,
    };
    push_floor_items(&mut above, &cam, &view, &project.materials, band);
    paint_items(&painter, above);

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
            other_floors: true,
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
    fn walls_reach_the_ceiling_and_the_highest_ap() {
        use ondalux_core::project::Project;
        let mut p = Project::new("t", 100, 100);
        // The floor-to-floor height (3 m by default).
        assert_eq!(wall_height(&p), 3.0);
        let ap = crate::canvas::new_ap(&p, WVec2::new(1.0, 1.0));
        for h in [4.2, 2.0] {
            let mut a = ap.clone();
            a.height_m = h;
            p.aps.push(a);
        }
        // An AP above the ceiling raises the walls with it.
        assert_eq!(wall_height(&p), 4.2);
        p.aps.retain(|a| a.height_m < 3.0);
        p.floor.height_m = 2.6;
        assert_eq!(wall_height(&p), 2.6);
    }

    #[test]
    fn points_behind_the_camera_are_rejected() {
        let st = View3dState {
            enabled: true,
            yaw: 0.0,
            pitch: 0.5,
            distance: 10.0,
            target: None,
            other_floors: true,
        };
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        let cam = Camera::new(&st, [0.0, 0.0, 0.0], rect);
        // Far behind the eye, which is south (+y) of the target.
        assert!(cam.project([0.0, 100.0, 0.0]).is_none());
    }
}
