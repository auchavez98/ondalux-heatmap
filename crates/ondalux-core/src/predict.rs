//! Point-to-point signal prediction: path loss plus 2D wall ray casting.

use crate::ap::{AccessPoint, Radio};
use crate::channel::{Band, PerBand};
use crate::geometry::{QueryScratch, SegmentIndex, Vec2};
use crate::material::{MaterialLibrary, Wall};
use crate::propagation::PropagationSettings;
use crate::units::{Db, Dbm};

/// Spatial index cell size, in meters.
const INDEX_CELL_M: f32 = 1.5;

/// Walls prepared for fast queries. Rebuild whenever walls or materials change.
#[derive(Debug, Clone)]
pub struct Obstacles {
    index: SegmentIndex,
    /// Loss of each wall, same order as the index segments.
    loss: Vec<PerBand<Db>>,
}

impl Obstacles {
    pub fn new(walls: &[Wall], materials: &MaterialLibrary) -> Self {
        let index = SegmentIndex::build(walls.iter().map(|w| w.segment).collect(), INDEX_CELL_M);
        let loss = walls.iter().map(|w| w.attenuation(materials)).collect();
        Self { index, loss }
    }

    pub fn scratch(&self) -> QueryScratch {
        self.index.new_scratch()
    }

    /// Total loss of all walls crossed by the straight path `a→b`.
    /// Each wall counts once, regardless of the incidence angle.
    pub fn loss_between(&self, a: Vec2, b: Vec2, band: Band, scratch: &mut QueryScratch) -> Db {
        let mut total = Db(0.0);
        self.index.for_each_crossing(a, b, scratch, |id| {
            total += self.loss[id as usize].get(band);
        });
        total
    }
}

/// Received signal from one radio at one point, including the AP's
/// horizontal antenna pattern.
pub fn rssi_at(
    ap: &AccessPoint,
    radio: &Radio,
    point: Vec2,
    obstacles: &Obstacles,
    settings: &PropagationSettings,
    scratch: &mut QueryScratch,
) -> Dbm {
    let band = radio.channel.band;
    let horizontal = ap.position.distance(point);
    let dh = ap.height_m - settings.receiver_height_m;
    let distance = horizontal.hypot(dh);
    let freq = radio.channel.range().center_mhz();
    let path = settings.model.path_loss(distance, freq, band);
    let walls = obstacles.loss_between(ap.position, point, band, scratch);
    radio.eirp() + settings.receiver_gain - path - walls - ap.pattern_loss(point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ap::ApId;
    use crate::channel::{Channel, ChannelWidth};
    use crate::geometry::Segment;
    use crate::material::MaterialId;
    use crate::propagation::{fspl, PathLossModel};

    fn ap_at(x: f32, y: f32, band: Band, ch: u16) -> AccessPoint {
        AccessPoint {
            id: ApId(1),
            name: "AP".into(),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(x, y),
            height_m: 1.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![Radio {
                channel: Channel::new(band, ch, ChannelWidth::Mhz20).unwrap(),
                tx_power: Dbm(20.0),
                antenna_gain: Db(3.0),
                enabled: true,
            }],
        }
    }

    fn free_space() -> PropagationSettings {
        PropagationSettings {
            model: PathLossModel::FreeSpace,
            ..Default::default()
        }
    }

    fn wall(x: f32, material: u32) -> Wall {
        Wall {
            segment: Segment::new(Vec2::new(x, -10.0), Vec2::new(x, 10.0)),
            material: MaterialId(material),
            thickness_cm: None,
        }
    }

    #[test]
    fn open_space_equals_eirp_minus_fspl() {
        let ap = ap_at(0.0, 0.0, Band::G2_4, 6);
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let mut s = obs.scratch();
        let rssi = rssi_at(
            &ap,
            &ap.radios[0],
            Vec2::new(10.0, 0.0),
            &obs,
            &free_space(),
            &mut s,
        );
        let expected = 23.0 - fspl(10.0, 2437.0).0;
        assert!((rssi.0 - expected).abs() < 1e-3, "{rssi:?} vs {expected}");
    }

    #[test]
    fn each_crossed_wall_subtracts_its_loss() {
        let lib = MaterialLibrary::default();
        let ap = ap_at(0.0, 0.0, Band::G5, 36);
        let p = Vec2::new(10.0, 0.0);
        let settings = free_space();
        let open = Obstacles::new(&[], &lib);
        let walls = Obstacles::new(&[wall(3.0, 1), wall(6.0, 5), wall(20.0, 6)], &lib);
        let mut s = walls.scratch();
        let base = rssi_at(&ap, &ap.radios[0], p, &open, &settings, &mut open.scratch());
        let blocked = rssi_at(&ap, &ap.radios[0], p, &walls, &settings, &mut s);
        // Drywall 4 dB + concrete 18 dB at 5 GHz; the metal wall at x = 20 is not crossed.
        assert!(((base - blocked).0 - 22.0).abs() < 1e-3);
    }

    #[test]
    fn height_difference_adds_distance() {
        let lib = MaterialLibrary::default();
        let obs = Obstacles::new(&[], &lib);
        let mut ap = ap_at(0.0, 0.0, Band::G2_4, 1);
        ap.height_m = 4.0; // 3 m above a 1 m receiver
        let rssi = rssi_at(
            &ap,
            &ap.radios[0],
            Vec2::new(4.0, 0.0),
            &obs,
            &free_space(),
            &mut obs.scratch(),
        );
        let expected = 23.0 - fspl(5.0, 2412.0).0;
        assert!((rssi.0 - expected).abs() < 1e-3);
    }
}
