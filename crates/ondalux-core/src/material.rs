//! Wall materials and walls.
//!
//! Default attenuation values are typical single-pass figures from public
//! measurement literature and vendor design guides. Real walls vary a lot
//! (thickness, moisture, rebar, coatings), so every value is user-editable.
//!
//! Solid materials (drywall, brick, concrete…) have a reference thickness:
//! a wall of another thickness scales the loss linearly (dB grow with the
//! path through a uniform material). Glass, metal, doors and similar have
//! none: their loss does not depend on the thickness entered.

use crate::channel::{Band, PerBand};
use crate::geometry::Segment;
use crate::units::Db;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MaterialId(pub u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    /// sRGBA color used to draw walls of this material.
    pub color: [u8; 4],
    /// Loss for one crossing, per band, at `thickness_cm`.
    pub attenuation: PerBand<Db>,
    /// Thickness the attenuation refers to; `None` = the loss does not
    /// depend on the wall thickness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness_cm: Option<f32>,
}

/// Wall thickness assumed when neither the wall nor its material has one.
pub const DEFAULT_WALL_THICKNESS_CM: f32 = 10.0;
/// Thickness scaling is limited to this range of factors.
const SCALE_RANGE: (f32, f32) = (0.1, 5.0);

impl Material {
    pub fn attenuation(&self, band: Band) -> Db {
        self.attenuation.get(band)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialLibrary {
    materials: Vec<Material>,
}

impl Default for MaterialLibrary {
    fn default() -> Self {
        let m = |id, name: &str, color, a24, a5, a6, t: Option<f32>| Material {
            id: MaterialId(id),
            name: name.to_owned(),
            color,
            attenuation: PerBand::new(Db(a24), Db(a5), Db(a6)),
            thickness_cm: t,
        };
        Self {
            materials: vec![
                m(
                    1,
                    "Drywall",
                    [200, 200, 200, 255],
                    3.0,
                    4.0,
                    5.0,
                    Some(12.0),
                ),
                m(2, "Wood", [160, 110, 60, 255], 4.0, 6.0, 7.0, Some(5.0)),
                m(3, "Glass", [100, 180, 230, 255], 3.0, 7.0, 8.0, None),
                m(4, "Brick", [180, 80, 60, 255], 8.0, 12.0, 14.0, Some(12.0)),
                m(
                    5,
                    "Concrete",
                    [120, 120, 120, 255],
                    12.0,
                    18.0,
                    21.0,
                    Some(20.0),
                ),
                m(6, "Metal", [70, 70, 90, 255], 25.0, 30.0, 32.0, None),
                m(
                    7,
                    "Concrete block",
                    [150, 150, 140, 255],
                    9.0,
                    12.0,
                    14.0,
                    Some(15.0),
                ),
                m(
                    8,
                    "Low-E glass",
                    [60, 140, 200, 255],
                    18.0,
                    23.0,
                    26.0,
                    None,
                ),
                m(9, "Wood door", [140, 95, 50, 255], 3.0, 4.0, 5.0, None),
                m(10, "Metal door", [90, 90, 110, 255], 15.0, 19.0, 21.0, None),
                m(
                    11,
                    "Elevator shaft",
                    [50, 50, 70, 255],
                    30.0,
                    35.0,
                    38.0,
                    None,
                ),
                m(
                    12,
                    "Cubicle partition",
                    [210, 190, 150, 255],
                    1.0,
                    2.0,
                    2.0,
                    None,
                ),
            ],
        }
    }
}

impl MaterialLibrary {
    pub fn iter(&self) -> impl Iterator<Item = &Material> {
        self.materials.iter()
    }

    pub fn get(&self, id: MaterialId) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }

    pub fn get_mut(&mut self, id: MaterialId) -> Option<&mut Material> {
        self.materials.iter_mut().find(|m| m.id == id)
    }

    /// Adds a new material and returns its id.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        color: [u8; 4],
        attenuation: PerBand<Db>,
    ) -> MaterialId {
        let id = MaterialId(self.materials.iter().map(|m| m.id.0).max().unwrap_or(0) + 1);
        self.materials.push(Material {
            id,
            name: name.into(),
            color,
            attenuation,
            thickness_cm: None,
        });
        id
    }

