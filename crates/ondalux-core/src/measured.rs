//! Measured heat maps: survey readings interpolated onto the grid as one
//! RSSI layer per radio, so [`crate::metrics::Metrics`] can derive the same
//! metrics (signal, SNR, SINR, AP count, overlap) as for predictions.

use crate::ap::ApId;
use crate::channel::{Band, Channel, ChannelWidth};
use crate::grid::{GridSpec, RadioKey, RadioLayer};
use crate::interpolate::{
    empirical_variogram, fit_variogram, InterpolationMethod, InterpolationSettings, Variogram,
    WeightGrid,
};
use crate::survey::{BssReading, Measurement, Survey};
use crate::units::Dbm;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasuredSettings {
    pub interpolation: InterpolationSettings,
    /// Only these SSIDs are used. Empty = every network heard.
    pub ssids: Vec<String>,
    /// Value assumed where a radio was not heard (detection floor).
    pub missing_rssi: Dbm,
}

impl Default for MeasuredSettings {
    fn default() -> Self {
        Self {
            interpolation: InterpolationSettings::default(),
            ssids: Vec::new(),
            missing_rssi: Dbm(-100.0),
        }
    }
}

impl MeasuredSettings {
    pub fn accepts(&self, r: &BssReading) -> bool {
        self.ssids.is_empty() || self.ssids.contains(&r.ssid)
    }
}

/// One physical radio inferred from the survey.
#[derive(Debug, Clone, PartialEq)]
pub struct MeasuredRadio {
    pub key: RadioKey,
    /// BSSIDs grouped into this radio (virtual APs of the same radio).
    pub bssids: Vec<String>,
    pub ssids: Vec<String>,
    pub channel: Channel,
}

#[derive(Debug, Clone)]
pub struct MeasuredMap {
    pub grid: GridSpec,
    pub radios: Vec<MeasuredRadio>,
    pub layers: Vec<RadioLayer>,
    /// Whether each cell is close enough to a measurement to have data.
    pub valid: Vec<bool>,
    /// Fitted variogram, when kriging was used.
    pub variogram: Option<Variogram>,
}

/// Groups virtual BSSIDs of one radio: same frequency, same MAC except for
/// the locally-administered bits of the first byte and the low nibble of the
/// last byte (the usual way vendors derive per-SSID BSSIDs).
fn radio_group_key(r: &BssReading) -> String {
    let bytes: Vec<u8> = r
        .bssid
        .split(':')
        .filter_map(|h| u8::from_str_radix(h, 16).ok())
        .collect();
    if bytes.len() != 6 {
        return format!("{}|{}", r.frequency_mhz, r.bssid);
    }
    format!(
        "{}|{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:x}x",
        r.frequency_mhz,
        bytes[0] & !0x02,
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5] >> 4
    )
}

pub fn build(survey: &Survey, grid: GridSpec, settings: &MeasuredSettings) -> MeasuredMap {
    let positions: Vec<_> = survey.measurements.iter().map(|m| m.position).collect();

    // Group readings into radios.
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let mut radios: Vec<MeasuredRadio> = Vec::new();
    let mut values: Vec<Vec<f32>> = Vec::new();
    for (pi, m) in survey.measurements.iter().enumerate() {
        for r in m.readings.iter().filter(|r| settings.accepts(r)) {
            let Some((band, number)) = r.channel() else {
                continue;
            };
            let gi = *groups.entry(radio_group_key(r)).or_insert_with(|| {
                let id = radios.len();
                radios.push(MeasuredRadio {
                    key: RadioKey {
                        ap: ApId(id as u32 + 1),
                        radio: 0,
                    },
                    bssids: Vec::new(),
                    ssids: Vec::new(),
                    // Scans do not report the channel width reliably; 20 MHz
                    // is assumed for interference.
                    channel: Channel::closest(band, number, ChannelWidth::Mhz20),
                });
                values.push(vec![settings.missing_rssi.0; positions.len()]);
                id
            });
            let radio = &mut radios[gi];
            if !radio.bssids.contains(&r.bssid) {
                radio.bssids.push(r.bssid.clone());
            }
            if !radio.ssids.contains(&r.ssid) {
                radio.ssids.push(r.ssid.clone());
            }
            let v = &mut values[gi][pi];
            *v = v.max(r.rssi.0);
        }
    }

    let variogram = (settings.interpolation.method == InterpolationMethod::Kriging)
        .then(|| fit_variogram(&empirical_variogram(&positions, &values, 15)));
    let weights = WeightGrid::build(
        grid,
        &positions,
        &settings.interpolation,
        variogram.as_ref(),
    );
    let valid: Vec<bool> = (0..grid.len()).map(|c| weights.has_data(c)).collect();
    let layers = radios
        .iter()
        .zip(&values)
        .map(|(radio, v)| {
            let rssi = weights
                .apply(v)
                .into_iter()
                .map(|x| if x.is_nan() { Dbm::FLOOR.0 } else { x })
                .collect();
            RadioLayer::from_values(radio.key, radio.channel, rssi)
        })
        .collect();
    MeasuredMap {
        grid,
        radios,
        layers,
        valid,
        variogram,
    }
}

