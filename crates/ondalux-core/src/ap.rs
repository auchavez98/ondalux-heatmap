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
    /// Antenna azimuth in degrees (0 = +x, counter-clockwise).
    ///
    /// STUB: stored and editable, but the engine only models omnidirectional
    /// antennas, so it has no effect on predictions yet.
    pub orientation_deg: f32,
    pub radios: Vec<Radio>,
}

impl AccessPoint {
    pub fn enabled_radios(&self) -> impl Iterator<Item = (usize, &Radio)> {
        self.radios.iter().enumerate().filter(|(_, r)| r.enabled)
    }
}