    /// Removes a material. Returns false if it does not exist.
    /// Callers must make sure no wall still references it.
    pub fn remove(&mut self, id: MaterialId) -> bool {
        let before = self.materials.len();
        self.materials.retain(|m| m.id != id);
        self.materials.len() != before
    }

    /// Gives the built-in materials (matched by name) their reference
    /// thickness, for libraries saved before thickness existed. Their walls
    /// have no thickness yet, so predictions do not change.
    pub fn fill_reference_thickness(&mut self) {
        let defaults = Self::default();
        for m in &mut self.materials {
            if m.thickness_cm.is_none() {
                m.thickness_cm = defaults
                    .iter()
                    .find(|d| d.name == m.name)
                    .and_then(|d| d.thickness_cm);
            }
        }
    }

    /// Attenuation for one crossing; unknown materials count as 0 dB.
    pub fn attenuation(&self, id: MaterialId, band: Band) -> Db {
        self.get(id).map_or(Db(0.0), |m| m.attenuation(band))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub segment: Segment,
    pub material: MaterialId,
    /// Thickness in cm; `None` = the material's reference thickness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness_cm: Option<f32>,
}

impl Wall {
    pub fn new(segment: Segment, material: MaterialId) -> Self {
        Self {
            segment,
            material,
            thickness_cm: None,
        }
    }

    /// Thickness to draw: the wall's, else its material's reference, else
    /// [`DEFAULT_WALL_THICKNESS_CM`].
    pub fn effective_thickness_cm(&self, lib: &MaterialLibrary) -> f32 {
        self.thickness_cm
            .or_else(|| lib.get(self.material).and_then(|m| m.thickness_cm))
            .unwrap_or(DEFAULT_WALL_THICKNESS_CM)
    }

    /// Loss for one crossing: the material's, scaled by thickness when
    /// both the wall and the material have one.
    pub fn attenuation(&self, lib: &MaterialLibrary) -> PerBand<Db> {
        let Some(m) = lib.get(self.material) else {
            return PerBand::default();
        };
        let k = match (self.thickness_cm, m.thickness_cm) {
            (Some(t), Some(r)) if r > 0.0 => (t / r).clamp(SCALE_RANGE.0, SCALE_RANGE.1),
            _ => 1.0,
        };
        let a = m.attenuation;
        PerBand::new(Db(a.g2_4.0 * k), Db(a.g5.0 * k), Db(a.g6.0 * k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_increase_with_frequency() {
        let lib = MaterialLibrary::default();
        for m in lib.iter() {
            let a = m.attenuation;
            assert!(a.g2_4.0 <= a.g5.0 && a.g5.0 <= a.g6.0, "{}", m.name);
        }
    }

    #[test]
    fn thickness_scales_solid_materials_only() {
        let lib = MaterialLibrary::default();
        let seg = Segment::new(
            crate::geometry::Vec2::new(0.0, 0.0),
            crate::geometry::Vec2::new(1.0, 0.0),
        );
        let mut concrete = Wall::new(seg, MaterialId(5));
        assert_eq!(concrete.attenuation(&lib).g5, Db(18.0));
        assert_eq!(concrete.effective_thickness_cm(&lib), 20.0);
        concrete.thickness_cm = Some(30.0);
        assert!((concrete.attenuation(&lib).g5.0 - 27.0).abs() < 1e-4);
        // Glass: the loss stays, the thickness is only drawn.
        let mut glass = Wall::new(seg, MaterialId(3));
        assert_eq!(
            glass.effective_thickness_cm(&lib),
            DEFAULT_WALL_THICKNESS_CM
        );
        glass.thickness_cm = Some(2.0);
        assert_eq!(glass.attenuation(&lib).g5, Db(7.0));
        assert_eq!(glass.effective_thickness_cm(&lib), 2.0);
    }

    #[test]
    fn add_and_remove() {
        let mut lib = MaterialLibrary::default();
        let id = lib.add(
            "Elevator",
            [0, 0, 0, 255],
            PerBand::new(Db(30.0), Db(35.0), Db(35.0)),
        );
        assert_eq!(lib.attenuation(id, Band::G5), Db(35.0));
        assert!(lib.remove(id));
        assert_eq!(lib.attenuation(id, Band::G5), Db(0.0));
    }
}
