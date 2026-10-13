//! macOS backend: CoreWLAN (`CWWiFiClient` / `CWInterface`).
//!
//! Limitations: since macOS 14 (Sonoma), `ssid` and `bssid` are nil unless
//! the app has Location Services permission. A plain command-line binary
//! cannot request it; it must run as a signed `.app` bundle with
//! `NSLocationUsageDescription`. Without it, networks are still reported
//! (RSSI, channel, security) but are keyed by a placeholder BSSID, so
//! different APs on the same channel cannot be told apart.
//! Scans take a few seconds and macOS rate-limits them.
#![allow(unsafe_code)]

use crate::{ScanError, WifiScanner};
use objc2_core_wlan::{CWChannelBand, CWNetwork, CWSecurity, CWWiFiClient};
use ondalux_core::geometry::Vec2;
use ondalux_core::survey::{normalize_bssid, BssReading, Security};
use ondalux_core::units::Dbm;

pub const NAME: &str = "macOS CoreWLAN";

pub struct CoreWlanScanner;

impl CoreWlanScanner {
    pub fn new() -> Result<Self, ScanError> {
        // SAFETY: CoreWLAN class methods without preconditions.
        let has_interface = unsafe { CWWiFiClient::sharedWiFiClient().interface().is_some() };
        if has_interface {
            Ok(Self)
        } else {
            Err(ScanError::NoInterface)
        }
    }
}

fn security_of(n: &CWNetwork) -> Security {
    // SAFETY: supportsSecurity only queries the network object.
    let s = |sec| unsafe { n.supportsSecurity(sec) };
    if s(CWSecurity::Enterprise)
        || s(CWSecurity::WPA2Enterprise)
        || s(CWSecurity::WPA3Enterprise)
        || s(CWSecurity::WPAEnterprise)
        || s(CWSecurity::WPAEnterpriseMixed)
        || s(CWSecurity::DynamicWEP)
    {
        Security::Enterprise
    } else if s(CWSecurity::WPA3Personal) && !s(CWSecurity::WPA3Transition) {
        Security::Wpa3Personal
    } else if s(CWSecurity::WPA2Personal)
        || s(CWSecurity::Personal)
        || s(CWSecurity::WPA3Transition)
    {
        Security::Wpa2Personal
    } else if s(CWSecurity::WPAPersonal) || s(CWSecurity::WPAPersonalMixed) {
        Security::WpaPersonal
    } else if s(CWSecurity::OWE) || s(CWSecurity::OWETransition) {
        Security::Owe
    } else if s(CWSecurity::WEP) {
        Security::Wep
    } else if s(CWSecurity::None) {
        Security::Open
    } else {
        Security::Unknown
    }
}

fn frequency_mhz(band: CWChannelBand, number: i64) -> Option<u32> {
    let n = u32::try_from(number).ok()?;
    if band == CWChannelBand::Band2GHz {
        Some(if n == 14 { 2484 } else { 2407 + 5 * n })
    } else if band == CWChannelBand::Band5GHz {
        Some(5000 + 5 * n)
    } else if band == CWChannelBand::Band6GHz {
        Some(5950 + 5 * n)
    } else {
        None
    }
}

impl WifiScanner for CoreWlanScanner {
    fn name(&self) -> &str {
        NAME
    }

    fn scan(&mut self, _at: Vec2) -> Result<Vec<BssReading>, ScanError> {
        // SAFETY: CoreWLAN calls on objects retained for the duration of
        // this function; scanForNetworksWithName_error blocks until done.
        unsafe {
            let iface = CWWiFiClient::sharedWiFiClient()
                .interface()
                .ok_or(ScanError::NoInterface)?;
            let set = iface
                .scanForNetworksWithName_error(None)
                .map_err(|e| ScanError::Platform(e.localizedDescription().to_string()))?;
            let networks = set.allObjects();
            let mut out = Vec::new();
            for i in 0..networks.count() {
                let n = networks.objectAtIndex(i);
                let Some(channel) = n.wlanChannel() else {
                    continue;
                };
                let Some(freq) =
                    frequency_mhz(channel.channelBand(), channel.channelNumber() as i64)
                else {
                    continue;
                };
                let bssid = n
                    .bssid()
                    .and_then(|b| normalize_bssid(&b.to_string()))
                    // Placeholder when Location permission hides the BSSID.
                    .unwrap_or_else(|| {
                        format!("00:00:00:00:{:02x}:{:02x}", (i >> 8) & 0xff, i & 0xff)
                    });
                out.push(BssReading {
                    bssid,
                    ssid: n.ssid().map(|s| s.to_string()).unwrap_or_default(),
                    rssi: Dbm(n.rssiValue() as f32),
                    frequency_mhz: freq,
                    security: security_of(&n),
                });
            }
            Ok(out)
        }
    }
}
