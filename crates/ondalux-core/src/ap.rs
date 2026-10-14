//! Access points and their radios.

use crate::channel::Channel;
use crate::geometry::Vec2;
use crate::units::{Db, Dbm};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApId(pub u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Radio {
    pub channel: Channel,
    /// Conducted transmit power at the antenna port.
    pub tx_power: Dbm,
    /// Antenna gain in dBi.
    pub antenna_gain: Db,
    pub enabled: bool,
}

impl Radio {
    /// Effective isotropic radiated power.
    pub fn eirp(&self) -> Dbm {
        self.tx_power + self.antenna_gain
    }
}

/// Where the AP is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mount {
    /// On the ceiling, radiating evenly around (the usual indoor install).
    #[default]
    Ceiling,
    /// On a wall, radiating mostly away from it.
    Wall,
}

/// Horizontal antenna pattern relative to the antenna gain.
///
/// Simplified 2D model (the parabolic main-lobe approximation used in
/// 3GPP TR 38.901): loss(φ) = min(12·(φ / beamwidth)², front_to_back),
/// with φ the angle off the pointing direction. It ignores side lobes and
/// the vertical pattern, so treat it as a first approximation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AntennaPattern {
    /// Horizontal half-power (−3 dB) beamwidth in degrees; 360 = omni.
    pub beamwidth_deg: f32,
    /// Maximum loss behind the antenna, in dB.
    pub front_to_back_db: f32,
}

impl Default for AntennaPattern {
    fn default() -> Self {
        Self::OMNI
    }
}

impl AntennaPattern {
    pub const OMNI: Self = Self {
        beamwidth_deg: 360.0,
        front_to_back_db: 0.0,
    };
    /// Typical wall-mounted AP: radiates into the room, the wall side is
    /// attenuated.
    pub const WALL: Self = Self {
        beamwidth_deg: 180.0,
        front_to_back_db: 10.0,
    };
    /// Patch / panel antenna.
    pub const DIRECTIONAL: Self = Self {
        beamwidth_deg: 65.0,
        front_to_back_db: 20.0,
    };

    pub fn is_omni(&self) -> bool {
        self.beamwidth_deg >= 360.0 || self.front_to_back_db <= 0.0
    }

    /// Loss (≥ 0 dB) at `off_axis_deg` degrees from the pointing direction.
    pub fn loss(&self, off_axis_deg: f32) -> Db {
        if self.is_omni() {
            return Db(0.0);
        }
        let r = off_axis_deg.abs().min(180.0) / self.beamwidth_deg.max(1.0);
        Db((12.0 * r * r).min(self.front_to_back_db))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessPoint {
    pub id: ApId,
    pub name: String,
    /// Vendor of the model, empty for generic APs. Older files load with
    /// an empty vendor.
    #[serde(default)]
    pub vendor: String,
    /// Model name: from the AP catalog or free text.
    pub model: String,
    /// Position on the floor plan, in meters.
    pub position: Vec2,
    /// Mounting height above the floor, in meters.
    pub height_m: f32,
    /// Pointing direction in degrees on the plan: 0 = +x (right), 90 = +y
    /// (down on the plan, so clockwise on screen). Only matters when the
    /// pattern is not omnidirectional.
    pub orientation_deg: f32,
    #[serde(default)]
    pub mount: Mount,
    /// Older files (before mounting existed) load as omnidirectional.
    #[serde(default)]
    pub pattern: AntennaPattern,
    pub radios: Vec<Radio>,
}

impl AccessPoint {
    pub fn enabled_radios(&self) -> impl Iterator<Item = (usize, &Radio)> {
        self.radios.iter().enumerate().filter(|(_, r)| r.enabled)
    }

    /// Unit vector of the pointing direction on the plan.
    pub fn direction(&self) -> Vec2 {
        let a = self.orientation_deg.to_radians();
        Vec2::new(a.cos(), a.sin())
    }

    /// Pattern that is actually used: ceiling APs are always treated as
    /// omnidirectional, the pointing direction only matters on a wall.
    pub fn effective_pattern(&self) -> AntennaPattern {
        match self.mount {
            Mount::Ceiling => AntennaPattern::OMNI,
            Mount::Wall => self.pattern,
        }
    }

    /// Pattern loss towards `point` (0 for ceiling or omni APs, and at the AP).
    pub fn pattern_loss(&self, point: Vec2) -> Db {
        let pattern = self.effective_pattern();
        if pattern.is_omni() {
            return Db(0.0);
        }
        let v = Vec2::new(point.x - self.position.x, point.y - self.position.y);
        let len = v.length();
        if len < 1e-4 {
            return Db(0.0);
        }
        let d = self.direction();
        let cos = ((v.x * d.x + v.y * d.y) / len).clamp(-1.0, 1.0);
        pattern.loss(cos.acos().to_degrees())
    }

    /// Changes the mount and switches to its usual pattern.
    pub fn set_mount(&mut self, mount: Mount) {
        self.mount = mount;
        self.pattern = match mount {
            Mount::Ceiling => AntennaPattern::OMNI,
            Mount::Wall => AntennaPattern::WALL,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(orientation_deg: f32, pattern: AntennaPattern) -> AccessPoint {
        AccessPoint {
            id: ApId(1),
            name: "AP".into(),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(0.0, 0.0),
            height_m: 3.0,
            orientation_deg,
            mount: Mount::Wall,
            pattern,
            radios: Vec::new(),
        }
    }

    #[test]
    fn omni_has_no_pattern_loss() {
        let a = ap(0.0, AntennaPattern::OMNI);
        assert_eq!(a.pattern_loss(Vec2::new(-5.0, 1.0)), Db(0.0));
    }

    #[test]
    fn half_power_at_half_beamwidth_and_capped_behind() {
        let p = AntennaPattern::WALL;
        assert!((p.loss(90.0).0 - 3.0).abs() < 1e-4);
        assert_eq!(p.loss(0.0), Db(0.0));
        assert_eq!(p.loss(180.0), Db(p.front_to_back_db));
    }

    #[test]
    fn pattern_follows_orientation() {
        // Pointing down the plan (+y).
        let a = ap(90.0, AntennaPattern::DIRECTIONAL);
        assert!(a.pattern_loss(Vec2::new(0.0, 5.0)).0 < 1e-3);
        assert_eq!(a.pattern_loss(Vec2::new(0.0, -5.0)), Db(20.0));
        let side = a.pattern_loss(Vec2::new(5.0, 0.0)).0;
        assert!(side > 3.0 && side <= 20.0);
    }

    #[test]
    fn ceiling_ignores_direction() {
        let mut a = ap(90.0, AntennaPattern::DIRECTIONAL);
        a.mount = Mount::Ceiling;
        assert_eq!(a.pattern_loss(Vec2::new(0.0, -5.0)), Db(0.0));
    }

    #[test]
    fn set_mount_picks_the_usual_pattern() {
        let mut a = ap(0.0, AntennaPattern::OMNI);
        a.set_mount(Mount::Wall);
        assert_eq!(a.pattern, AntennaPattern::WALL);
        a.set_mount(Mount::Ceiling);
        assert!(a.pattern.is_omni());
    }

    #[test]
    fn old_files_load_as_ceiling_omni() {
        let json = r#"{"id":1,"name":"AP","model":"","position":{"x":0,"y":0},
            "height_m":3,"orientation_deg":0,"radios":[]}"#;
        let a: AccessPoint = serde_json::from_str(json).unwrap();
        assert_eq!(a.mount, Mount::Ceiling);
        assert!(a.pattern.is_omni());
    }
}
