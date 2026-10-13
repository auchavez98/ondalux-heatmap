//! Path loss models and receiver noise.
//!
//! See `docs/rf-models.md` for the derivation of each formula.

use crate::channel::{Band, ChannelWidth, PerBand};
use crate::units::{Db, Dbm};
use serde::{Deserialize, Serialize};

/// Distances below this are clamped to avoid the singularity at d = 0.
pub const MIN_DISTANCE_M: f32 = 0.1;

/// Free-space path loss (Friis), in dB.
///
/// `FSPL = 20·log10(d_m) + 20·log10(f_MHz) − 27.55`
pub fn fspl(distance_m: f32, freq_mhz: f32) -> Db {
    let d = distance_m.max(MIN_DISTANCE_M);
    Db(20.0 * d.log10() + 20.0 * freq_mhz.log10() - 27.55)
}

/// Log-distance path loss, anchored to free space at `ref_distance_m`:
///
/// `PL(d) = FSPL(d0) + 10·n·log10(d / d0)` for d ≥ d0, `FSPL(d)` otherwise.
pub fn log_distance(distance_m: f32, freq_mhz: f32, exponent: f32, ref_distance_m: f32) -> Db {
    let d = distance_m.max(MIN_DISTANCE_M);
    if d <= ref_distance_m {
        return fspl(d, freq_mhz);
    }
    Db(fspl(ref_distance_m, freq_mhz).0 + 10.0 * exponent * (d / ref_distance_m).log10())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PathLossModel {
    FreeSpace,
    LogDistance {
        /// Path loss exponent n per band (2.0 = free space).
        exponent: PerBand<f32>,
        /// Reference distance d0 in meters.
        ref_distance_m: f32,
    },
}

impl Default for PathLossModel {
    fn default() -> Self {
        // Walls are modelled explicitly, so the exponent stays close to
        // free space (multi-wall model). Raise it for cluttered spaces.
        PathLossModel::LogDistance {
            exponent: PerBand::new(2.0, 2.0, 2.0),
            ref_distance_m: 1.0,
        }
    }
}

impl PathLossModel {
    pub fn path_loss(&self, distance_m: f32, freq_mhz: f32, band: Band) -> Db {
        match self {
            PathLossModel::FreeSpace => fspl(distance_m, freq_mhz),
            PathLossModel::LogDistance {
                exponent,
                ref_distance_m,
            } => log_distance(distance_m, freq_mhz, exponent.get(band), *ref_distance_m),
        }
    }
}

/// Global settings for a prediction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropagationSettings {
    pub model: PathLossModel,
    /// Height of the client device above the floor, in meters.
    pub receiver_height_m: f32,
    /// Antenna gain of the client device, in dBi.
    pub receiver_gain: Db,
    /// Receiver noise figure, in dB.
    pub noise_figure: Db,
}

impl Default for PropagationSettings {
    fn default() -> Self {
        Self {
            model: PathLossModel::default(),
            receiver_height_m: 1.0,
            receiver_gain: Db(0.0),
            noise_figure: Db(6.0),
        }
    }
}

impl PropagationSettings {
    /// Thermal noise floor for a channel width:
    /// `N = −174 dBm/Hz + 10·log10(BW_Hz) + NF`.
    pub fn noise_floor(&self, width: ChannelWidth) -> Dbm {
        let bw_hz = width.mhz() as f32 * 1e6;
        Dbm(-174.0 + 10.0 * bw_hz.log10() + self.noise_figure.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn fspl_known_values() {
        // Reference values computed from the Friis equation.
        assert!(close(fspl(1.0, 2400.0).0, 40.05, 0.01));
        assert!(close(fspl(1.0, 5000.0).0, 46.43, 0.01));
        assert!(close(fspl(10.0, 2400.0).0, 60.05, 0.01));
        assert!(close(fspl(100.0, 5800.0).0, 87.72, 0.01));
    }

    #[test]
    fn fspl_doubling_distance_adds_6db() {
        let delta = fspl(20.0, 5180.0).0 - fspl(10.0, 5180.0).0;
        assert!(close(delta, 6.02, 0.01));
    }

    #[test]
    fn log_distance_matches_fspl_with_exponent_2() {
        for d in [0.5, 1.0, 3.0, 25.0, 80.0] {
            assert!(close(
                log_distance(d, 2437.0, 2.0, 1.0).0,
                fspl(d, 2437.0).0,
                1e-3
            ));
        }
    }

    #[test]
    fn log_distance_exponent_3() {
        // 10 m, n = 3: FSPL(1 m) + 30 dB.
        let pl = log_distance(10.0, 2400.0, 3.0, 1.0).0;
        assert!(close(pl, 70.05, 0.01));
    }

    #[test]
    fn zero_distance_is_finite() {
        assert!(fspl(0.0, 2400.0).0.is_finite());
    }

    #[test]
    fn noise_floor_20mhz() {
        let s = PropagationSettings::default();
        assert!(close(s.noise_floor(ChannelWidth::Mhz20).0, -94.99, 0.01));
        assert!(close(s.noise_floor(ChannelWidth::Mhz80).0, -88.97, 0.01));
    }
}
