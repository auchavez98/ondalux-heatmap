//! Wall detection on a floor plan bitmap ("ink" mask).
//!
//! Classical image processing, no machine learning (see
//! `docs/wall-detection.md`):
//!
//! 1. Walls drawn as two thin parallel lines are filled into solid bands.
//! 2. Every ink pixel gets its horizontal and vertical run length. A pixel
//!    belongs to a horizontal wall if its horizontal run is long and its
//!    vertical run (the thickness) is within the wall thickness range;
//!    vertical walls likewise. Text, furniture, door arcs and thin
//!    dimension lines fail one of the two tests.
//! 3. Candidate pixels are grouped into bands; each band becomes one
//!    segment along its center line, with its thickness.
//! 4. Collinear pieces are merged across small gaps (crossing walls) and
//!    endpoints are snapped onto perpendicular walls so corners close.
//!
//! Only horizontal and vertical walls are detected; diagonal and curved
//! walls have to be drawn by hand. Results are a proposal for the user to
//! review.

use crate::geometry::Vec2;

/// Binary image: `true` where the plan has ink (dark lines or fills).
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<bool>,
}

impl Mask {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![false; width * height],
        }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.data[y * self.width + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: bool) {
        self.data[y * self.width + x] = v;
    }

    /// Ink where luminance is below `threshold`.
    pub fn from_luma(width: usize, height: usize, luma: &[u8], threshold: u8) -> Self {
        Self {
            width,
            height,
            data: luma.iter().map(|&l| l < threshold).collect(),
        }
    }

    /// Otsu's threshold for a luminance histogram, clamped so that very
    /// light plans do not turn background noise into ink.
    pub fn otsu_threshold(luma: &[u8]) -> u8 {
        let mut hist = [0u64; 256];
        for &l in luma {
            hist[l as usize] += 1;
        }
        let total = luma.len() as f64;
        let sum_all: f64 = hist
            .iter()
            .enumerate()
            .map(|(i, &c)| i as f64 * c as f64)
            .sum();
        let (mut w_b, mut sum_b, mut best, mut best_t) = (0.0, 0.0, 0.0, 128u8);
        for (t, &c) in hist.iter().enumerate() {
            w_b += c as f64;
            if w_b == 0.0 {
                continue;
            }
            let w_f = total - w_b;
            if w_f == 0.0 {
                break;
            }
            sum_b += t as f64 * c as f64;
            let m_b = sum_b / w_b;
            let m_f = (sum_all - sum_b) / w_f;
            let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
            if between > best {
                best = between;
                best_t = t as u8;
            }
        }
        best_t.clamp(60, 200)
    }

    /// Downscales by an integer factor; a block is ink if any pixel is
    /// (keeps thin lines).
    pub fn downscale(&self, factor: usize) -> Mask {
        if factor <= 1 {
            return self.clone();
        }
        let (w, h) = (self.width.div_ceil(factor), self.height.div_ceil(factor));
        let mut out = Mask::new(w, h);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    out.set(x / factor, y / factor, true);
                }
            }
        }
        out
    }

    fn transposed(&self) -> Mask {
        let mut out = Mask::new(self.height, self.width);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    out.set(y, x, true);
                }
            }
        }
        out
    }

    pub fn ink_count(&self) -> usize {
        self.data.iter().filter(|&&b| b).count()
    }
}

/// Detection settings, in pixels of the mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectParams {
    /// Shorter straight pieces are ignored.
    pub min_length_px: f32,
    pub min_thickness_px: f32,
    pub max_thickness_px: f32,
    /// Two long parallel lines closer than this are treated as the two
    /// faces of one wall and filled. 0 disables it.
    pub max_gap_px: f32,
}

/// A detected wall: center line and thickness, in mask pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectedWall {
    pub a: Vec2,
    pub b: Vec2,
    pub thickness_px: f32,
}

impl DetectedWall {
    pub fn length(&self) -> f32 {
        self.a.distance(self.b)
    }

    fn horizontal(&self) -> bool {
        (self.a.y - self.b.y).abs() < (self.a.x - self.b.x).abs()
    }
}

/// Run length of every pixel along rows (0 for background).
fn row_runs(m: &Mask) -> Vec<u16> {
    let mut out = vec![0u16; m.data.len()];
    for y in 0..m.height {
        let row = y * m.width;
        let mut x = 0;
        while x < m.width {
            if !m.data[row + x] {
                x += 1;
                continue;
            }
            let start = x;
            while x < m.width && m.data[row + x] {
                x += 1;
            }
            let len = (x - start).min(u16::MAX as usize) as u16;
            out[row + start..row + x].fill(len);
        }
    }
    out
}

