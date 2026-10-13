//! Ondalux Heatmap scan: WiFi scanning behind a common trait.
//!
//! | Platform | Backend | Module |
//! |---|---|---|
//! | Windows | Native WLAN API (`wlanapi.dll`) | [`windows`] |
//! | macOS | CoreWLAN | `macos` |
//! | Linux | nl80211 (netlink) | `linux` |
//!
//! Platform limitations are documented in `docs/scanning.md`.
//! [`SimulatedScanner`] produces readings from the prediction engine, for
//! demos and testing without a WiFi adapter; it never pretends to be real.

pub mod ie;
pub mod simulated;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
pub mod windows;

use ondalux_core::geometry::Vec2;
use ondalux_core::survey::BssReading;
use std::fmt;

pub use simulated::SimulatedScanner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    /// No scanning backend for this platform.
    Unsupported(String),
    /// No WiFi adapter found (or it is disabled).
    NoInterface,
    /// The OS refused the scan (e.g. missing location permission).
    PermissionDenied(String),
    /// Any other error reported by the OS.
    Platform(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Unsupported(m) => write!(f, "WiFi scanning is not supported: {m}"),
            ScanError::NoInterface => write!(f, "no WiFi adapter found (is WiFi turned on?)"),
            ScanError::PermissionDenied(m) => write!(f, "permission denied: {m}"),
            ScanError::Platform(m) => write!(f, "WiFi scan failed: {m}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// A source of WiFi scans.
pub trait WifiScanner: Send {
    /// Human-readable backend name, stored as the survey source.
    fn name(&self) -> &str;

    /// Performs one scan and returns every BSS heard.
    ///
    /// `at` is where the user says they are standing, in plan meters. Real
    /// scanners ignore it; the simulated scanner needs it.
    fn scan(&mut self, at: Vec2) -> Result<Vec<BssReading>, ScanError>;
}

/// The native scanner for the current OS.
pub fn platform_scanner() -> Result<Box<dyn WifiScanner>, ScanError> {
    #[cfg(windows)]
    {
        return Ok(Box::new(windows::WlanScanner::new()?));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(Box::new(macos::CoreWlanScanner::new()?));
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(Box::new(linux::Nl80211Scanner::new()?));
    }
    #[allow(unreachable_code)]
    Err(ScanError::Unsupported(format!(
        "no backend for {}",
        std::env::consts::OS
    )))
}

/// Formats a MAC address as lowercase `aa:bb:cc:dd:ee:ff`.
pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_formatting() {
        assert_eq!(
            format_mac(&[0xAA, 0x0B, 0, 1, 0x22, 0xFF]),
            "aa:0b:00:01:22:ff"
        );
    }
}
