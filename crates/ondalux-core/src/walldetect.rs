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
//! 5. Diagonal walls: the dominant non-axis angles are found by rotating a
//!    reduced copy of the mask in small steps; for each one the mask is
//!    rotated so those walls become horizontal, steps 1–4 run again and the
//!    results are rotated back.
//! 6. Curved walls: the ink left after removing the straight walls is
//!    thinned to its center line; pieces with wall thickness (from a
//!    distance transform) and enough length become polylines simplified
//!    with Douglas–Peucker, one wall per straight piece.
//!
//! Results are a proposal for the user to review.

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

/// Horizontal and vertical walls (the base algorithm), unfiltered.
fn detect_axis(mask: &Mask, p: &DetectParams) -> Vec<DetectedWall> {
    let min_len = p.min_length_px.max(2.0) as u16;
    let filled = fill_double_lines(mask, min_len, p.max_gap_px.max(0.0) as usize);
    let h = horizontal_bands(&filled, p);
    let t = filled.transposed();
    let v: Vec<DetectedWall> = horizontal_bands(&t, p);
    let mut h = merge_collinear(h, 1.2);
    let mut v: Vec<DetectedWall> = merge_collinear(v, 1.2).iter().map(transpose_wall).collect();
    snap_endpoints(&mut h, &mut v);
    h.into_iter().chain(v).collect()
}

/// Detects walls in `mask`: horizontal and vertical walls first, then walls
/// at other dominant angles (diagonals) and finally curved walls, which are
/// approximated by straight pieces.
pub fn detect(mask: &Mask, p: &DetectParams) -> Vec<DetectedWall> {
    let mut walls = detect_axis(mask, p);

    // Diagonals: rotate so they become horizontal, detect, rotate back.
    for angle in dominant_angles(mask, p) {
        let rot = Rotated::new(mask, angle);
        for w in detect_axis(&rot.mask, p) {
            let w = DetectedWall {
                a: rot.to_source(w.a),
                b: rot.to_source(w.b),
                thickness_px: w.thickness_px,
            };
            // The rotated mask also contains walls at other angles; keep
            // only those really at this angle and not already found.
            let dir = (w.b.y - w.a.y).atan2(w.b.x - w.a.x);
            let off =
                angle_diff(dir, angle).min(angle_diff(dir, angle + std::f32::consts::FRAC_PI_2));
            // A wall is longer than it is thick (corners seen at a slant
            // are not).
            if off < 4f32.to_radians()
                && w.length() >= w.thickness_px * 2.0
                && !walls.iter().any(|o| same_wall(o, &w))
            {
                walls.push(w);
            }
        }
    }
    walls.retain(|w| w.length() >= p.min_length_px);

    // Curved walls: what is left once straight walls are removed.
    let curves = detect_curves(mask, p, &walls);
    walls.extend(curves);
    snap_free_endpoints(&mut walls);

    for w in &mut walls {
        // Keep a, b ordered left→right / top→bottom.
        if (w.horizontal() && w.a.x > w.b.x) || (!w.horizontal() && w.a.y > w.b.y) {
            std::mem::swap(&mut w.a, &mut w.b);
        }
    }
    walls
}

/// Smallest difference between two line directions (radians, modulo π).
fn angle_diff(a: f32, b: f32) -> f32 {
    let pi = std::f32::consts::PI;
    let d = (a - b).rem_euclid(pi);
    d.min(pi - d)
}

/// Two walls describe the same thing: parallel, overlapping and close.
fn same_wall(a: &DetectedWall, b: &DetectedWall) -> bool {
    let da = (a.b.y - a.a.y).atan2(a.b.x - a.a.x);
    let db = (b.b.y - b.a.y).atan2(b.b.x - b.a.x);
    if angle_diff(da, db) > 6f32.to_radians() {
        return false;
    }
    let tol = a.thickness_px.max(b.thickness_px);
    let mid = Vec2::new((b.a.x + b.b.x) * 0.5, (b.a.y + b.b.y) * 0.5);
    point_segment_distance(mid, a.a, a.b) <= tol
}

fn point_segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(closest_on_segment(p, a, b))
}

fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-9 {
        return a;
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    Vec2::new(a.x + dx * t, a.y + dy * t)
}

/// A mask rotated by `-angle` (so lines at `angle` become horizontal),
/// with the mapping back to the source.
struct Rotated {
    mask: Mask,
    angle: f32,
    src_center: Vec2,
    dst_center: Vec2,
}

