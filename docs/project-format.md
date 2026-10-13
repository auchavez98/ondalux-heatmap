# `.hcraft` project format — version 1

A `.hcraft` file is a standard ZIP archive. Any unzip tool can open it.

| Entry | Compression | Content |
|---|---|---|
| `project.json` | deflate | Header + project model (UTF-8 JSON) |
| `floorplan.png` / `floorplan.jpg` | stored | Original floor plan image bytes (optional) |

Why ZIP instead of compressed JSON: the floor plan image is already compressed
and can be large, so it is stored as-is next to a human-readable JSON file.
ZIP is pure Rust to read and write (no C toolchain) and widely supported.

## `project.json`

```json
{
  "format": "heatcraft-project",
  "format_version": 1,
  "generator": "HeatCraft 0.1.0",
  "project": { ... }
}
```

Readers must reject files whose `format` differs and files whose
`format_version` is higher than the one they support. Older versions are
migrated on load, one version at a time.

### `project`

| Field | Type | Notes |
|---|---|---|
| `name` | string | |
| `plan.width_px`, `plan.height_px` | integer | Floor plan image size |
| `plan.calibration` | object or null | `a_px`, `b_px` (`{x, y}` in image pixels) and `distance_m`. Null = uncalibrated (20 px/m assumed) |
| `materials.materials[]` | array | `id` (int), `name`, `color` ([r, g, b, a]), `attenuation` (`{"2.4GHz": dB, "5GHz": dB, "6GHz": dB}`) |
| `walls[]` | array | `segment.a`, `segment.b` (`{x, y}` in **meters**), `material` (material id) |
| `aps[]` | array | See below |
| `settings` | object | `model`, `receiver_height_m`, `receiver_gain` (dBi), `noise_figure` (dB) |
| `requirements` | object | `min_rssi` (dBm), `min_snr` (dB), `cca_threshold` (dBm) |
| `grid_cell_m` | number | Heat map sampling step in meters |

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
  "model": "Generic Wi-Fi 6",
  "position": { "x": 4.0, "y": 5.0 },
  "height_m": 2.8,
  "orientation_deg": 90.0,
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
`orientation_deg` is stored but not used by the engine yet (omni antennas only).
