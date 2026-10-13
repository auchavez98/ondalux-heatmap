# Survey CSV format

Survey measurements can be imported from (and exported to) CSV. One row per
BSS heard at one point.

```csv
point,x_m,y_m,timestamp_ms,ssid,bssid,rssi_dbm,frequency_mhz,security
1,4.200,7.500,1760373000000,Corp,aa:bb:cc:00:11:22,-58,5180,WPA2-Personal
1,4.200,7.500,1760373000000,Guest,aa:bb:cc:00:11:23,-60,5180,Open
2,9.000,7.500,1760373015000,Corp,aa:bb:cc:00:11:22,-66,5180,WPA2-Personal
```

## Columns

Headers are case-insensitive and may appear in any order. Unknown columns
are ignored.

| Column | Required | Accepted names | Notes |
|---|---|---|---|
| Position | ✔ | `x_m`, `y_m` (or `x`, `y`) | Meters from the top-left of the plan |
| | | `x_px`, `y_px` | Alternative: image pixels, converted with the plan calibration |
| BSSID | ✔ | `bssid`, `mac` | `aa:bb:…`, `AA-BB-…`, `aabb.cc…` or 12 hex digits |
| Signal | ✔ | `rssi_dbm`, `rssi`, `signal_dbm`, `signal` | dBm, between −130 and +10 |
| Frequency | ✔ (one of) | `frequency_mhz`, `frequency`, `freq` | Primary channel center, MHz |
| | | `channel` (+ optional `band`) | Without `band`, channels 1–14 are 2.4 GHz, others 5 GHz. `band`: `2.4`, `5`, `6` (text like `6 GHz` works) |
| SSID | | `ssid`, `network` | Empty = hidden network |
| Security | | `security`, `auth`, `encryption` | `Open`, `OWE`, `WEP`, `WPA`, `WPA2`, `WPA3`, `SAE`, `Enterprise`, `802.1X`… Others → Unknown |
| Point | | `point`, `point_id`, `measurement` | Groups rows into one measurement. Without it, rows with identical coordinates are grouped |
| Time | | `timestamp_ms`, `timestamp` | Unix epoch in milliseconds |

## Separators and decimals

If the header contains more `;` than `,`, the file is read as
semicolon-separated and decimal commas are accepted (`-55,5`). This is the
format spreadsheet programs produce in Spanish and many other locales.

## Errors

Missing required columns abort the import. Invalid rows (bad BSSID, RSSI,
position or channel) are skipped and listed as warnings with their line
number. If the same BSSID appears twice at one point, the strongest reading
is kept.

The export (Survey panel → Export CSV) writes exactly the header shown
above, so exported files can be imported again.
