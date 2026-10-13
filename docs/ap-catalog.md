# AP model catalog

Ondalux Heatmap ships a catalog of access point models grouped by vendor
(`crates/ondalux-core/catalog/ap-models.json`). Picking a model fills an AP's
radios (bands, transmit power, antenna gain) with the vendor's datasheet
values. Every value can still be edited per AP.

## Where the values come from

Every model has a `source` URL. Values were taken from the vendor's own
datasheet or specification page, retrieved on 2026-10-13. A few are marked
otherwise in their `notes`:

- **HPE Aruba Networking:** the official PDFs could not be downloaded
  automatically. Transmit power and gains come from the Aruba data sheet
  text as reproduced by a reseller. For the AP-635, the antenna gains were
  confirmed in the HPE data sheet itself.
- **Cisco Meraki:** the RF tables list per-rate values without labelling
  them aggregate or per chain. The highest listed value is used. Treat these
  two models as approximate.
- **Channel width:** when a datasheet does not state the maximum width per
  band, a typical maximum is used and the model's notes say so.

Models whose datasheet does not publish a maximum transmit power are **not**
included rather than guessed. Examples found during research: Ruckus R650,
Juniper Mist AP45, NETGEAR WAX620, Fortinet FortiAP 231F, Ruijie
RG-RAP2260(G). You can add them as custom models once you have their figures.

## How the figures are used

| Field | Meaning |
|---|---|
| `max_tx_power_dbm` | Maximum transmit power as published, per band |
| `tx_power_basis` | What that figure is: `aggregate_conducted` (sum of all chains), `per_chain_conducted`, `vendor_max` (unqualified "max TX power") or `eirp` |
| `antenna_gain_dbi` | Antenna gain per band. When the vendor publishes both a single-element peak and a combined/average pattern gain (Aruba, Huawei), the combined figure is used, since the engine models one omnidirectional radiator |
| `spatial_streams` | MIMO chains, e.g. 4 for 4x4:4 |
| `max_width` | Widest channel the model supports on that band |

The **design TX power** given to the engine is a conducted total:

- `aggregate_conducted` and `vendor_max` are used as-is;
- `per_chain_conducted` gets +10·log10(chains);
- `eirp` gets − antenna gain.

Datasheet maxima are the hardware ceiling. **Your country's regulatory
limits and the controller's configuration are usually lower.** Adjust the
power per AP to match your deployment, or create a custom model with your
region's values.

## Custom models

**Edit → AP catalog…** lets you:

- browse and search every model, with its values and source link;
- choose the model used by the AP tool for new APs (also selectable in the
  toolbar while the AP tool is active);
- create a custom model, or edit a built-in one as a custom copy. Custom
  models replace built-in ones with the same vendor and model name, and
  deleting the custom copy brings the built-in one back;
- import models from a JSON file, or export your custom models.

Custom models are saved to `<config dir>/ondalux/ap-models.json`:

| OS | Config dir |
|---|---|
| Windows | `%APPDATA%` |
| macOS | `~/Library/Application Support` |
| Linux | `$XDG_CONFIG_HOME` or `~/.config` |

## File format

```json
{
  "models": [
    {
      "vendor": "Ubiquiti",
      "model": "U6 Pro",
      "standard": "Wi-Fi 6",
      "environment": "indoor",
      "radios": [
        { "band": "2.4GHz", "max_tx_power_dbm": 22, "antenna_gain_dbi": 4, "spatial_streams": 2, "max_width": "40" },
        { "band": "5GHz", "max_tx_power_dbm": 26, "antenna_gain_dbi": 6, "spatial_streams": 4, "max_width": "160" }
      ],
      "tx_power_basis": "vendor_max",
      "source": "https://techspecs.ui.com/unifi/wifi/u6-pro",
      "notes": ""
    }
  ]
}
```

A bare list (`[ {...}, {...} ]`) is also accepted on import. Entries are
validated: vendor and model are required, power must be in −10…36 dBm, gain
in −5…25 dBi, streams in 1…16, and the width must exist in that band.

## Contributing models

Add an entry to `catalog/ap-models.json` with the vendor's datasheet URL as
`source`. `cargo test -p ondalux-core catalog` checks every entry
(validation, unique vendor + model, `https://` source).