/// Strongest reading at one measurement, optionally limited to a band.
pub fn best_rssi(m: &Measurement, settings: &MeasuredSettings, band: Option<Band>) -> Option<Dbm> {
    m.readings
        .iter()
        .filter(|r| settings.accepts(r))
        .filter(|r| band.is_none_or(|b| r.band() == Some(b)))
        .map(|r| r.rssi)
        .max_by(|a, b| a.0.total_cmp(&b.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Vec2;
    use crate::survey::tests::reading;

    fn survey() -> Survey {
        let mut s = Survey::new("t", "test");
        let pts = [
            (1.0, 1.0, -40.0),
            (9.0, 1.0, -60.0),
            (1.0, 9.0, -60.0),
            (9.0, 9.0, -80.0),
        ];
        for (i, (x, y, v)) in pts.into_iter().enumerate() {
            s.measurements.push(Measurement {
                id: i as u32 + 1,
                position: Vec2::new(x, y),
                timestamp_ms: 0,
                readings: vec![
                    // Two virtual BSSIDs of the same 5 GHz radio.
                    reading("02:11:22:33:44:50", "Corp", v, 5180),
                    reading("00:11:22:33:44:51", "Guest", v - 3.0, 5180),
                    // A different radio, only heard at the first point.
                    reading("00:aa:bb:cc:dd:01", "Corp", -70.0, 2437),
                ]
                .into_iter()
                .filter(|r| i == 0 || r.frequency_mhz != 2437)
                .collect(),
            });
        }
        s
    }

    fn cell(g: &GridSpec, x: f32, y: f32) -> usize {
        (y / g.cell_m) as usize * g.cols + (x / g.cell_m) as usize
    }

    #[test]
    fn groups_virtual_bssids_into_one_radio() {
        let g = GridSpec::covering(10.0, 10.0, 2.0);
        let m = build(&survey(), g, &MeasuredSettings::default());
        assert_eq!(m.radios.len(), 2);
        let r5 = m
            .radios
            .iter()
            .find(|r| r.channel.band == Band::G5)
            .unwrap();
        assert_eq!(r5.bssids.len(), 2);
        assert_eq!(r5.ssids, vec!["Corp".to_owned(), "Guest".to_owned()]);
    }

    #[test]
    fn layers_match_readings_at_measurement_points() {
        let g = GridSpec::covering(10.0, 10.0, 2.0);
        for method in [InterpolationMethod::Idw, InterpolationMethod::Kriging] {
            let settings = MeasuredSettings {
                interpolation: InterpolationSettings {
                    method,
                    ..Default::default()
                },
                ..Default::default()
            };
            let m = build(&survey(), g, &settings);
            let i5 = m
                .radios
                .iter()
                .position(|r| r.channel.band == Band::G5)
                .unwrap();
            let i24 = 1 - i5;
            let at = |layer: usize, x, y| m.layers[layer].rssi[cell(&g, x, y)];
            assert!((at(i5, 1.0, 1.0) + 40.0).abs() < 1e-2, "{method:?}");
            assert!((at(i5, 9.0, 9.0) + 80.0).abs() < 1e-2, "{method:?}");
            // Not heard at (9, 9): the detection floor is used.
            assert!((at(i24, 9.0, 9.0) + 100.0).abs() < 1e-2, "{method:?}");
            assert_eq!(
                m.variogram.is_some(),
                method == InterpolationMethod::Kriging
            );
        }
    }

    #[test]
    fn ssid_filter_and_mask() {
        let g = GridSpec::covering(30.0, 10.0, 1.0);
        let settings = MeasuredSettings {
            ssids: vec!["Guest".into()],
            interpolation: InterpolationSettings {
                max_distance_m: 3.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let m = build(&survey(), g, &settings);
        assert_eq!(m.radios.len(), 1);
        assert_eq!(m.radios[0].ssids, vec!["Guest".to_owned()]);
        assert!(m.valid[cell(&g, 1.5, 1.5)]);
        assert!(!m.valid[cell(&g, 25.5, 5.5)]);
        assert_eq!(m.layers[0].rssi[cell(&g, 25.5, 5.5)], Dbm::FLOOR.0);
    }

    #[test]
    fn best_rssi_respects_filters() {
        let s = survey();
        let all = MeasuredSettings::default();
        assert_eq!(best_rssi(&s.measurements[0], &all, None), Some(Dbm(-40.0)));
        assert_eq!(
            best_rssi(&s.measurements[0], &all, Some(Band::G2_4)),
            Some(Dbm(-70.0))
        );
        let guest = MeasuredSettings {
            ssids: vec!["Guest".into()],
            ..Default::default()
        };
        assert_eq!(
            best_rssi(&s.measurements[0], &guest, None),
            Some(Dbm(-43.0))
        );
        assert_eq!(best_rssi(&s.measurements[1], &all, Some(Band::G6)), None);
    }
}