impl Rotated {
    fn new(src: &Mask, angle: f32) -> Self {
        let (c, s) = (angle.cos().abs(), angle.sin().abs());
        let (w, h) = (src.width as f32, src.height as f32);
        let nw = ((w * c + h * s).ceil() as usize).max(1);
        let nh = ((w * s + h * c).ceil() as usize).max(1);
        let mut r = Self {
            mask: Mask::new(nw, nh),
            angle,
            src_center: Vec2::new(w * 0.5, h * 0.5),
            dst_center: Vec2::new(nw as f32 * 0.5, nh as f32 * 0.5),
        };
        for y in 0..nh {
            for x in 0..nw {
                let p = r.to_source(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                if p.x >= 0.0 && p.y >= 0.0 {
                    let (sx, sy) = (p.x as usize, p.y as usize);
                    if sx < src.width && sy < src.height && src.get(sx, sy) {
                        r.mask.set(x, y, true);
                    }
                }
            }
        }
        r
    }

    /// Rotated coordinates → source coordinates.
    fn to_source(&self, p: Vec2) -> Vec2 {
        let (c, s) = (self.angle.cos(), self.angle.sin());
        let (dx, dy) = (p.x - self.dst_center.x, p.y - self.dst_center.y);
        Vec2::new(
            self.src_center.x + c * dx - s * dy,
            self.src_center.y + s * dx + c * dy,
        )
    }
}

/// Angles (radians, in (0, π/2)) other than horizontal/vertical along
/// which the plan has a lot of long straight ink. Found on a reduced copy:
/// a coarse sweep in 2° steps, refined in 0.5° steps. A pixel scores its
/// row run length when the run is long and the band around it has wall
/// thickness, so a line exactly at the angle scores far more than the same
/// line a few degrees off.
fn dominant_angles(mask: &Mask, p: &DetectParams) -> Vec<f32> {
    let side = mask.width.max(mask.height);
    let factor = side.div_ceil(700).max(1);
    let small = mask.downscale(factor);
    let k = factor as f32;
    let min_len = (p.min_length_px / k).max(6.0) as u16;
    let min_t = (p.min_thickness_px / k).max(1.0) as u16;
    let max_t = (p.max_thickness_px / k).ceil() as u16 + 1;
    let score = |deg: f32| -> u64 {
        let r = Rotated::new(&small, deg.to_radians());
        let rows = row_runs(&r.mask);
        let cols = col_runs(&r.mask);
        rows.iter()
            .zip(&cols)
            // Long along the row and much longer than thick: walls at
            // other angles seen at a slant have runs as thick as long.
            .filter(|(h, v)| **h >= min_len && **h >= **v * 4 && **v >= min_t && **v <= max_t)
            .map(|(h, _)| u64::from(*h))
            .sum()
    };
    let axis = score(0.0);
    // Lines at θ and θ + 90° look the same once rotated, so 0..90 covers
    // all. The ends of the sweep only catch axis walls seen at a slant.
    let sweep: Vec<(f32, u64)> = (5..=42)
        .map(|i| i as f32 * 2.0)
        .map(|d| (d, score(d)))
        .collect();
    let floor = (axis / 40).max(u64::from(min_len).pow(2) * u64::from(min_t) * 2);
    let mut found: Vec<(f32, u64)> = Vec::new();
    for i in 1..sweep.len().saturating_sub(1) {
        let (d, s) = sweep[i];
        if s >= floor && s > sweep[i - 1].1 && s >= sweep[i + 1].1 {
            let best = [-1.5f32, -1.0, -0.5, 0.0, 0.5, 1.0, 1.5]
                .iter()
                .map(|o| (d + o, score(d + o)))
                .max_by_key(|x| x.1)
                .unwrap_or((d, s));
            found.push(best);
        }
    }
    found.sort_by_key(|x| std::cmp::Reverse(x.1));
    found.truncate(4);
    found.into_iter().map(|(d, _)| d.to_radians()).collect()
}

/// Distance (in pixels, chamfer 3-4) from each ink pixel to the nearest
/// background pixel; 0 on background, `f32::MAX` if there is none.
fn distance_transform(m: &Mask) -> Vec<f32> {
    let (w, h) = (m.width, m.height);
    let big = u32::MAX / 4;
    let mut d: Vec<u32> = m.data.iter().map(|&b| if b { big } else { 0 }).collect();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if d[i] == 0 {
                continue;
            }
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 3);
            }
            if y > 0 {
                v = v.min(d[i - w] + 3);
                if x > 0 {
                    v = v.min(d[i - w - 1] + 4);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + 4);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if d[i] == 0 {
                continue;
            }
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + 3);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + 3);
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + 4);
                }
                if x > 0 {
                    v = v.min(d[i + w - 1] + 4);
                }
            }
            d[i] = v;
        }
    }
    d.into_iter()
        .map(|v| if v >= big { f32::MAX } else { v as f32 / 3.0 })
        .collect()
}

