//! 2D geometry in world coordinates (meters) and a uniform-grid spatial
//! index used to find the walls crossed by a ray quickly.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn distance(self, other: Vec2) -> f32 {
        (self - other).length()
    }

    /// 2D cross product (z component of the 3D cross product).
    pub fn cross(self, other: Vec2) -> f32 {
        self.x * other.y - self.y * other.x
    }

    pub fn min(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x.min(other.x), self.y.min(other.y))
    }

    pub fn max(self, other: Vec2) -> Vec2 {
        Vec2::new(self.x.max(other.x), self.y.max(other.y))
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, rhs: Vec2) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, rhs: f32) -> Vec2 {
        Vec2::new(self.x * rhs, self.y * rhs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub a: Vec2,
    pub b: Vec2,
}

impl Segment {
    pub const fn new(a: Vec2, b: Vec2) -> Self {
        Self { a, b }
    }

    pub fn length(&self) -> f32 {
        self.a.distance(self.b)
    }

    /// True if the two closed segments share at least one point.
    /// Collinear overlapping segments count as intersecting.
    pub fn intersects(&self, other: &Segment) -> bool {
        let (p, r) = (self.a, self.b - self.a);
        let (q, s) = (other.a, other.b - other.a);
        let denom = r.cross(s);
        let qp = q - p;
        const EPS: f32 = 1e-6;
        if denom.abs() < EPS * r.length().max(1.0) * s.length().max(1.0) {
            // Parallel. Intersect only if collinear and overlapping.
            if qp.cross(r).abs() > EPS * r.length().max(1.0) {
                return false;
            }
            let rr = r.x * r.x + r.y * r.y;
            if rr == 0.0 {
                return other.distance_to_point(p) < EPS;
            }
            let t0 = (qp.x * r.x + qp.y * r.y) / rr;
            let t1 = t0 + (s.x * r.x + s.y * r.y) / rr;
            let (lo, hi) = (t0.min(t1), t0.max(t1));
            return hi >= 0.0 && lo <= 1.0;
        }
        let t = qp.cross(s) / denom;
        let u = qp.cross(r) / denom;
        (-EPS..=1.0 + EPS).contains(&t) && (-EPS..=1.0 + EPS).contains(&u)
    }

    pub fn distance_to_point(&self, p: Vec2) -> f32 {
        let d = self.b - self.a;
        let len2 = d.x * d.x + d.y * d.y;
        if len2 == 0.0 {
            return p.distance(self.a);
        }
        let t = (((p.x - self.a.x) * d.x + (p.y - self.a.y) * d.y) / len2).clamp(0.0, 1.0);
        p.distance(self.a + d * t)
    }

    /// True if the segment touches the axis-aligned box `[min, max]`.
    fn touches_box(&self, min: Vec2, max: Vec2) -> bool {
        clip_to_box(self.a, self.b, min, max).is_some()
    }
}

/// Liang–Barsky clipping of segment `a→b` against a box.
/// Returns the parametric range `[t0, t1] ⊆ [0, 1]` inside the box.
fn clip_to_box(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> Option<(f32, f32)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-d.x, a.x - min.x),
        (d.x, max.x - a.x),
        (-d.y, a.y - min.y),
        (d.y, max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    Some((t0, t1))
}

/// Uniform grid over a set of segments. Each cell lists the segments that
/// touch it (conservatively, with a small margin), so a ray only has to test
/// the segments in the cells it passes through.
#[derive(Debug, Clone)]
pub struct SegmentIndex {
    segments: Vec<Segment>,
    origin: Vec2,
    cell: f32,
    cols: usize,
    rows: usize,
    cells: Vec<Vec<u32>>,
}

/// Per-thread scratch buffer to deduplicate segments spanning several cells.
#[derive(Debug, Default, Clone)]
pub struct QueryScratch {
    stamps: Vec<u32>,
    generation: u32,
}

impl SegmentIndex {
    /// Builds the index. `cell_size` is in meters; 1–2 m works well indoors.
    pub fn build(segments: Vec<Segment>, cell_size: f32) -> Self {
        assert!(cell_size > 0.0, "cell_size must be positive");
        if segments.is_empty() {
            return Self {
                segments,
                origin: Vec2::default(),
                cell: cell_size,
                cols: 0,
                rows: 0,
                cells: Vec::new(),
            };
        }
        let mut lo = Vec2::new(f32::MAX, f32::MAX);
        let mut hi = Vec2::new(f32::MIN, f32::MIN);
        for s in &segments {
            lo = lo.min(s.a).min(s.b);
            hi = hi.max(s.a).max(s.b);
        }
        let margin = cell_size * 0.5;
        let origin = lo - Vec2::new(margin, margin);
        let cols = (((hi.x - origin.x) / cell_size).floor() as usize + 2).max(1);
        let rows = (((hi.y - origin.y) / cell_size).floor() as usize + 2).max(1);
        let mut cells = vec![Vec::new(); cols * rows];
        let eps = cell_size * 1e-3;
        for (id, s) in segments.iter().enumerate() {
            let smin = s.a.min(s.b);
            let smax = s.a.max(s.b);
            let c0 = ((smin.x - origin.x - eps) / cell_size).floor().max(0.0) as usize;
            let r0 = ((smin.y - origin.y - eps) / cell_size).floor().max(0.0) as usize;
            let c1 = (((smax.x - origin.x + eps) / cell_size).floor() as usize).min(cols - 1);
            let r1 = (((smax.y - origin.y + eps) / cell_size).floor() as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let bmin = Vec2::new(
                        origin.x + c as f32 * cell_size - eps,
                        origin.y + r as f32 * cell_size - eps,
                    );
                    let bmax = Vec2::new(
                        bmin.x + cell_size + 2.0 * eps,
                        bmin.y + cell_size + 2.0 * eps,
                    );
                    if s.touches_box(bmin, bmax) {
                        cells[r * cols + c].push(id as u32);
                    }
                }
            }
        }
        Self {
            segments,
            origin,
            cell: cell_size,
            cols,
            rows,
            cells,
        }
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn new_scratch(&self) -> QueryScratch {
        QueryScratch {
            stamps: vec![0; self.segments.len()],
            generation: 0,
        }
    }

    /// Calls `f(segment_id)` once for every segment intersecting `a→b`.
    pub fn for_each_crossing(
        &self,
        a: Vec2,
        b: Vec2,
        scratch: &mut QueryScratch,
        mut f: impl FnMut(u32),
    ) {
        if self.cells.is_empty() {
            return;
        }
        if scratch.stamps.len() != self.segments.len() {
            *scratch = self.new_scratch();
        }
        scratch.generation = scratch.generation.wrapping_add(1);
        if scratch.generation == 0 {
            scratch.stamps.fill(0);
            scratch.generation = 1;
        }
        let gen = scratch.generation;
        let ray = Segment::new(a, b);
        self.traverse(a, b, |cell| {
            for &id in &self.cells[cell] {
                let stamp = &mut scratch.stamps[id as usize];
                if *stamp == gen {
                    continue;
                }
                *stamp = gen;
                if ray.intersects(&self.segments[id as usize]) {
                    f(id);
                }
            }
        });
    }

    /// Amanatides–Woo grid traversal: visits every cell the segment passes through.
    fn traverse(&self, a: Vec2, b: Vec2, mut visit: impl FnMut(usize)) {
        let max = Vec2::new(
            self.origin.x + self.cols as f32 * self.cell,
            self.origin.y + self.rows as f32 * self.cell,
        );
        let Some((t0, t1)) = clip_to_box(a, b, self.origin, max) else {
            return;
        };
        let d = b - a;
        let start = a + d * t0;
        let to_cell = |v: f32, o: f32, n: usize| -> i64 {
            (((v - o) / self.cell).floor() as i64).clamp(0, n as i64 - 1)
        };
        let mut cx = to_cell(start.x, self.origin.x, self.cols);
        let mut cy = to_cell(start.y, self.origin.y, self.rows);
        let axis = |dv: f32, av: f32, o: f32, c: i64| -> (i64, f32, f32) {
            if dv > 0.0 {
                let edge = o + (c + 1) as f32 * self.cell;
                (1, (edge - av) / dv, self.cell / dv)
            } else if dv < 0.0 {
                let edge = o + c as f32 * self.cell;
                (-1, (edge - av) / dv, -self.cell / dv)
            } else {
                (0, f32::INFINITY, f32::INFINITY)
            }
        };
        let (sx, mut tmx, tdx) = axis(d.x, a.x, self.origin.x, cx);
        let (sy, mut tmy, tdy) = axis(d.y, a.y, self.origin.y, cy);
        loop {
            visit(cy as usize * self.cols + cx as usize);
            if tmx < tmy {
                if tmx > t1 {
                    break;
                }
                cx += sx;
                tmx += tdx;
            } else {
                if tmy > t1 {
                    break;
                }
                cy += sy;
                tmy += tdy;
            }
            if cx < 0 || cy < 0 || cx >= self.cols as i64 || cy >= self.rows as i64 {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f32, ay: f32, bx: f32, by: f32) -> Segment {
        Segment::new(Vec2::new(ax, ay), Vec2::new(bx, by))
    }

    #[test]
    fn crossing_segments_intersect() {
        assert!(seg(0.0, 0.0, 2.0, 2.0).intersects(&seg(0.0, 2.0, 2.0, 0.0)));
    }

    #[test]
    fn disjoint_segments_do_not_intersect() {
        assert!(!seg(0.0, 0.0, 1.0, 0.0).intersects(&seg(0.0, 1.0, 1.0, 1.0)));
        assert!(!seg(0.0, 0.0, 1.0, 1.0).intersects(&seg(2.0, 0.0, 3.0, -1.0)));
    }

    #[test]
    fn touching_endpoint_counts() {
        assert!(seg(0.0, 0.0, 1.0, 0.0).intersects(&seg(1.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn collinear_overlap_and_gap() {
        assert!(seg(0.0, 0.0, 2.0, 0.0).intersects(&seg(1.0, 0.0, 3.0, 0.0)));
        assert!(!seg(0.0, 0.0, 1.0, 0.0).intersects(&seg(2.0, 0.0, 3.0, 0.0)));
    }

    #[test]
    fn point_distance() {
        let s = seg(0.0, 0.0, 10.0, 0.0);
        assert!((s.distance_to_point(Vec2::new(5.0, 3.0)) - 3.0).abs() < 1e-6);
        assert!((s.distance_to_point(Vec2::new(-4.0, 3.0)) - 5.0).abs() < 1e-6);
    }

    /// Small deterministic PRNG so tests need no extra dependency.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
        }
        fn point(&mut self, size: f32) -> Vec2 {
            Vec2::new(self.next() * size, self.next() * size)
        }
    }

    #[test]
    fn index_matches_brute_force() {
        let mut rng = Lcg(42);
        let walls: Vec<Segment> = (0..300)
            .map(|_| {
                let a = rng.point(50.0);
                let b = a + (rng.point(10.0) - Vec2::new(5.0, 5.0));
                Segment::new(a, b)
            })
            .collect();
        // Include axis-aligned walls on cell boundaries, the classic DDA edge case.
        let mut walls = walls;
        walls.push(seg(10.0, 0.0, 10.0, 50.0));
        walls.push(seg(0.0, 20.0, 50.0, 20.0));
        let index = SegmentIndex::build(walls.clone(), 1.5);
        let mut scratch = index.new_scratch();
        for _ in 0..2000 {
            let a = rng.point(60.0) - Vec2::new(5.0, 5.0);
            let b = rng.point(60.0) - Vec2::new(5.0, 5.0);
            let ray = Segment::new(a, b);
            let mut expected: Vec<u32> = walls
                .iter()
                .enumerate()
                .filter(|(_, w)| ray.intersects(w))
                .map(|(i, _)| i as u32)
                .collect();
            let mut got = Vec::new();
            index.for_each_crossing(a, b, &mut scratch, |id| got.push(id));
            expected.sort_unstable();
            got.sort_unstable();
            assert_eq!(got, expected, "ray {a:?} -> {b:?}");
        }
    }

    #[test]
    fn empty_index_finds_nothing() {
        let index = SegmentIndex::build(Vec::new(), 1.0);
        let mut scratch = index.new_scratch();
        let mut n = 0;
        index.for_each_crossing(
            Vec2::new(0.0, 0.0),
            Vec2::new(5.0, 5.0),
            &mut scratch,
            |_| n += 1,
        );
        assert_eq!(n, 0);
    }
}
