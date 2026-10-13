//! The project data model: floor plan calibration, materials, walls, APs
//! and prediction settings. Serialized by `heatcraft-io`.

use crate::ap::{AccessPoint, ApId};
use crate::geometry::Vec2;
use crate::material::{MaterialId, MaterialLibrary, Wall};
use crate::metrics::Requirements;
use crate::propagation::PropagationSettings;
use serde::{Deserialize, Serialize};

/// Scale used until the user calibrates the plan.
pub const DEFAULT_PX_PER_M: f32 = 20.0;

/// Two points on the image (in pixels) and the real distance between them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub a_px: Vec2,
    pub b_px: Vec2,
    pub distance_m: f32,
}

impl Calibration {
    /// Returns `None` if the points coincide or the distance is not positive.
    pub fn new(a_px: Vec2, b_px: Vec2, distance_m: f32) -> Option<Self> {
        let valid = distance_m > 0.0 && distance_m.is_finite() && a_px.distance(b_px) > 1.0;
        valid.then_some(Self {
            a_px,
            b_px,
            distance_m,
        })
    }

    pub fn px_per_m(&self) -> f32 {
        self.a_px.distance(self.b_px) / self.distance_m
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorPlan {
    pub width_px: u32,
    pub height_px: u32,
    pub calibration: Option<Calibration>,
}

impl FloorPlan {
    pub fn px_per_m(&self) -> f32 {
        self.calibration.map_or(DEFAULT_PX_PER_M, |c| c.px_per_m())
    }

    pub fn size_m(&self) -> (f32, f32) {
        let s = self.px_per_m();
        (self.width_px as f32 / s, self.height_px as f32 / s)
    }

    pub fn px_to_m(&self, px: Vec2) -> Vec2 {
        px * (1.0 / self.px_per_m())
    }

    pub fn m_to_px(&self, m: Vec2) -> Vec2 {
        m * self.px_per_m()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub plan: FloorPlan,
    pub materials: MaterialLibrary,
    pub walls: Vec<Wall>,
    pub aps: Vec<AccessPoint>,
    pub settings: PropagationSettings,
    pub requirements: Requirements,
    /// Heat map sampling step, in meters.
    pub grid_cell_m: f32,
}

impl Project {
    pub fn new(name: impl Into<String>, width_px: u32, height_px: u32) -> Self {
        Self {
            name: name.into(),
            plan: FloorPlan {
                width_px,
                height_px,
                calibration: None,
            },
            materials: MaterialLibrary::default(),
            walls: Vec::new(),
            aps: Vec::new(),
            settings: PropagationSettings::default(),
            requirements: Requirements::default(),
            grid_cell_m: 0.25,
        }
    }

    /// Changes the plan calibration. Walls and APs keep their position on
    /// the image, so their coordinates in meters are rescaled.
    pub fn set_calibration(&mut self, calibration: Option<Calibration>) {
        let old = self.plan.px_per_m();
        self.plan.calibration = calibration;
        let k = old / self.plan.px_per_m();
        if (k - 1.0).abs() < f32::EPSILON {
            return;
        }
        for w in &mut self.walls {
            w.segment.a = w.segment.a * k;
            w.segment.b = w.segment.b * k;
        }
        for ap in &mut self.aps {
            ap.position = ap.position * k;
        }
    }

    pub fn next_ap_id(&self) -> ApId {
        ApId(self.aps.iter().map(|a| a.id.0).max().map_or(1, |m| m + 1))
    }

    pub fn ap(&self, id: ApId) -> Option<&AccessPoint> {
        self.aps.iter().find(|a| a.id == id)
    }

    pub fn ap_mut(&mut self, id: ApId) -> Option<&mut AccessPoint> {
        self.aps.iter_mut().find(|a| a.id == id)
    }

    /// True if any wall uses the material.
    pub fn material_in_use(&self, id: MaterialId) -> bool {
        self.walls.iter().any(|w| w.material == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Segment;

    #[test]
    fn calibration_scale() {
        let c = Calibration::new(Vec2::new(100.0, 100.0), Vec2::new(400.0, 500.0), 10.0).unwrap();
        assert!((c.px_per_m() - 50.0).abs() < 1e-4);
        assert!(Calibration::new(Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0), 5.0).is_none());
        assert!(Calibration::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), 0.0).is_none());
    }

    #[test]
    fn recalibration_keeps_pixel_positions() {
        let mut p = Project::new("t", 1000, 800);
        p.walls.push(Wall {
            segment: Segment::new(Vec2::new(1.0, 1.0), Vec2::new(5.0, 1.0)),
            material: MaterialId(1),
        });
        let px_before = p.plan.m_to_px(p.walls[0].segment.b);
        p.set_calibration(Calibration::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(500.0, 0.0),
            10.0,
        ));
        let px_after = p.plan.m_to_px(p.walls[0].segment.b);
        assert!(px_before.distance(px_after) < 1e-3);
        assert!((p.walls[0].segment.b.x - 2.0).abs() < 1e-4); // 100 px at 50 px/m
    }

    #[test]
    fn plan_size_in_meters() {
        let p = Project::new("t", 2000, 1000);
        assert_eq!(p.plan.size_m(), (100.0, 50.0));
    }
}
