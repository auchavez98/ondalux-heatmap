//! SIMULATED scanner: derives readings from the prediction engine plus
//! random shadow fading. For demos, training and testing the survey workflow
//! without a WiFi adapter. Surveys made with it are labelled "Simulated".

use crate::{ScanError, WifiScanner};
use ondalux_core::ap::AccessPoint;
use ondalux_core::geometry::Vec2;
use ondalux_core::material::{MaterialLibrary, Wall};
use ondalux_core::predict::{rssi_at, Obstacles};
use ondalux_core::propagation::PropagationSettings;
use ondalux_core::survey::{BssReading, Security};
use ondalux_core::units::Dbm;

pub const NAME: &str = "Simulated (prediction + random fading)";
/// Readings below this level are not reported, like a real receiver.
const SENSITIVITY: f32 = -92.0;

pub struct SimulatedScanner {
    aps: Vec<AccessPoint>,
    obstacles: Obstacles,
    settings: PropagationSettings,
    /// Standard deviation of log-normal shadow fading, in dB.
    pub fading_db: f32,
    pub ssid: String,
    rng: u64,
}

impl SimulatedScanner {
    pub fn new(
        aps: Vec<AccessPoint>,
        walls: &[Wall],
        materials: &MaterialLibrary,
        settings: PropagationSettings,
        seed: u64,
    ) -> Self {
        Self {
            aps,
            obstacles: Obstacles::new(walls, materials),
            settings,
            fading_db: 4.0,
            ssid: "Ondalux-Sim".into(),
            rng: seed | 1,
        }
    }

    /// Standard normal sample (Box–Muller over a 64-bit LCG).
    fn gaussian(&mut self) -> f32 {
        let mut uniform = || {
            self.rng = self
                .rng
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.rng >> 11) as f64 / (1u64 << 53) as f64).max(1e-12)
        };
        let (u1, u2) = (uniform(), uniform());
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

impl WifiScanner for SimulatedScanner {
    fn name(&self) -> &str {
        NAME
    }

    fn scan(&mut self, at: Vec2) -> Result<Vec<BssReading>, ScanError> {
        let mut scratch = self.obstacles.scratch();
        let mut out = Vec::new();
        for ap_i in 0..self.aps.len() {
            for radio_i in 0..self.aps[ap_i].radios.len() {
                let (ap, radio) = (&self.aps[ap_i], &self.aps[ap_i].radios[radio_i]);
                if !radio.enabled {
                    continue;
                }
                let rssi = rssi_at(ap, radio, at, &self.obstacles, &self.settings, &mut scratch).0;
                let frequency_mhz = radio.channel.primary_center_mhz() as u32;
                let id = ap.id.0;
                let fading = self.gaussian() * self.fading_db;
                let rssi = (rssi + fading).round();
                if rssi < SENSITIVITY {
                    continue;
                }
                out.push(BssReading {
                    // Locally administered address derived from AP id and radio.
                    bssid: format!(
                        "02:00:{:02x}:{:02x}:{:02x}:{:02x}",
                        (id >> 16) & 0xff,
                        (id >> 8) & 0xff,
                        id & 0xff,
                        radio_i * 16
                    ),
                    ssid: self.ssid.clone(),
                    rssi: Dbm(rssi.min(-20.0)),
                    frequency_mhz,
                    security: Security::Wpa2Personal,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::ap::{ApId, Radio};
    use ondalux_core::channel::{Band, Channel, ChannelWidth};
    use ondalux_core::units::Db;

    fn ap() -> AccessPoint {
        AccessPoint {
            id: ApId(7),
            name: "AP".into(),
            vendor: String::new(),
            model: String::new(),
            position: Vec2::new(0.0, 0.0),
            height_m: 1.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![
                Radio {
                    channel: Channel::new(Band::G2_4, 6, ChannelWidth::Mhz20).unwrap(),
                    tx_power: Dbm(20.0),
                    antenna_gain: Db(0.0),
                    enabled: true,
                },
                Radio {
                    channel: Channel::new(Band::G5, 36, ChannelWidth::Mhz40).unwrap(),
                    tx_power: Dbm(20.0),
                    antenna_gain: Db(0.0),
                    enabled: false,
                },
            ],
        }
    }

    #[test]
    fn readings_follow_prediction_with_noise() {
        let mut s = SimulatedScanner::new(
            vec![ap()],
            &[],
            &MaterialLibrary::default(),
            PropagationSettings::default(),
            42,
        );
        s.fading_db = 0.0;
        let r = s.scan(Vec2::new(10.0, 0.0)).unwrap();
        assert_eq!(r.len(), 1, "disabled radio must not appear");
        assert_eq!(r[0].frequency_mhz, 2437);
        assert_eq!(r[0].bssid, "02:00:00:00:07:00");
        // 20 dBm − FSPL(10 m, 2437 MHz) ≈ 20 − 60.2
        assert!((r[0].rssi.0 + 40.0).abs() < 1.0, "{:?}", r[0].rssi);

        s.fading_db = 4.0;
        let samples: Vec<f32> = (0..400)
            .map(|_| s.scan(Vec2::new(10.0, 0.0)).unwrap()[0].rssi.0)
            .collect();
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let sd =
            (samples.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / samples.len() as f32).sqrt();
        assert!((mean + 40.0).abs() < 1.0, "mean {mean}");
        assert!((sd - 4.0).abs() < 0.8, "sd {sd}");
    }

    #[test]
    fn far_away_radios_are_not_heard() {
        let mut s = SimulatedScanner::new(
            vec![ap()],
            &[],
            &MaterialLibrary::default(),
            PropagationSettings::default(),
            1,
        );
        s.fading_db = 0.0;
        assert!(s.scan(Vec2::new(5000.0, 0.0)).unwrap().is_empty());
    }
}
