//! Linux backend: nl80211 over generic netlink (no NetworkManager needed).
//!
//! Each scan asks the kernel to trigger a scan on every station interface,
//! waits for it, then dumps the BSS table (signal in mBm, frequency, beacon
//! IEs parsed for SSID and security).
//!
//! Limitations: triggering a scan requires `CAP_NET_ADMIN` (run as root or
//! `sudo setcap cap_net_admin+ep ondalux`). Without it the trigger is
//! refused and the kernel's cached results are returned, which are only as
//! fresh as the last scan made by NetworkManager/wpa_supplicant (often up to
//! a couple of minutes old). The wait after triggering is a fixed delay.
//! This backend compiles in CI but has not been tested on real hardware yet.

use crate::{format_mac, ie, ScanError, WifiScanner};
use futures::stream::TryStreamExt;
use ondalux_core::geometry::Vec2;
use ondalux_core::survey::BssReading;
use ondalux_core::units::Dbm;
use std::time::Duration;
use wl_nl80211::{Nl80211Attr, Nl80211BssInfo, Nl80211InterfaceType, Nl80211Scan};

pub const NAME: &str = "Linux nl80211";
/// Time allowed for a triggered scan to finish before reading results.
const SCAN_WAIT: Duration = Duration::from_secs(4);

pub struct Nl80211Scanner {
    rt: tokio::runtime::Runtime,
}

fn err(e: impl std::fmt::Display) -> ScanError {
    ScanError::Platform(e.to_string())
}

impl Nl80211Scanner {
    pub fn new() -> Result<Self, ScanError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(err)?;
        let s = Self { rt };
        if s.interfaces()?.is_empty() {
            return Err(ScanError::NoInterface);
        }
        Ok(s)
    }

    fn interfaces(&self) -> Result<Vec<u32>, ScanError> {
        self.rt.block_on(async {
            let (conn, handle, _) = wl_nl80211::new_connection().map_err(err)?;
            tokio::spawn(conn);
            let mut stream = handle.interface().get(Vec::new()).execute().await;
            let mut out = Vec::new();
            while let Some(msg) = stream.try_next().await.map_err(err)? {
                let mut index = None;
                let mut station = false;
                for attr in &msg.payload.attributes {
                    match attr {
                        Nl80211Attr::IfIndex(i) => index = Some(*i),
                        Nl80211Attr::IfType(t) => station = *t == Nl80211InterfaceType::Station,
                        _ => {}
                    }
                }
                if let (Some(i), true) = (index, station) {
                    out.push(i);
                }
            }
            Ok(out)
        })
    }

    fn scan_interface(&self, if_index: u32) -> Result<Vec<BssReading>, ScanError> {
        let triggered = self.rt.block_on(async {
            let (conn, handle, _) = wl_nl80211::new_connection().map_err(err)?;
            tokio::spawn(conn);
            let attrs = Nl80211Scan::new(if_index).build();
            let mut stream = handle.scan().trigger(attrs).execute().await;
            // Errors (typically EPERM without CAP_NET_ADMIN) fall back to
            // the cached results below.
            let mut ok = true;
            loop {
                match stream.try_next().await {
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            Ok::<bool, ScanError>(ok)
        })?;
        if triggered {
            std::thread::sleep(SCAN_WAIT);
        }
        self.rt.block_on(async {
            let (conn, handle, _) = wl_nl80211::new_connection().map_err(err)?;
            tokio::spawn(conn);
            let mut stream = handle.scan().dump(if_index).execute().await;
            let mut out = Vec::new();
            while let Some(msg) = stream.try_next().await.map_err(err)? {
                for attr in &msg.payload.attributes {
                    if let Nl80211Attr::Bss(info) = attr {
                        if let Some(r) = reading_from(info) {
                            out.push(r);
                        }
                    }
                }
            }
            Ok(out)
        })
    }
}

fn reading_from(info: &[Nl80211BssInfo]) -> Option<BssReading> {
    let mut bssid = None;
    let mut freq = None;
    let mut signal = None;
    let mut capability = 0u16;
    let mut ies: &[u8] = &[];
    for i in info {
        match i {
            Nl80211BssInfo::Bssid(m) => bssid = Some(format_mac(m)),
            Nl80211BssInfo::Frequency(f) => freq = Some(*f),
            Nl80211BssInfo::SignalMbm(s) => signal = Some(*s as f32 / 100.0),
            Nl80211BssInfo::Capability(c) => capability = c.bits(),
            Nl80211BssInfo::RawInformationElements(b) => ies = b,
            Nl80211BssInfo::RawBeaconInformationElements(b) if ies.is_empty() => ies = b,
            _ => {}
        }
    }
    Some(BssReading {
        bssid: bssid?,
        ssid: ie::ssid(ies).unwrap_or_default(),
        rssi: Dbm(signal?),
        frequency_mhz: freq?,
        security: ie::security(ies, capability),
    })
}

impl WifiScanner for Nl80211Scanner {
    fn name(&self) -> &str {
        NAME
    }

    fn scan(&mut self, _at: Vec2) -> Result<Vec<BssReading>, ScanError> {
        let interfaces = self.interfaces()?;
        if interfaces.is_empty() {
            return Err(ScanError::NoInterface);
        }
        let mut all: Vec<BssReading> = Vec::new();
        for i in interfaces {
            for r in self.scan_interface(i)? {
                match all.iter_mut().find(|x| x.bssid == r.bssid) {
                    Some(x) if x.rssi.0 < r.rssi.0 => *x = r,
                    Some(_) => {}
                    None => all.push(r),
                }
            }
        }
        Ok(all)
    }
}
