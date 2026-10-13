//! Field survey data: WiFi scans taken at known positions on the plan.

use crate::channel::{channel_from_frequency, Band};
use crate::geometry::Vec2;
use crate::units::Dbm;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Security of a BSS as advertised in its beacons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    Open,
    /// Opportunistic Wireless Encryption ("Enhanced Open").
    Owe,
    Wep,
    WpaPersonal,
    Wpa2Personal,
    Wpa3Personal,
    /// Any 802.1X / EAP mode (WPA, WPA2 or WPA3 Enterprise).
    Enterprise,
    Unknown,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "Open",
            Security::Owe => "OWE",
            Security::Wep => "WEP",
            Security::WpaPersonal => "WPA-Personal",
            Security::Wpa2Personal => "WPA2-Personal",
            Security::Wpa3Personal => "WPA3-Personal",
            Security::Enterprise => "Enterprise",
            Security::Unknown => "Unknown",
        }
    }

    /// Parses the labels written by [`Self::label`] and common variants.
    pub fn parse(s: &str) -> Security {
        let k: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        match k.as_str() {
            "open" | "none" => Security::Open,
            "owe" | "enhancedopen" => Security::Owe,
            "wep" => Security::Wep,
            "wpa" | "wpapersonal" | "wpapsk" => Security::WpaPersonal,
            "wpa2" | "wpa2personal" | "wpa2psk" => Security::Wpa2Personal,
            "wpa3" | "wpa3personal" | "wpa3sae" | "sae" => Security::Wpa3Personal,
            "enterprise" | "wpaenterprise" | "wpa2enterprise" | "wpa3enterprise" | "8021x"
            | "eap" => Security::Enterprise,
            _ => Security::Unknown,
        }
    }
}

impl fmt::Display for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One BSS (AP radio + SSID) heard in a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BssReading {
    /// Lowercase, colon-separated MAC, e.g. `aa:bb:cc:dd:ee:ff`.
    pub bssid: String,
    /// Network name; empty for hidden networks.
    pub ssid: String,
    pub rssi: Dbm,
    /// Center frequency of the primary 20 MHz channel, in MHz.
    pub frequency_mhz: u32,
    pub security: Security,
}

impl BssReading {
    pub fn channel(&self) -> Option<(Band, u16)> {
        channel_from_frequency(self.frequency_mhz)
    }

    pub fn band(&self) -> Option<Band> {
        self.channel().map(|(b, _)| b)
    }
}

/// Normalizes a MAC address to lowercase `aa:bb:cc:dd:ee:ff`.
/// Accepts `:`, `-` or `.` separators, or none. Returns `None` if invalid.
pub fn normalize_bssid(s: &str) -> Option<String> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let separators_ok = s
        .chars()
        .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '-' | '.'));
    if hex.len() != 12 || !separators_ok {
        return None;
    }
    let lower = hex.to_ascii_lowercase();
    Some(
        (0..6)
            .map(|i| &lower[i * 2..i * 2 + 2])
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// One scan taken at one place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub id: u32,
    /// Position on the plan, in meters.
    pub position: Vec2,
    /// Milliseconds since the Unix epoch; 0 if unknown.
    pub timestamp_ms: i64,
    pub readings: Vec<BssReading>,
}

/// A set of measurements, e.g. one walk-through of the building.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Survey {
    pub name: String,
    /// Where the data came from, e.g. "Windows WLAN API" or "CSV: walk1.csv".
    pub source: String,
    pub measurements: Vec<Measurement>,
}

/// Summary of one BSSID across a survey.
#[derive(Debug, Clone, PartialEq)]
pub struct BssSummary {
    pub bssid: String,
    pub ssid: String,
    pub frequency_mhz: u32,
    pub security: Security,
    pub max_rssi: Dbm,
    /// Number of measurements where it was heard.
    pub heard_at: usize,
}

