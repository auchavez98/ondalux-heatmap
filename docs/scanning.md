# WiFi scanning by platform

`ondalux-scan` exposes one trait, `WifiScanner::scan(at) -> Vec<BssReading>`,
with a native backend per OS and a simulated one. Each reading has SSID,
BSSID, RSSI (dBm), primary channel center frequency (from which band and
channel are derived) and security.

| | Windows | macOS | Linux |
|---|---|---|---|
| API | Native WiFi (`wlanapi.dll`) | CoreWLAN | nl80211 (generic netlink) |
| Rust binding | `windows-sys` | `objc2-core-wlan` | `wl-nl80211` |
| Fresh scan | `WlanScan` + completion notification (≤ 8 s) | `scanForNetworksWithName:error:` (blocking) | `NL80211_CMD_TRIGGER_SCAN`, then 4 s wait |
| Security source | Beacon IEs (RSN / WPA) | `supportsSecurity:` | Beacon IEs (RSN / WPA) |
| Tested on hardware | ✅ (Windows 11, Intel adapter) | ❌ compiles only | ❌ compiles only |

## Windows

- **Location permission (Windows 11 24H2 and later):** scan results are only
  returned to desktop apps allowed to use location. Enable *Settings →
  Privacy & security → Location* and *Let desktop apps access your
  location*. Otherwise the app shows "permission denied".
- The driver may throttle scans to one every few seconds. If `WlanScan` is
  refused, the cached BSS list is returned instead.
- RSSI comes from `WLAN_BSS_ENTRY.lRssi`. Some drivers report it in coarse
  steps.
- Channel width is not reported. Measured interference assumes 20 MHz.

## macOS

- **Location Services:** since macOS 14, CoreWLAN hides `ssid` and `bssid`
  unless the app has location permission. Getting it requires a signed
  `.app` bundle with `NSLocationUsageDescription` and a `CLLocationManager`
  authorization request. **This is not implemented yet.** Without it,
  networks are still listed (RSSI, channel, security) but under placeholder
  BSSIDs (`00:00:00:00:xx:xx`) that change between scans. Per-AP heat maps
  are then unreliable, but best-signal maps still work.
- Scans block for a few seconds and are rate-limited by the OS.
- **Status: compiles (cross-checked with `cargo clippy --target
  aarch64-apple-darwin`) but has not been run on a Mac.**

## Linux

- Triggering a scan needs `CAP_NET_ADMIN`:

  ```bash
  sudo setcap cap_net_admin+ep target/release/ondalux
  ```

  Without it, the trigger is refused and the kernel's cached results are
  returned. These are only as fresh as the last scan made by
  NetworkManager/wpa_supplicant, often up to two minutes old. For survey
  work, grant the capability.
- After triggering, the backend waits a fixed 4 s and does not listen for
  the `NEW_SCAN_RESULTS` event yet.
- Signal comes from `NL80211_BSS_SIGNAL_MBM` (true dBm).
- **Status: compiles (cross-checked with `cargo clippy --target
  x86_64-unknown-linux-gnu`) but has not been run on Linux hardware.**

## Simulated scanner

`SimulatedScanner` computes each enabled radio's RSSI with the prediction
engine at the clicked position, adds log-normal shadow fading
(σ = 4 dB by default), rounds to 1 dB and drops readings below −92 dBm. It
reports SSID `Ondalux-Sim` and locally administered BSSIDs. Surveys made
with it are labelled `Simulated (prediction + random fading)` as their
source. It exists for demos, training and tests. Its readings are never
real measurements.

## Trying a scan from the command line

```bash
cargo run -p ondalux-scan --example scan
```
