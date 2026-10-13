//! Strongly typed RF units, so dBm and dB cannot be mixed by accident.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Neg, Sub};

/// Absolute power level in dBm.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dbm(pub f32);

/// Relative power ratio (gain or loss) in dB.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Db(pub f32);

impl Dbm {
    /// Value used for "no signal" cells. Far below any real noise floor.
    pub const FLOOR: Dbm = Dbm(-150.0);

    pub fn to_milliwatts(self) -> f32 {
        10f32.powf(self.0 / 10.0)
    }

    pub fn from_milliwatts(mw: f32) -> Dbm {
        if mw <= 0.0 {
            Dbm::FLOOR
        } else {
            Dbm((10.0 * mw.log10()).max(Dbm::FLOOR.0))
        }
    }

    /// Sum of several incoherent signals (added in linear power).
    pub fn power_sum(levels: impl IntoIterator<Item = Dbm>) -> Dbm {
        Dbm::from_milliwatts(levels.into_iter().map(Dbm::to_milliwatts).sum())
    }
}

impl Add<Db> for Dbm {
    type Output = Dbm;
    fn add(self, rhs: Db) -> Dbm {
        Dbm(self.0 + rhs.0)
    }
}

impl Sub<Db> for Dbm {
    type Output = Dbm;
    fn sub(self, rhs: Db) -> Dbm {
        Dbm(self.0 - rhs.0)
    }
}

/// Difference between two absolute levels is a ratio (e.g. SNR).
impl Sub<Dbm> for Dbm {
    type Output = Db;
    fn sub(self, rhs: Dbm) -> Db {
        Db(self.0 - rhs.0)
    }
}

impl Add for Db {
    type Output = Db;
    fn add(self, rhs: Db) -> Db {
        Db(self.0 + rhs.0)
    }
}

impl AddAssign for Db {
    fn add_assign(&mut self, rhs: Db) {
        self.0 += rhs.0;
    }
}

impl Neg for Db {
    type Output = Db;
    fn neg(self) -> Db {
        Db(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milliwatt_round_trip() {
        assert!((Dbm(0.0).to_milliwatts() - 1.0).abs() < 1e-6);
        assert!((Dbm(20.0).to_milliwatts() - 100.0).abs() < 1e-3);
        assert!((Dbm::from_milliwatts(100.0).0 - 20.0).abs() < 1e-5);
    }

    #[test]
    fn two_equal_signals_add_3db() {
        let s = Dbm::power_sum([Dbm(-70.0), Dbm(-70.0)]);
        assert!((s.0 - (-66.99)).abs() < 0.01, "{s:?}");
    }

    #[test]
    fn zero_power_clamps_to_floor() {
        assert_eq!(Dbm::from_milliwatts(0.0), Dbm::FLOOR);
        assert_eq!(Dbm::power_sum([]), Dbm::FLOOR);
    }

    #[test]
    fn arithmetic() {
        assert_eq!(Dbm(20.0) - Db(3.0), Dbm(17.0));
        assert_eq!(Dbm(-60.0) - Dbm(-90.0), Db(30.0));
    }
}