/// Run length of every pixel along columns, computed row by row.
fn col_runs(m: &Mask) -> Vec<u16> {
    let (w, h) = (m.width, m.height);
    let mut down = vec![0u16; m.data.len()];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if m.data[i] {
                down[i] = if y > 0 {
                    down[i - w].saturating_add(1)
                } else {
                    1
                };
            }
        }
    }
    let mut out = vec![0u16; m.data.len()];
    for y in (0..h).rev() {
        for x in 0..w {
            let i = y * w + x;
            if m.data[i] {
                out[i] = if y + 1 < h && m.data[i + w] {
                    out[i + w]
                } else {
                    down[i]
                };
            }
        }
    }
    out
}

/// Calls `visit(x, first_gap_row, gap)` for every pair of long horizontal
/// lines that look like the two faces of one wall in column `x`: both
/// thin, of similar weight, and at most `max_gap` apart.
fn double_line_pairs(
    m: &Mask,
    runs: &[u16],
    min_len: u16,
    max_gap: usize,
    mut visit: impl FnMut(usize, usize, usize),
) {
    let w = m.width;
    for x in 0..w {
        // Previous line in this column: (first row, thickness).
        let mut prev: Option<(usize, usize)> = None;
        let mut y = 0;
        while y < m.height {
            if runs[y * w + x] < min_len {
                y += 1;
                continue;
            }
            let start = y;
            while y < m.height && runs[y * w + x] >= min_len {
                y += 1;
            }
            let thick = y - start;
            if let Some((p_start, p_thick)) = prev {
                let gap = start - (p_start + p_thick);
                let thin = |t: usize| t <= 6 && t <= gap.max(2);
                let similar = thick.max(p_thick) <= 2 * thick.min(p_thick) + 1;
                if gap >= 2 && gap <= max_gap && thin(thick) && thin(p_thick) && similar {
                    visit(x, p_start + p_thick, gap);
                }
            }
            prev = Some((start, thick));
        }
    }
}

/// Fills the light gap between the two faces of double-line walls.
fn fill_double_lines_rows(m: &Mask, runs: &[u16], min_len: u16, max_gap: usize) -> Mask {
    let mut out = m.clone();
    let w = m.width;
    double_line_pairs(m, runs, min_len, max_gap, |x, y0, gap| {
        for y in y0..y0 + gap {
            out.data[y * w + x] = true;
        }
    });
    out
}

fn fill_double_lines(m: &Mask, min_len: u16, max_gap: usize) -> Mask {
    if max_gap == 0 {
        return m.clone();
    }
    let rows = fill_double_lines_rows(m, &row_runs(m), min_len, max_gap);
    let t = m.transposed();
    let cols = fill_double_lines_rows(&t, &row_runs(&t), min_len, max_gap).transposed();
    let mut out = rows;
    for (o, c) in out.data.iter_mut().zip(cols.data) {
        *o |= c;
    }
    out
}