/// Zhang–Suen thinning: reduces bands to one-pixel center lines.
fn thin(m: &mut Mask) {
    let (w, h) = (m.width, m.height);
    if w < 3 || h < 3 {
        return;
    }
    let mut remove = Vec::new();
    loop {
        let mut changed = false;
        for step in 0..2 {
            remove.clear();
            for y in 1..h - 1 {
                for x in 1..w - 1 {
                    if !m.get(x, y) {
                        continue;
                    }
                    // Clockwise from north.
                    let n = [
                        m.get(x, y - 1),
                        m.get(x + 1, y - 1),
                        m.get(x + 1, y),
                        m.get(x + 1, y + 1),
                        m.get(x, y + 1),
                        m.get(x - 1, y + 1),
                        m.get(x - 1, y),
                        m.get(x - 1, y - 1),
                    ];
                    let b = n.iter().filter(|&&v| v).count();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let a = (0..8).filter(|&k| !n[k] && n[(k + 1) % 8]).count();
                    if a != 1 {
                        continue;
                    }
                    let (p2, p4, p6, p8) = (n[0], n[2], n[4], n[6]);
                    let ok = if step == 0 {
                        !(p2 && p4 && p6) && !(p4 && p6 && p8)
                    } else {
                        !(p2 && p4 && p8) && !(p2 && p6 && p8)
                    };
                    if ok {
                        remove.push((x, y));
                    }
                }
            }
            if !remove.is_empty() {
                changed = true;
                for &(x, y) in &remove {
                    m.set(x, y, false);
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Ramer–Douglas–Peucker simplification of a polyline.
fn simplify(points: &[Vec2], eps: f32) -> Vec<Vec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (mut worst, mut at) = (0.0, 0);
    for (i, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let d = point_segment_distance(*p, a, b);
        if d > worst {
            worst = d;
            at = i;
        }
    }
    if worst <= eps {
        return vec![a, b];
    }
    let mut left = simplify(&points[..=at], eps);
    let right = simplify(&points[at..], eps);
    left.pop();
    left.extend(right);
    left
}

/// 8-connected ink neighbors of `(x, y)`, straight steps first.
fn neighbors(m: &Mask, x: usize, y: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    const OFFSETS: [(i32, i32); 8] = [
        (0, -1),
        (1, 0),
        (0, 1),
        (-1, 0),
        (1, -1),
        (1, 1),
        (-1, 1),
        (-1, -1),
    ];
    OFFSETS.iter().filter_map(move |&(dx, dy)| {
        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
        (nx >= 0
            && ny >= 0
            && (nx as usize) < m.width
            && (ny as usize) < m.height
            && m.get(nx as usize, ny as usize))
        .then_some((nx as usize, ny as usize))
    })
}

/// Curved (and any other leftover) walls: bands of wall thickness not
/// explained by the straight walls already found, reduced to their center
/// line and turned into simplified polylines.
fn detect_curves(mask: &Mask, p: &DetectParams, straight: &[DetectedWall]) -> Vec<DetectedWall> {
    let (w, h) = (mask.width, mask.height);
    if w < 3 || h < 3 {
        return Vec::new();
    }
    let dt = distance_transform(mask);
    // Ink of wall thickness, away from solid fills (whose edges would
    // otherwise look like thin bands)…
    let mut not_core = Mask::new(w, h);
    for ((nc, &ink), &d) in not_core.data.iter_mut().zip(&mask.data).zip(&dt) {
        *nc = !(ink && d * 2.0 > p.max_thickness_px * 1.2 + 1.0);
    }
    let to_core = distance_transform(&not_core);
    let mut residual = Mask::new(w, h);
    for ((r, &ink), &d) in residual.data.iter_mut().zip(&mask.data).zip(&to_core) {
        *r = ink && d > p.max_thickness_px + 1.0;
    }
    // …minus the straight walls already detected (with a margin).
    for s in straight {
        let r = s.thickness_px * 0.5 + 2.0;
        let x0 = (s.a.x.min(s.b.x) - r).max(0.0) as usize;
        let y0 = (s.a.y.min(s.b.y) - r).max(0.0) as usize;
        let x1 = ((s.a.x.max(s.b.x) + r).max(0.0) as usize).min(w - 1);
        let y1 = ((s.a.y.max(s.b.y) + r).max(0.0) as usize).min(h - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if point_segment_distance(c, s.a, s.b) <= r {
                    residual.set(x, y, false);
                }
            }
        }
    }
    if residual.ink_count() == 0 {
        return Vec::new();
    }
    thin(&mut residual);
    // Keep center-line pixels whose local thickness is a wall's (this drops
    // text, hatching and thin drawing lines).
    for (r, &d) in residual.data.iter_mut().zip(&dt) {
        let t = d * 2.0;
        *r &= t >= p.min_thickness_px * 0.9 && t <= p.max_thickness_px * 1.2;
    }

    // Trace chains of skeleton pixels, starting from end points so open
    // curves come out whole; whatever is left afterwards is a loop.
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut rest: Vec<(usize, usize)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if residual.get(x, y) {
                if neighbors(&residual, x, y).count() == 1 {
                    starts.push((x, y));
                } else {
                    rest.push((x, y));
                }
            }
        }
    }
    starts.extend(rest);
    let mut visited = vec![false; w * h];
    let mut out = Vec::new();
    for (sx, sy) in starts {
        if visited[sy * w + sx] {
            continue;
        }
        visited[sy * w + sx] = true;
        let mut chain = vec![(sx, sy)];
        let (mut x, mut y) = (sx, sy);
        while let Some((nx, ny)) =
            neighbors(&residual, x, y).find(|&(nx, ny)| !visited[ny * w + nx])
        {
            visited[ny * w + nx] = true;
            chain.push((nx, ny));
            (x, y) = (nx, ny);
        }
        let pts: Vec<Vec2> = chain
            .iter()
            .map(|&(x, y)| Vec2::new(x as f32 + 0.5, y as f32 + 0.5))
            .collect();
        let length: f32 = pts.windows(2).map(|s| s[0].distance(s[1])).sum();
        // Curved walls are long and wide; letters and symbols are not.
        let (mut lo, mut hi) = (pts[0], pts[0]);
        for q in &pts {
            lo = Vec2::new(lo.x.min(q.x), lo.y.min(q.y));
            hi = Vec2::new(hi.x.max(q.x), hi.y.max(q.y));
        }
        let min_curve = p.min_length_px * 3.0;
        if length < min_curve || lo.distance(hi) < min_curve {
            continue;
        }
        let mut thick: Vec<f32> = chain.iter().map(|&(x, y)| dt[y * w + x] * 2.0).collect();
        thick.sort_by(f32::total_cmp);
        let t = thick[thick.len() / 2].max(p.min_thickness_px);
        for s in simplify(&pts, (t * 0.5).max(1.5)).windows(2) {
            if s[0].distance(s[1]) >= 1.0 {
                out.push(DetectedWall {
                    a: s[0],
                    b: s[1],
                    thickness_px: t,
                });
            }
        }
    }
    out
}

/// Moves loose ends of diagonal and curved walls onto the nearest other
/// wall when it is within reach, so they join their neighbors. Ends that
/// already touch another wall stay put.
fn snap_free_endpoints(walls: &mut [DetectedWall]) {
    let snapshot = walls.to_vec();
    for (i, w) in walls.iter_mut().enumerate() {
        let axis = (w.a.x - w.b.x).abs() < 0.5 || (w.a.y - w.b.y).abs() < 0.5;
        if axis {
            continue;
        }
        for end in 0..2 {
            let p = if end == 0 { w.a } else { w.b };
            let mut best: Option<(f32, Vec2)> = None;
            for (j, o) in snapshot.iter().enumerate() {
                if j == i {
                    continue;
                }
                let q = closest_on_segment(p, o.a, o.b);
                let d = p.distance(q);
                let reach = o.thickness_px * 0.5 + w.thickness_px + 2.0;
                if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, q));
                }
            }
            match best {
                Some((d, q)) if d > 0.01 => {
                    if end == 0 {
                        w.a = q;
                    } else {
                        w.b = q;
                    }
                }
                _ => {}
            }
        }
    }
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

    /// Ink within `t/2` of the segment a–b.
    fn band(m: &mut Mask, a: Vec2, b: Vec2, t: f32) {
        for y in 0..m.height {
            for x in 0..m.width {
                let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if point_segment_distance(c, a, b) <= t * 0.5 {
                    m.set(x, y, true);
                }
            }
        }
    }

    fn angle_deg(w: &DetectedWall) -> f32 {
        (w.b.y - w.a.y)
            .atan2(w.b.x - w.a.x)
            .to_degrees()
            .rem_euclid(180.0)
    }

    #[test]
    fn finds_diagonal_walls() {
        let mut m = Mask::new(400, 400);
        band(&mut m, Vec2::new(80.0, 80.0), Vec2::new(292.0, 292.0), 8.0);
        let walls = detect(&m, &params());
        assert_eq!(walls.len(), 1, "{walls:?}");
        let w = &walls[0];
        assert!(near(angle_deg(w), 45.0, 1.0), "{w:?}");
        assert!(near(w.length(), 300.0, 12.0), "{w:?}");
        assert!(near(w.thickness_px, 8.0, 1.5), "{w:?}");
        // On the center line.
        let mid = Vec2::new((w.a.x + w.b.x) * 0.5, (w.a.y + w.b.y) * 0.5);
        assert!(near(mid.x, mid.y, 2.0), "{w:?}");
    }

    #[test]
    fn finds_both_diagonal_directions_and_joins_axis_walls() {
        let mut m = Mask::new(500, 400);
        // Top wall (center y = 50) with diagonals hanging from it at 30°
        // and 120° (= -60°).
        rect(&mut m, 40, 46, 460, 54);
        let (c30, s30) = (30f32.to_radians().cos(), 30f32.to_radians().sin());
        band(
            &mut m,
            Vec2::new(60.0, 50.0),
            Vec2::new(60.0 + 300.0 * c30, 50.0 + 300.0 * s30),
            8.0,
        );
        let (c60, s60) = (60f32.to_radians().cos(), 60f32.to_radians().sin());
        band(
            &mut m,
            Vec2::new(440.0, 50.0),
            Vec2::new(440.0 - 280.0 * c60, 50.0 + 280.0 * s60),
            8.0,
        );
        let walls = detect(&m, &params());
        assert_eq!(walls.len(), 3, "{walls:?}");
        let mut angles: Vec<f32> = walls.iter().map(angle_deg).collect();
        angles.sort_by(f32::total_cmp);
        assert!(
            near(angles[0], 0.0, 0.5) || near(angles[2], 180.0, 0.5),
            "{angles:?}"
        );
        assert!(
            walls.iter().any(|w| near(angle_deg(w), 30.0, 1.0)),
            "{angles:?}"
        );
        assert!(
            walls.iter().any(|w| near(angle_deg(w), 120.0, 1.0)),
            "{angles:?}"
        );
        // Diagonal ends touching the top wall sit on its center line.
        for w in walls
            .iter()
            .filter(|w| !near(angle_deg(w) % 180.0, 0.0, 1.0))
        {
            let top = w.a.y.min(w.b.y);
            assert!(near(top, 50.0, 2.5), "{w:?}");
        }
    }

    #[test]
    fn approximates_curved_walls() {
        let mut m = Mask::new(400, 400);
        // Quarter ring: center (50, 50), radius 250, 8 px thick.
        let (cx, cy, r) = (50.0f32, 50.0f32, 250.0f32);
        for y in 0..400 {
            for x in 0..400 {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(Vec2::new(cx, cy));
                if (d - r).abs() <= 4.0 && x as f32 >= cx && y as f32 >= cy {
                    m.set(x, y, true);
                }
            }
        }
        let p = DetectParams {
            min_length_px: 60.0,
            ..params()
        };
        let walls = detect(&m, &p);
        assert!(walls.len() >= 3, "{walls:?}");
        let total: f32 = walls.iter().map(DetectedWall::length).sum();
        let arc = std::f32::consts::FRAC_PI_2 * r;
        assert!(near(total, arc, arc * 0.15), "{total} vs {arc}: {walls:?}");
        for w in &walls {
            let mid = Vec2::new((w.a.x + w.b.x) * 0.5, (w.a.y + w.b.y) * 0.5);
            assert!(near(mid.distance(Vec2::new(cx, cy)), r, 8.0), "{w:?}");
            assert!(near(w.thickness_px, 8.0, 2.0), "{w:?}");
        }
    }

    #[test]
    fn solid_fills_do_not_become_curves() {
        let mut m = Mask::new(300, 300);
        rect(&mut m, 50, 50, 250, 250); // a solid column / filled area
        for y in 0..300 {
            for x in 0..300 {
                let d = Vec2::new(x as f32, y as f32).distance(Vec2::new(150.0, 150.0));
                if d > 120.0 && d < 124.0 {
                    m.set(x, y, true); // thin (4 px) circle: a drawing line
                }
            }
        }
        let p = DetectParams {
            min_thickness_px: 6.0,
            ..params()
        };
        let walls = detect(&m, &p);
        // Neither the filled square nor the thin circle is a wall.
        assert!(walls.is_empty(), "{walls:?}");
    }
}
