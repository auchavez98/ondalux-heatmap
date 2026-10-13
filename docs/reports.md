# Reports and exports

**File → Export** offers:

| Export | Content |
|---|---|
| Heat map image (PNG) | The heat map on screen (metric, band and source: predicted, measured or difference) over the plan, with walls, APs and a legend, at the plan's resolution (up to 4096 px wide) |
| Report (PDF) | Design report, see below. Generated in the background |
| Access points (CSV) | One row per radio: `ap_id, name, vendor, model, x_m, y_m, height_m, radio, band, channel, width_mhz, tx_power_dbm, antenna_gain_dbi, eirp_dbm, enabled` |
| Survey measurements (CSV) | Active survey, format in [survey-csv.md](survey-csv.md) |

The same report can be produced without the UI:

```bash
cargo run --release -p ondalux-report --example report -- project.lux out_dir es --pages
```

## PDF report

Options: language (Spanish or English), author, bands, and comparison with
the active survey.

1. **Summary**: date, plan size and scale, number of APs and walls,
   coverage requirements, propagation model, and a table per band with
   coverage, number of coverage gaps and overlap.
2. **One section per band**:
   - signal strength map;
   - coverage map against the requirements, with gaps tinted red and
     numbered;
   - table of areas with poor coverage (area, center, best signal);
   - recommendations.
3. **Access point table**: model, position, height, and each radio's
   channel/width and Tx + gain = EIRP.
4. **Method and limitations**.

The PDF uses the standard Helvetica fonts (nothing embedded) with WinAnsi
encoding, so Spanish text works. Maps are embedded as lossless images.

## Analysed area

Coverage percentages and gaps are computed **inside the bounding box of
the walls** (the building footprint), so empty areas around the building
on the plan do not count as uncovered. Without walls, the whole plan is
analysed.

## Coverage gaps

A gap is a 4-connected group of grid cells that do not meet both the
minimum signal and the minimum SNR. Gaps smaller than 1 m² are ignored.
Gaps are numbered by decreasing area. For each gap the report gives area,
center (centroid) and the best signal found inside it.

## Recommendation rules

Every recommendation comes from one explicit rule (no AI):

| Rule | Condition | Severity |
|---|---|---|
| No access points | No enabled radio in the band | Critical |
| Low coverage | Coverage < 95 % of the analysed area (< 80 %: critical) | Warning / Critical |
| Add an AP near a gap | Coverage < 95 %: for the 3 largest gaps, suggest an AP near their center | Warning |
| Minor gaps | Coverage ≥ 95 % but gaps ≥ 1 m² exist | Info |
| Channel overlap | > 20 % of covered cells hear ≥ 2 radios on overlapping channels above the CCA threshold | Warning |
| Co-channel pair | Two APs on overlapping channels where one hears the other above CCA (predicted at the other AP's position) | Warning |
| 2.4 GHz channel plan | 2.4 GHz radios outside channels 1 / 6 / 11 | Info |
| 2.4 GHz width | 2.4 GHz radios using 40 MHz | Info |
| Reduce channel width | 5/6 GHz: ≥ 4 radios, overlap warning active, and radios at ≥ 80 MHz | Info |
| Model bias | With a survey: mean and RMSE of measured − predicted best signal at survey points | Warning |
| All good | None of the above | Info |

Thresholds are constants in `crates/ondalux-core/src/analysis.rs`.

## Performance

Measured with `cargo bench -p ondalux-report` on an Intel Core 5 210H:

| Operation | Time |
|---|---|
| Heat map image, 1600 × 1000 px | ~120 ms |
| Full PDF, 80 × 50 m, 6 APs, 2 bands | ~0.56 s |