/// Horizontal walls of a mask: bands of pixels with a long row run and a
/// column run (thickness) within range.
fn horizontal_bands(m: &Mask, p: &DetectParams) -> Vec<DetectedWall> {
    let hr = row_runs(m);
    let vr = col_runs(m);
    let min_len = p.min_length_px.max(2.0) as u16;
    let (tmin, tmax) = (
        p.min_thickness_px.max(1.0).ceil() as u16,
        p.max_thickness_px as u16,
    );
    let w = m.width;
    // Candidate runs per row: (x0, x1) with x1 exclusive.
    let mut rows: Vec<Vec<(usize, usize)>> = vec![Vec::new(); m.height];
    let min_piece = (tmin as usize * 2).max(3);
    for (y, row_out) in rows.iter_mut().enumerate() {
        let mut x = 0;
        while x < w {
            let i = y * w + x;
            let cand = |i: usize| hr[i] >= min_len && vr[i] >= tmin && vr[i] <= tmax;
            if !cand(i) {
                x += 1;
                continue;
            }
            let start = x;
            while x < w && cand(y * w + x) {
                x += 1;
            }
            if x - start >= min_piece {
                row_out.push((start, x));
            }
        }
    }
    // Group overlapping runs of consecutive rows into bands (union-find).
    let mut ids: Vec<Vec<usize>> = rows.iter().map(|r| vec![0; r.len()]).collect();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for y in 0..m.height {
        for k in 0..rows[y].len() {
            let id = parent.len();
            parent.push(id);
            ids[y][k] = id;
            if y == 0 {
                continue;
            }
            let (a0, a1) = rows[y][k];
            for j in 0..rows[y - 1].len() {
                let (b0, b1) = rows[y - 1][j];
                let overlap = a1.min(b1) as isize - a0.max(b0) as isize;
                let shorter = (a1 - a0).min(b1 - b0) as isize;
                if overlap * 2 >= shorter {
                    let (ra, rb) = (find(&mut parent, id), find(&mut parent, ids[y - 1][j]));
                    if ra != rb {
                        parent[ra] = rb;
                    }
                }
            }
        }
    }
    struct Band {
        y0: usize,
        y1: usize,
        starts: Vec<usize>,
        ends: Vec<usize>,
    }
    let mut bands: std::collections::HashMap<usize, Band> = Default::default();
    for y in 0..m.height {
        for (k, &(x0, x1)) in rows[y].iter().enumerate() {
            let root = find(&mut parent, ids[y][k]);
            let b = bands.entry(root).or_insert(Band {
                y0: y,
                y1: y,
                starts: Vec::new(),
                ends: Vec::new(),
            });
            b.y1 = y;
            b.starts.push(x0);
            b.ends.push(x1);
        }
    }
    let median = |v: &mut Vec<usize>| {
        v.sort_unstable();
        v[v.len() / 2] as f32
    };
    let mut out = Vec::new();
    for (_, mut b) in bands {
        let thickness = (b.y1 - b.y0 + 1) as f32;
        if thickness < p.min_thickness_px || thickness > p.max_thickness_px {
            continue;
        }
        let x0 = median(&mut b.starts);
        let x1 = median(&mut b.ends);
        if x1 - x0 < p.min_length_px * 0.5 {
            continue;
        }
        let y = (b.y0 + b.y1 + 1) as f32 / 2.0;
        out.push(DetectedWall {
            a: Vec2::new(x0, y),
            b: Vec2::new(x1, y),
            thickness_px: thickness,
        });
    }
    out
}

/// Merges collinear pieces separated by small gaps (e.g. where another
/// wall crosses). `walls` must all be horizontal (y constant).
fn merge_collinear(mut walls: Vec<DetectedWall>, max_gap_factor: f32) -> Vec<DetectedWall> {
    walls.sort_by(|a, b| a.a.y.total_cmp(&b.a.y).then(a.a.x.total_cmp(&b.a.x)));
    let mut changed = true;
    while changed {
        changed = false;
        let mut out: Vec<DetectedWall> = Vec::with_capacity(walls.len());
        'next: for w in walls {
            for o in out.iter_mut() {
                let t = o.thickness_px.max(w.thickness_px);
                if (o.a.y - w.a.y).abs() > t / 2.0 {
                    continue;
                }
                let gap = w.a.x.max(o.a.x) - w.b.x.min(o.b.x);
                if gap <= t * max_gap_factor {
                    let (lo, lw) = (o.length(), w.length());
                    let total = (lo + lw).max(1e-3);
                    let y = (o.a.y * lo + w.a.y * lw) / total;
                    o.thickness_px = (o.thickness_px * lo + w.thickness_px * lw) / total;
                    o.a = Vec2::new(o.a.x.min(w.a.x), y);
                    o.b = Vec2::new(o.b.x.max(w.b.x), y);
                    changed = true;
                    continue 'next;
                }
            }
            out.push(w);
        }
        walls = out;
    }
    walls
}

fn transpose_wall(w: &DetectedWall) -> DetectedWall {
    DetectedWall {
        a: Vec2::new(w.a.y, w.a.x),
        b: Vec2::new(w.b.y, w.b.x),
        thickness_px: w.thickness_px,
    }
}

/// Moves endpoints of horizontal walls onto the center line of a nearby
/// vertical wall (and vice versa), so corners and T-junctions close.
fn snap_endpoints(h: &mut [DetectedWall], v: &mut [DetectedWall]) {
    let snap = |h: &mut [DetectedWall], v: &[DetectedWall]| {
        for w in h.iter_mut() {
            for end in 0..2 {
                let p = if end == 0 { w.a } else { w.b };
                let mut best: Option<(f32, f32)> = None;
                for o in v {
                    let x = o.a.x;
                    let (y0, y1) = (o.a.y.min(o.b.y), o.a.y.max(o.b.y));
                    let reach = o.thickness_px / 2.0 + w.thickness_px * 1.5 + 2.0;
                    let dx = (x - p.x).abs();
                    let inside =
                        p.y >= y0 - w.thickness_px * 1.5 && p.y <= y1 + w.thickness_px * 1.5;
                    if dx <= reach && inside && best.is_none_or(|(d, _)| dx < d) {
                        best = Some((dx, x));
                    }
                }
                if let Some((_, x)) = best {
                    if end == 0 {
                        w.a.x = x;
                    } else {
                        w.b.x = x;
                    }
                }
            }
        }
    };
    snap(h, v);
    let mut ht: Vec<_> = h.iter().map(transpose_wall).collect();
    let mut vt: Vec<_> = v.iter().map(transpose_wall).collect();
    snap(&mut vt, &ht);
    for (dst, src) in v.iter_mut().zip(&vt) {
        *dst = transpose_wall(src);
    }
    ht.clear();
}