impl Survey {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            measurements: Vec::new(),
        }
    }

    pub fn next_id(&self) -> u32 {
        self.measurements
            .iter()
            .map(|m| m.id)
            .max()
            .map_or(1, |m| m + 1)
    }

    /// Every BSSID heard, sorted by SSID then BSSID.
    pub fn bss_summary(&self) -> Vec<BssSummary> {
        let mut map: BTreeMap<String, BssSummary> = BTreeMap::new();
        for m in &self.measurements {
            for r in &m.readings {
                let e = map.entry(r.bssid.clone()).or_insert_with(|| BssSummary {
                    bssid: r.bssid.clone(),
                    ssid: r.ssid.clone(),
                    frequency_mhz: r.frequency_mhz,
                    security: r.security,
                    max_rssi: r.rssi,
                    heard_at: 0,
                });
                e.heard_at += 1;
                if r.rssi > e.max_rssi {
                    e.max_rssi = r.rssi;
                }
            }
        }
        let mut v: Vec<_> = map.into_values().collect();
        v.sort_by(|a, b| a.ssid.cmp(&b.ssid).then_with(|| a.bssid.cmp(&b.bssid)));
        v
    }

    /// Distinct SSIDs heard (hidden networks appear as an empty string).
    pub fn ssids(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .measurements
            .iter()
            .flat_map(|m| m.readings.iter().map(|r| r.ssid.clone()))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn reading(bssid: &str, ssid: &str, rssi: f32, f: u32) -> BssReading {
        BssReading {
            bssid: bssid.into(),
            ssid: ssid.into(),
            rssi: Dbm(rssi),
            frequency_mhz: f,
            security: Security::Wpa2Personal,
        }
    }

    #[test]
    fn bssid_normalization() {
        assert_eq!(
            normalize_bssid("AA-BB-CC-00-11-22").as_deref(),
            Some("aa:bb:cc:00:11:22")
        );
        assert_eq!(
            normalize_bssid("aabb.cc00.1122").as_deref(),
            Some("aa:bb:cc:00:11:22")
        );
        assert_eq!(normalize_bssid("aa:bb:cc:00:11"), None);
        assert_eq!(normalize_bssid("zz:bb:cc:00:11:22"), None);
    }

    #[test]
    fn security_parsing() {
        assert_eq!(Security::parse("WPA2-Personal"), Security::Wpa2Personal);
        assert_eq!(Security::parse("wpa3 sae"), Security::Wpa3Personal);
        assert_eq!(Security::parse("802.1X"), Security::Enterprise);
        assert_eq!(Security::parse("OPEN"), Security::Open);
        assert_eq!(Security::parse("???"), Security::Unknown);
        for s in [
            Security::Open,
            Security::Owe,
            Security::Wep,
            Security::WpaPersonal,
            Security::Wpa2Personal,
            Security::Wpa3Personal,
            Security::Enterprise,
        ] {
            assert_eq!(Security::parse(s.label()), s);
        }
    }

    #[test]
    fn summary_tracks_max_and_count() {
        let mut s = Survey::new("walk", "test");
        s.measurements.push(Measurement {
            id: 1,
            position: Vec2::new(0.0, 0.0),
            timestamp_ms: 0,
            readings: vec![
                reading("aa:aa:aa:aa:aa:01", "Corp", -60.0, 5180),
                reading("aa:aa:aa:aa:aa:02", "Guest", -70.0, 2437),
            ],
        });
        s.measurements.push(Measurement {
            id: 2,
            position: Vec2::new(5.0, 0.0),
            timestamp_ms: 0,
            readings: vec![reading("aa:aa:aa:aa:aa:01", "Corp", -50.0, 5180)],
        });
        let sum = s.bss_summary();
        assert_eq!(sum.len(), 2);
        assert_eq!(sum[0].ssid, "Corp");
        assert_eq!(sum[0].max_rssi, Dbm(-50.0));
        assert_eq!(sum[0].heard_at, 2);
        assert_eq!(s.ssids(), vec!["Corp".to_owned(), "Guest".to_owned()]);
        assert_eq!(s.next_id(), 3);
        assert_eq!(sum[0].bssid, "aa:aa:aa:aa:aa:01");
        assert_eq!(
            s.measurements[0].readings[1].channel(),
            Some((Band::G2_4, 6))
        );
    }
}
