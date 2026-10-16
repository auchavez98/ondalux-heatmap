# Automatic wall detection

**Beta feature.** With the option "Detect walls automatically when importing
a plan" checked (welcome screen or the detection window; off by default,
saved in `<config>/ondalux/ui.json`), Ondalux proposes walls as soon as a
floor plan is imported. The proposal is shown over the plan (orange = thick walls, blue = thin walls).
The user picks a material for each group, adjusts the sensitivity and
accepts or cancels. It is also available any time, checked or not, from
**Edit → Detect walls from the plan (beta)…** or the Walls tool card.

Detection is classical image processing (no machine learning, no extra
runtime). Code: `ondalux-core/src/walldetect.rs` (algorithm) and
`ondalux-io/src/walls.rs` + `pdf.rs` (inputs).

## Inputs

1. **Vector PDF (preferred).** If the plan came from a PDF, the first page
   is drawn again into a black-and-white mask with only its dark strokes
   and fills: text, images and dashed lines (hidden or overhead elements)
   are left out, hairlines are one pixel wide and white fills erase. The
   original PDF is kept in the project (`floorplan-source.pdf`) so
   detection can be re-run later. A scanned PDF (almost no line art) falls
   back to the image.
2. **Image (PNG/JPEG, scanned PDF).** Luminance is thresholded with
   Otsu's method (clamped to 60–200).

Masks are processed at most 3000 px per side (blocks keep thin lines);
results are scaled back to plan pixels.

## Algorithm

1. **Double-line walls.** Two long, thin, similar-weight parallel lines
   close to each other are the two faces of one wall: the gap between them
   is filled. The usual gap is estimated as the most frequent distance
   between such pairs; it is only used if there is a clear peak.
2. **Candidates.** For every ink pixel, the horizontal and vertical run
   lengths are computed. A pixel belongs to a horizontal wall if its
   horizontal run is long and its vertical run (the thickness) is within
   the wall thickness range; vertical walls likewise. Text, furniture,
   door swings and thin dimension lines fail one of the two tests.
3. **Bands.** Candidate runs on consecutive rows that overlap are grouped;
   each group becomes one segment along its center line with its thickness
   (median ends, robust to slanted joints).
4. **Clean-up.** Collinear pieces are merged across gaps up to 1.2× their
   thickness (where another wall crosses); door openings, which are wider,
   stay open. Endpoints are snapped onto the center line of perpendicular
   walls so corners and T-junctions close.
5. **Diagonal walls.** A reduced copy of the mask (≤ 700 px) is rotated in
   2° steps between 10° and 84°; each angle is scored by the long,
   wall-thick row runs it produces (a run scores its length, so a wall
   exactly at that angle scores much more than one a few degrees off).
   Clear local peaks (refined to 0.5°, at most four) are dominant angles.
   For each, the full mask is rotated so those walls become horizontal or
   vertical, steps 1–4 run again, and walls really at that angle are
   rotated back and kept unless already found.
6. **Curved walls.** Straight walls found so far are erased from the mask
   (with a small margin), as are areas near solid fills. What is left is
   thinned to one-pixel center lines (Zhang–Suen); a chamfer distance
   transform gives the local thickness, and only center-line pixels of
   wall thickness remain. Chains are traced from their end points; those
   long and wide enough (≥ 3× the minimum length in both length and
   extent, which drops letters and symbols) are simplified with
   Douglas–Peucker (tolerance: half the wall thickness) into straight
   pieces with the chain's median thickness.
7. **Joining.** Loose ends of diagonal and curved walls are moved onto the
   nearest wall within reach, so they connect.

### Automatic parameters

- **Line weight**: the thinnest thickness (1–3 px) with substantial length
  among long straight ink. Walls must be thicker than `1.5 × line weight + 1`.
- **Typical wall**: the most common thickness above that, by area; the
  maximum accepted is 3× typical.
- **Minimum length**: 1/100 of the plan side, at least 6× the minimum
  thickness.

The **sensitivity** slider scales minimum length and thickness (stricter:
only long clear walls; more permissive: short or thin walls, with more
risk of picking up furniture).

## Materials

Walls clearly thicker than the median (≥ 1.35×) are "thick" (default
concrete), the rest "thin" (default drywall). Both are editable before
adding, and per wall afterwards.

## Limitations

- Diagonal walls are found only at angles that carry a lot of wall (up to
  four dominant angles per plan); a single short diagonal may be missed.
- Curved walls are approximated by straight pieces (within half a wall
  thickness of the curve). Curves drawn as two thin lines are not filled,
  so only solid or thick-stroke curves are found. Pieces at the ends of an
  arc that are nearly straight may come out as short straight walls.
- Windows, doors and glass partitions are not classified; openings stay as
  gaps.
- Hatched or heavily furnished raster plans, photos and low-contrast scans
  give worse results; use a PDF exported from CAD when possible.
- Measured on a synthetic CAD-style plan (exterior poché walls, partitions,
  a double-line wall, a 45° partition, a curved reception wall, doors,
  text, desks, dimensions, a dashed beam): every wall found, including the
  diagonal and the curve, with no false positives, both from the PDF line
  art and from the rasterized image. Run on your own plans with
  `cargo run --release -p ondalux-io --example detect_walls -- plan.pdf out.png`.