/// Detects horizontal and vertical walls in `mask`.
pub fn detect(mask: &Mask, p: &DetectParams) -> Vec<DetectedWall> {
    let min_len = p.min_length_px.max(2.0) as u16;
    let filled = fill_double_lines(mask, min_len, p.max_gap_px.max(0.0) as usize);
    let h = horizontal_bands(&filled, p);
    let t = filled.transposed();
    let v: Vec<DetectedWall> = horizontal_bands(&t, p);
    let mut h = merge_collinear(h, 1.2);
    let mut v: Vec<DetectedWall> = merge_collinear(v, 1.2).iter().map(transpose_wall).collect();
    snap_endpoints(&mut h, &mut v);
    h.into_iter()
        .chain(v)
        .filter(|w| w.length() >= p.min_length_px)
        .map(|mut w| {
            // Keep a, b ordered left→right / top→bottom.
            if (w.horizontal() && w.a.x > w.b.x) || (!w.horizontal() && w.a.y > w.b.y) {
                std::mem::swap(&mut w.a, &mut w.b);
            }
            w
        })
        .collect()
}

/// Guesses parameters from the plan itself:
/// - the double-line gap is the most common distance between pairs of thin
///   parallel lines (walls drawn by their two faces), if there is a clear
///   peak;
/// - the plan's line weight is the most common thickness of long thin ink
///   (by length); walls must be clearly thicker than that;
/// - the typical wall thickness is the most common thickness above the
///   line weight, by area.
pub fn estimate_params(mask: &Mask) -> DetectParams {
    let side = mask.width.max(mask.height).max(1) as f32;
    let long = (side / 60.0).max(12.0) as u16;
    // Double-line walls are at most ~1/80 of the plan apart.
    let gap_cap = (side / 80.0).max(6.0) as usize;
    let cap = (side / 25.0).max(8.0) as usize;

    let mut gaps = vec![0u64; gap_cap + 1];
    let mut count_gaps = |m: &Mask| {
        let runs = row_runs(m);
        double_line_pairs(m, &runs, long, gap_cap, |_, _, g| gaps[g] += 1);
    };
    count_gaps(mask);
    count_gaps(&mask.transposed());
    let (gap_peak, gap_count) = gaps
        .iter()
        .enumerate()
        .max_by_key(|(_, &c)| c)
        .map_or((0, 0), |(g, &c)| (g, c));
    let gap_total: u64 = gaps.iter().sum();
    let pixels_long = side as u64 / 2;
    let max_gap = if gap_count > pixels_long && gap_count * 4 > gap_total {
        (gap_peak as f32 * 1.3).ceil()
    } else {
        0.0
    };

    let filled = fill_double_lines(mask, long, max_gap as usize);
    let hr = row_runs(&filled);
    let vr = col_runs(&filled);
    // Area of long ink by thickness.
    let mut area = vec![0u64; cap + 1];
    for i in 0..filled.data.len() {
        if !filled.data[i] {
            continue;
        }
        if hr[i] >= long && (vr[i] as usize) <= cap {
            area[vr[i] as usize] += 1;
        }
        if vr[i] >= long && (hr[i] as usize) <= cap {
            area[hr[i] as usize] += 1;
        }
    }
    // Length = area / thickness; the line weight is the thickness with the
    // most length among thin ink.
    let length = |t: usize| area.get(t).map_or(0, |a| a / t as u64);
    let max_len = (1..=3).map(length).max().unwrap_or(0);
    let line_weight = (1..=3)
        .find(|&t| length(t) * 4 >= max_len && max_len > 0)
        .unwrap_or(1) as f32;
    let min_t = (line_weight * 1.5 + 1.0).max(2.0);
    let typical = area
        .iter()
        .enumerate()
        .skip(min_t.ceil() as usize)
        .max_by_key(|(_, &c)| c)
        .map_or(min_t * 2.0, |(t, _)| t as f32);
    DetectParams {
        min_length_px: (side / 100.0).max(min_t * 6.0).max(10.0),
        min_thickness_px: min_t,
        max_thickness_px: (typical * 3.0).max(min_t + 6.0),
        max_gap_px: max_gap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(m: &mut Mask, x0: usize, y0: usize, x1: usize, y1: usize) {
        for y in y0..y1 {
            for x in x0..x1 {
                m.set(x, y, true);
            }
        }
    }

    /// A 400×300 room drawn with solid 8 px walls, a door gap and some
    /// "text" (small blobs) inside.
    fn solid_room() -> Mask {
        let mut m = Mask::new(500, 400);
        rect(&mut m, 50, 50, 450, 58); // top
        rect(&mut m, 50, 342, 200, 350); // bottom-left
        rect(&mut m, 260, 342, 450, 350); // bottom-right (door gap 200..260)
        rect(&mut m, 50, 50, 58, 350); // left
        rect(&mut m, 442, 50, 450, 350); // right
        rect(&mut m, 250, 58, 256, 200); // inner partition (6 px)
        for i in 0..10 {
            rect(&mut m, 100 + i * 9, 150, 104 + i * 9, 160); // text
        }
        rect(&mut m, 120, 250, 380, 251); // thin dimension line
        m
    }

    fn params() -> DetectParams {
        DetectParams {
            min_length_px: 30.0,
            min_thickness_px: 4.0,
            max_thickness_px: 20.0,
            max_gap_px: 0.0,
        }
    }

    fn near(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn finds_solid_walls_and_ignores_text_and_thin_lines() {
        let walls = detect(&solid_room(), &params());
        let horizontal: Vec<_> = walls.iter().filter(|w| w.horizontal()).collect();
        let vertical: Vec<_> = walls.iter().filter(|w| !w.horizontal()).collect();
        // Top, two bottom pieces (door gap kept); left, right, partition.
        assert_eq!(horizontal.len(), 3, "{walls:?}");
        assert_eq!(vertical.len(), 3, "{walls:?}");
        let top = horizontal.iter().find(|w| w.a.y < 100.0).unwrap();
        assert!(near(top.a.y, 54.0, 1.0) && near(top.thickness_px, 8.0, 0.5));
        // Corners snap to the center line of the side walls.
        assert!(
            near(top.a.x, 54.0, 1.5) && near(top.b.x, 446.0, 1.5),
            "{top:?}"
        );
        assert!(horizontal
            .iter()
            .all(|w| w.a.y > 40.0 && (w.a.y < 70.0 || w.a.y > 330.0)));
        let partition = vertical.iter().find(|w| near(w.a.x, 253.0, 1.0)).unwrap();
        assert!(near(partition.thickness_px, 6.0, 0.5));
        // It meets the top wall.
        assert!(near(partition.a.y, 54.0, 1.5), "{partition:?}");
    }

    #[test]
    fn fills_double_line_walls() {
        let mut m = Mask::new(400, 300);
        // A wall drawn as two 2 px lines, 10 px apart.
        rect(&mut m, 40, 100, 360, 102);
        rect(&mut m, 40, 112, 360, 114);
        let p = DetectParams {
            max_gap_px: 14.0,
            min_thickness_px: 6.0,
            ..params()
        };
        let walls = detect(&m, &p);
        assert_eq!(walls.len(), 1, "{walls:?}");
        assert!(near(walls[0].a.y, 107.0, 1.0));
        assert!(near(walls[0].thickness_px, 14.0, 0.5));
        // Without filling, two 2 px lines are too thin to be walls.
        assert!(detect(&m, &params()).is_empty());
    }

    #[test]
    fn estimates_typical_thickness() {
        let p = estimate_params(&solid_room());
        assert!(
            p.min_thickness_px <= 6.0 && p.max_thickness_px >= 8.0,
            "{p:?}"
        );
        assert_eq!(p.max_gap_px, 0.0);
    }

    #[test]
    fn otsu_splits_dark_and_light() {
        let mut luma = vec![250u8; 1000];
        luma[..100].fill(20);
        let t = Mask::otsu_threshold(&luma);
        assert!(t > 20 && t < 250, "{t}");
    }

    #[test]
    fn downscale_keeps_thin_lines() {
        let mut m = Mask::new(100, 100);
        rect(&mut m, 0, 51, 100, 52);
        let d = m.downscale(4);
        assert_eq!((d.width, d.height), (25, 25));
        assert!((0..25).all(|x| d.get(x, 12)));
    }
}
