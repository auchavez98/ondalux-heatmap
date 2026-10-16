# Changelog

All notable changes to Ondalux Heatmap. Also published at https://ondaluxheatmap.com/changelog/
(Spanish: https://ondaluxheatmap.com/es/novedades/).

## 0.2.0 — 2026-10-16

**3D, multi-floor buildings and wall thickness**

- 3D view (D): walls extruded at their real thickness over the heat map, access points at their height; orbit, pan and zoom.
- Multi-floor buildings: one plan per floor, floor height, slab loss and plan offset. APs up to two floors above and below add their signal through the slabs.
- A PDF with several pages becomes one floor per page. Calibrating one floor applies the scale to the floors of the same size.
- Wall thickness in cm, per wall and for new walls. The loss of solid materials (concrete, block, brick, drywall, wood) follows the thickness.
- Wall detection (beta) now finds diagonal and curved walls, and keeps the measured thickness.
- 14 Fortinet FortiAP models (Wi-Fi 6, 6E and 7): 119 models from 15 vendors in the catalog.
- PDF reports cover every floor of the building.
- Help menu: website, GitHub repository, report a problem and check for updates. The app tells you when a new version is out; you decide whether to download it.
- Project files are now format 3; projects from 0.1.0 open as a single floor.

## 0.1.0 — 2026-10-14

**First release**

- Predictive design on floor plans (PNG, JPG, PDF) with scale calibration, walls and a material library per band.
- Heat maps for 2.4, 5 and 6 GHz: signal, SNR, SINR, APs heard, channel overlap and requirements met.
- AP catalog with 105 models from 14 vendors, plus custom models; ceiling or wall mounting with direction.
- Automatic wall detection (beta) from vector PDFs or images.
- On-site survey with Windows WiFi scanning, IDW or kriging interpolation, and predicted vs. measured comparison.
- PDF reports (English or Spanish), PNG and CSV exports. Interface in English and Spanish.
