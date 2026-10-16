# `.lux` project format — version 3

A `.lux` file is a standard ZIP archive. Any unzip tool can open it.

| Entry | Compression | Content |
|---|---|---|
| `project.json` | deflate | Header + project model (UTF-8 JSON) |
| `floorplan.png` / `floorplan.jpg` | stored | Original floor plan image bytes (optional) |
| `floorplan-source.pdf` | deflated | Original PDF the plan was imported from (optional), used for wall detection |
| `floors/<n>/floorplan.<ext>`, `floors/<n>/floorplan-source.pdf` | as above | The same for floor `n` (0-based, building order) when `n` ≥ 1; the lowest floor uses the entries above |
| `[floors/<n>/]floorplan-source.json` | deflated | Optional: `{"page": p, "same_as": m}`. The plan is page `p` (0-based) of the source PDF, and the PDF bytes are stored in floor `m`'s entry (a multi-page PDF imported as several floors is stored once). Absent = page 0, PDF in this floor's entry |

Why ZIP instead of compressed JSON: the floor plan image is already compressed
and can be large, so it is stored as-is next to a human-readable JSON file.
ZIP is pure Rust to read and write (no C toolchain) and widely supported.

## `project.json`

```json
{
  "format": "ondalux-project",
  "format_version": 3,
  "generator": "Ondalux Heatmap 0.2.0",
  "project": { ... }
}
```

Readers must reject files whose `format` differs and files whose
`format_version` is higher than the one they support. Older versions are
migrated on load, one version at a time.

| Version | Changes |
|---|---|
| 1 | Initial format (floor plan, materials, walls, APs, settings) |
| 2 | Adds `surveys` and `survey_settings`. Version 1 files load unchanged with no surveys |
| 3 | Multi-floor buildings: `floors[]`, `active_floor`, `cross_floor`; wall `thickness_cm` and material `thickness_cm`. Version 1–2 files (a single floor at the top level: `plan`, `walls`, `aps`, `surveys`) load as one floor, and built-in materials get their reference thickness |

Files written under the project's former name (extension `.hcraft`, `format`
`"heatcraft-project"`) are still read.

### `project`

| Field | Type | Notes |
|---|---|---|
| `name` | string | |
| `floors[]` | array | The floors, lowest first; see below (v3) |
| `active_floor` | integer | Floor shown when the project is opened (v3) |
| `cross_floor` | bool | Include APs of up to two floors above and below in predictions (v3, default true) |
| `materials.materials[]` | array | `id` (int), `name`, `color` ([r, g, b, a]), `attenuation` (`{"2.4GHz": dB, "5GHz": dB, "6GHz": dB}`), `thickness_cm` (optional, v3: the thickness the attenuation refers to; walls of another thickness scale it linearly; absent = the loss does not depend on thickness) |
| `settings` | object | `model`, `receiver_height_m`, `receiver_gain` (dBi), `noise_figure` (dB) |
| `requirements` | object | `min_rssi` (dBm), `min_snr` (dB), `cca_threshold` (dBm) |
| `grid_cell_m` | number | Heat map sampling step in meters |
| `survey_settings` | object | `interpolation` (`method`: `"idw"`/`"kriging"`, `idw_power`, `neighbors`, `max_distance_m`), `ssids` (list; empty = all), `missing_rssi` (dBm) (v2) |

### Floor

| Field | Type | Notes |
|---|---|---|
| `info.name` | string | Empty = numbered by the app |
| `info.height_m` | number | Floor-to-floor height (default 3) |
| `info.slab_loss_db` | number | Loss of the slab under this floor, all bands (default 15) |
| `info.offset_m` | `{x, y}` | Where this floor's plan origin sits in building coordinates, to line plans up |
| `plan.width_px`, `plan.height_px` | integer | Floor plan image size |
| `plan.calibration` | object or null | `a_px`, `b_px` (`{x, y}` in image pixels) and `distance_m`. Null = uncalibrated (20 px/m assumed) |
| `walls[]` | array | `segment.a`, `segment.b` (`{x, y}` in **meters**), `material` (material id), `thickness_cm` (optional, v3; absent = the material's reference thickness) |
| `aps[]` | array | See below. AP ids are unique across floors |
| `surveys[]` | array | Field surveys, see below |

All positions are in meters in the plan's coordinate system: origin at the
top-left corner of the image, x to the right, y down. Pixel position =
meters × px_per_m.

`settings.model` is one of:

```json
{ "type": "free_space" }
{ "type": "log_distance", "exponent": {"2.4GHz": 2.0, "5GHz": 2.0, "6GHz": 2.0}, "ref_distance_m": 1.0 }
```

### Access point

```json
{
  "id": 1,
  "name": "Lobby",
  "vendor": "Ubiquiti",
  "model": "Generic Wi-Fi 6",
  "position": { "x": 4.0, "y": 5.0 },
  "height_m": 2.8,
  "orientation_deg": 90.0,
  "mount": "wall",
  "pattern": { "beamwidth_deg": 180.0, "front_to_back_db": 10.0 },
  "radios": [
    {
      "channel": { "band": "6GHz", "number": 37, "width": "80" },
      "tx_power": 18.0,
      "antenna_gain": 4.5,
      "enabled": true
    }
  ]
}
```

`band` is `"2.4GHz"`, `"5GHz"` or `"6GHz"`. `width` is `"20"`, `"40"`,
`"80"`, `"160"` or `"320"` (MHz). `tx_power` is in dBm, `antenna_gain` in dBi.
`mount` is `"ceiling"` (default) or `"wall"`. `orientation_deg` is the pointing
direction on the plan (0 = +x, 90 = +y, i.e. down on the plan) and, like
`pattern`, only affects predictions for wall APs. Files without `mount` or
`pattern` load as ceiling / omnidirectional.
`vendor` is optional (empty for generic APs) and identifies the catalog model
together with `model`.

### Survey

```json
{
  "name": "Survey 1",
  "source": "Windows WLAN API",
  "measurements": [
    {
      "id": 1,
      "position": { "x": 4.2, "y": 7.5 },
      "timestamp_ms": 1760373000000,
      "readings": [
        {
          "bssid": "aa:bb:cc:00:11:22",
          "ssid": "Corp",
          "rssi": -58.0,
          "frequency_mhz": 5180,
          "security": "wpa2_personal"
        }
      ]
    }
  ]
}
```

`security` is one of `open`, `owe`, `wep`, `wpa_personal`, `wpa2_personal`,
`wpa3_personal`, `enterprise`, `unknown`. `frequency_mhz` is the primary
20 MHz channel center. Band and channel are derived from it.
