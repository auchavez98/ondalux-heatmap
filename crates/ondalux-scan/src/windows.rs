//! Windows backend: Native WiFi (WLAN) API.
//!
//! `WlanScan` asks the driver for a fresh scan; completion is signalled by an
//! ACM notification (`scan_complete` / `scan_fail`), after which
//! `WlanGetNetworkBssList` returns every BSS with RSSI, center frequency and
//! raw beacon IEs (parsed for SSID and security).
//!
//! Limitations: Windows 11 24H2 and later only return scan results to apps
//! allowed to use location (Settings → Privacy & security → Location →
//! "Let desktop apps access your location"); otherwise the API reports
//! access denied. Drivers may throttle scans to about one every few seconds.
#![allow(unsafe_code)]

use crate::{format_mac, ie, ScanError, WifiScanner};
use ondalux_core::geometry::Vec2;
use ondalux_core::survey::BssReading;
use ondalux_core::units::Dbm;
use std::ffi::c_void;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_any, wlan_notification_acm_scan_complete, wlan_notification_acm_scan_fail,
    WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanGetNetworkBssList, WlanOpenHandle,
    WlanRegisterNotification, WlanScan, L2_NOTIFICATION_DATA, WLAN_BSS_ENTRY, WLAN_BSS_LIST,
    WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_NOTIFICATION_SOURCE_ACM,
};

pub const NAME: &str = "Windows WLAN API";
/// Upper bound for a scan; most drivers finish in 2–4 s.
const SCAN_TIMEOUT: Duration = Duration::from_secs(8);
const WLAN_NOTIFICATION_SOURCE_NONE: u32 = 0;

/// Interfaces whose scan finished, written by the notification callback.
#[derive(Default)]
struct Notifications {
    done: Mutex<Vec<GUID>>,
    cond: Condvar,
}

pub struct WlanScanner {
    handle: HANDLE,
    /// Boxed so its address stays valid for the registered callback.
    notifications: Box<Notifications>,
}

// SAFETY: a WLAN client handle may be used from any thread of the process;
// the scanner is only used by one thread at a time (&mut self).
unsafe impl Send for WlanScanner {}

fn win_err(context: &str, code: u32) -> ScanError {
    if code == ERROR_ACCESS_DENIED {
        ScanError::PermissionDenied(
            "Windows denied access to WiFi scan results. Enable Settings → Privacy & security → \
             Location, and \"Let desktop apps access your location\"."
                .into(),
        )
    } else {
        ScanError::Platform(format!("{context} failed with Windows error {code}"))
    }
}

unsafe extern "system" fn on_notification(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    // SAFETY: called by wlanapi with a valid notification; `context` is the
    // `Notifications` box registered in `new`, alive until unregistration.
    unsafe {
        let (Some(data), Some(n)) = (data.as_ref(), (context as *const Notifications).as_ref())
        else {
            return;
        };
        let code = data.NotificationCode as i32;
        if data.NotificationSource == WLAN_NOTIFICATION_SOURCE_ACM
            && (code == wlan_notification_acm_scan_complete
                || code == wlan_notification_acm_scan_fail)
        {
            if let Ok(mut done) = n.done.lock() {
                done.push(data.InterfaceGuid);
                n.cond.notify_all();
            }
        }
    }
}

fn same_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

