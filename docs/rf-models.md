# RF models used by HeatCraft

This document describes exactly what the prediction engine
(`crates/heatcraft-core`) computes, and what it does not. All formulas come
from public RF propagation theory and the IEEE 802.11 channel plan.

## 1. Received signal (RSSI)

For one radio at one point:

```
RSSI = EIRP + G_rx − PL(d, f) − Σ L_wall(band)
EIRP = P_tx + G_tx
```

| Term | Meaning | Where it comes from |
|---|---|---|
| `P_tx` | Conducted transmit power (dBm) | AP radio settings |
| `G_tx` | AP antenna gain (dBi), **omnidirectional** | AP radio settings |
| `G_rx` | Client antenna gain (dBi), default 0 | Prediction settings |
| `PL(d, f)` | Path loss over the 3D distance `d` | Section 2 |
| `L_wall` | Loss of each wall crossed by the straight AP→point path | Section 3 |

The distance is 3D: `d = √(d_xy² + (h_AP − h_client)²)`, where `h_client`
defaults to 1.0 m. Distances below 0.1 m are clamped to avoid the
singularity at `d = 0`.

The frequency `f` is the center of the occupied channel (for bonded 40/80/160
MHz channels, the center of the whole block).

## 2. Path loss

### Free space (Friis)

```
FSPL(d, f) = 20·log10(d [m]) + 20·log10(f [MHz]) − 27.55   [dB]
```

Reference values checked by unit tests:

| d | f | FSPL |
|---|---|---|
| 1 m | 2400 MHz | 40.05 dB |
| 1 m | 5000 MHz | 46.43 dB |
| 10 m | 2400 MHz | 60.05 dB |
| 100 m | 5800 MHz | 87.72 dB |

### Log-distance (default)

```
PL(d) = FSPL(d0, f) + 10·n·log10(d / d0)     for d ≥ d0
PL(d) = FSPL(d, f)                           for d < d0
```

`n` (path loss exponent) is configurable per band, `d0` defaults to 1 m.
With `n = 2` the model equals free space. Because walls are modelled
explicitly (a "multi-wall" approach), the default is `n = 2.0`; typical
values for open indoor spaces with furniture and people are 2.0–2.5.
Using large exponents (3–4) *and* explicit walls double-counts losses.

## 3. Wall attenuation (2D ray casting)

The straight segment from the AP to the point is intersected with every wall
on the plan. Each crossed wall subtracts its material's loss for that band,
**once**, regardless of the incidence angle and wall thickness.

A uniform grid index (1.5 m cells, Amanatides–Woo traversal) limits the
segment–segment tests to walls near the ray; a test compares it against
brute force on random walls and rays.

Default material losses (single crossing, dB):

| Material | 2.4 GHz | 5 GHz | 6 GHz |
|---|---|---|---|
| Drywall | 3 | 4 | 5 |
| Wood | 4 | 6 | 7 |
| Glass | 3 | 7 | 8 |
| Brick | 8 | 12 | 14 |
| Concrete | 12 | 18 | 21 |
| Metal | 25 | 30 | 32 |

These are typical figures from public measurement literature and vendor
design guides. Real walls vary widely (thickness, rebar, low-e coatings,
moisture), so every value is editable per project.

## 4. Noise, SNR and interference

Thermal noise floor for the channel width `B`, with receiver noise figure
`NF` (default 6 dB):

```
N = −174 dBm/Hz + 10·log10(B [Hz]) + NF        (20 MHz → −95 dBm)
SNR = RSSI_best − N
```

The **serving radio** at a point is the strongest radio in the selected band.
Every other radio (not on the same AP) whose occupied spectrum overlaps the
serving channel adds interference, weighted by the fraction of its power
that falls inside the serving channel:

```
I = Σ_j  P_j · overlap(j, serving) / width(j)     (linear mW)
SINR = RSSI_best − 10·log10(N_mW + I)
```

This covers both **co-channel** interference (full overlap) and
**adjacent/partially overlapping** channels (e.g. 2.4 GHz channels 1 and 3,
or a 20 MHz channel inside another AP's 80 MHz block). Interference is
treated as continuous, worst case: real CSMA/CA airtime sharing is not
modelled.

## 5. Heat map metrics

| Metric | Value per point |
|---|---|
| Signal strength | Strongest RSSI in the band (dBm) |
| SNR | Strongest RSSI over the noise floor (dB) |
| SINR | Strongest RSSI over noise + interference (dB) |
| AP count | Number of APs heard at ≥ the minimum signal |
| Channel overlap | Number of radios on an overlapping channel heard above the CCA threshold (default −82 dBm) |
| Coverage requirement | Meets both minimum signal and minimum SNR (default −67 dBm / 25 dB, voice/video) |

## 6. Channel plan

| Band | Center frequency | Channels |
|---|---|---|
| 2.4 GHz | 2407 + 5·ch MHz (ch 14 = 2484) | 1–14 |
| 5 GHz | 5000 + 5·ch MHz | 36–177 (20 MHz primaries) |
| 6 GHz | 5950 + 5·ch MHz | 1, 5, 9 … 233 |

Bonded channels are the aligned block containing the primary channel
(5 GHz blocks start at 36 or 149; 6 GHz at channel 1). 2.4 GHz HT40 places the
secondary above for channels 1–7 and below otherwise. The overlapping
"320-2" 6 GHz plan is not modelled.

## 7. Known limitations

- **2D model.** One floor; no inter-floor attenuation.
- **No reflection, diffraction or multipath.** Corridors acting as
  waveguides and signal bending around corners are not predicted, so
  shadowed areas behind thick walls are pessimistic.
- **Omnidirectional antennas only.** The AP orientation is stored but unused.
- **No clutter model** besides the path loss exponent (furniture, people,
  shelving).
- **Wall loss ignores incidence angle and thickness.**
- **Interference ignores the spectral mask** of non-overlapping adjacent
  channels and airtime sharing.

Phase 2 (field survey) is meant to calibrate these predictions against real
measurements.

## 8. Performance

The engine keeps one RSSI layer per radio and recomputes only the layers
whose inputs changed (position, height, channel, EIRP). Layers are computed
in parallel by rows with rayon. Benchmark (`cargo bench -p heatcraft-core`),
100 × 100 m plan at 20 px/m (2000 × 2000 px) sampled every 0.2 m
(250 000 cells), ~440 walls, 20 dual-band APs, on an Intel Core 5 210H
(12 threads):

| Operation | Time |
|---|---|
| One radio layer | ~27 ms |
| Combine 20 layers into 5 GHz metrics | ~11 ms |
| **Move one AP (2 layers + metrics)** | **~50 ms** |

The UI runs the computation on a worker thread and always processes only
the latest request, so dragging stays smooth even when an update takes longer
than a frame. GPU compute (wgpu) was not needed to reach the < 100 ms target.
