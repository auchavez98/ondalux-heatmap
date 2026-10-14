//! Wall materials and walls.
//!
//! Default attenuation values are typical single-pass figures from public
//! measurement literature and vendor design guides. Real walls vary a lot
//! (thickness, moisture, rebar, coatings), so every value is user-editable.

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
    /// Loss for one crossing, per band.
    pub attenuation: PerBand<Db>,
}

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
        let m = |id, name: &str, color, a24, a5, a6| Material {
            id: MaterialId(id),
            name: name.to_owned(),
            color,
            attenuation: PerBand::new(Db(a24), Db(a5), Db(a6)),
        };
        Self {
            materials: vec![
                m(1, "Drywall", [200, 200, 200, 255], 3.0, 4.0, 5.0),
                m(2, "Wood", [160, 110, 60, 255], 4.0, 6.0, 7.0),
                m(3, "Glass", [100, 180, 230, 255], 3.0, 7.0, 8.0),
                m(4, "Brick", [180, 80, 60, 255], 8.0, 12.0, 14.0),
                m(5, "Concrete", [120, 120, 120, 255], 12.0, 18.0, 21.0),
                m(6, "Metal", [70, 70, 90, 255], 25.0, 30.0, 32.0),
                m(7, "Concrete block", [150, 150, 140, 255], 9.0, 12.0, 14.0),
                m(8, "Low-E glass", [60, 140, 200, 255], 18.0, 23.0, 26.0),
                m(9, "Wood door", [140, 95, 50, 255], 3.0, 4.0, 5.0),
                m(10, "Metal door", [90, 90, 110, 255], 15.0, 19.0, 21.0),
                m(11, "Elevator shaft", [50, 50, 70, 255], 30.0, 35.0, 38.0),
                m(12, "Cubicle partition", [210, 190, 150, 255], 1.0, 2.0, 2.0),
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

    /// Attenuation for one crossing; unknown materials count as 0 dB.
    pub fn attenuation(&self, id: MaterialId, band: Band) -> Db {
        self.get(id).map_or(Db(0.0), |m| m.attenuation(band))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    pub segment: Segment,
    pub material: MaterialId,
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