impl WlanScanner {
    pub fn new() -> Result<Self, ScanError> {
        let mut version = 0u32;
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: out-pointers are valid locals.
        let rc = unsafe { WlanOpenHandle(2, std::ptr::null(), &mut version, &mut handle) };
        if rc != ERROR_SUCCESS {
            return Err(win_err("WlanOpenHandle", rc));
        }
        let notifications = Box::<Notifications>::default();
        let ctx = &*notifications as *const Notifications as *const c_void;
        // SAFETY: the callback and context outlive the registration, which
        // is removed in Drop before the box is freed.
        let rc = unsafe {
            WlanRegisterNotification(
                handle,
                WLAN_NOTIFICATION_SOURCE_ACM,
                1,
                Some(on_notification),
                ctx,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        let scanner = Self {
            handle,
            notifications,
        };
        if rc != ERROR_SUCCESS {
            return Err(win_err("WlanRegisterNotification", rc));
        }
        Ok(scanner)
    }

    fn interfaces(&self) -> Result<Vec<GUID>, ScanError> {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        // SAFETY: valid handle and out-pointer.
        let rc = unsafe { WlanEnumInterfaces(self.handle, std::ptr::null(), &mut list) };
        if rc != ERROR_SUCCESS {
            return Err(win_err("WlanEnumInterfaces", rc));
        }
        // SAFETY: on success `list` points to a WLAN_INTERFACE_INFO_LIST with
        // dwNumberOfItems trailing entries; freed with WlanFreeMemory.
        let guids = unsafe {
            let n = (*list).dwNumberOfItems as usize;
            let first = std::ptr::addr_of!((*list).InterfaceInfo) as *const WLAN_INTERFACE_INFO;
            let v = (0..n).map(|i| (*first.add(i)).InterfaceGuid).collect();
            WlanFreeMemory(list as *const c_void);
            v
        };
        Ok(guids)
    }

    fn bss_list(&self, guid: &GUID) -> Result<Vec<BssReading>, ScanError> {
        let mut list: *mut WLAN_BSS_LIST = std::ptr::null_mut();
        // SAFETY: valid handle, interface GUID and out-pointer.
        let rc = unsafe {
            WlanGetNetworkBssList(
                self.handle,
                guid,
                std::ptr::null(),
                dot11_BSS_type_any,
                0,
                std::ptr::null(),
                &mut list,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(win_err("WlanGetNetworkBssList", rc));
        }
        // SAFETY: on success `list` holds dwNumberOfItems entries; each
        // entry's IEs live at entry + ulIeOffset for ulIeSize bytes inside
        // the same allocation. Freed with WlanFreeMemory.
        let readings = unsafe {
            let n = (*list).dwNumberOfItems as usize;
            let first = std::ptr::addr_of!((*list).wlanBssEntries) as *const WLAN_BSS_ENTRY;
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let e = &*first.add(i);
                let ies = std::slice::from_raw_parts(
                    (e as *const WLAN_BSS_ENTRY as *const u8).add(e.ulIeOffset as usize),
                    e.ulIeSize as usize,
                );
                let len = (e.dot11Ssid.uSSIDLength as usize).min(32);
                let ssid = String::from_utf8_lossy(&e.dot11Ssid.ucSSID[..len]).into_owned();
                v.push(BssReading {
                    bssid: format_mac(&e.dot11Bssid),
                    ssid,
                    rssi: Dbm(e.lRssi as f32),
                    frequency_mhz: e.ulChCenterFrequency / 1000,
                    security: ie::security(ies, e.usCapabilityInformation),
                });
            }
            WlanFreeMemory(list as *const c_void);
            v
        };
        Ok(readings)
    }

    /// Triggers a scan on `guid` and waits for its completion notification.
    fn scan_and_wait(&self, guid: &GUID) {
        if let Ok(mut done) = self.notifications.done.lock() {
            done.retain(|g| !same_guid(g, guid));
        }
        // SAFETY: valid handle and GUID; optional arguments are null.
        let rc = unsafe {
            WlanScan(
                self.handle,
                guid,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        if rc != ERROR_SUCCESS {
            // The cached BSS list is still returned below.
            return;
        }
        let deadline = Instant::now() + SCAN_TIMEOUT;
        let Ok(mut done) = self.notifications.done.lock() else {
            return;
        };
        while !done.iter().any(|g| same_guid(g, guid)) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.notifications.cond.wait_timeout(done, left) {
                Ok((d, _)) => done = d,
                Err(_) => return,
            }
        }
    }
}

impl WifiScanner for WlanScanner {
    fn name(&self) -> &str {
        NAME
    }

    fn scan(&mut self, _at: Vec2) -> Result<Vec<BssReading>, ScanError> {
        let guids = self.interfaces()?;
        if guids.is_empty() {
            return Err(ScanError::NoInterface);
        }
        let mut all: Vec<BssReading> = Vec::new();
        for guid in &guids {
            self.scan_and_wait(guid);
            for r in self.bss_list(guid)? {
                // Several adapters may hear the same BSS: keep the strongest.
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

impl Drop for WlanScanner {
    fn drop(&mut self) {
        // SAFETY: unregister the callback before the context box is freed,
        // then close the handle.
        unsafe {
            WlanRegisterNotification(
                self.handle,
                WLAN_NOTIFICATION_SOURCE_NONE,
                1,
                None,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            );
            WlanCloseHandle(self.handle, std::ptr::null());
        }
    }
}
