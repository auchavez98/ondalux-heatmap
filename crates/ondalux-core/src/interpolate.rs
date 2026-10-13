//! Spatial interpolation of survey measurements onto the heat map grid.
//!
//! Interpolation weights depend only on where the measurements were taken
//! (and on the variogram, for kriging), not on the values. They are computed
//! once per grid cell and then applied to every BSSID, which keeps kriging
//! affordable even with dozens of access points.
//!
//! See `docs/rf-models.md` § "Measured heat maps".

use crate::geometry::Vec2;
use crate::grid::GridSpec;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterpolationMethod {
    /// Inverse distance weighting.
    Idw,
    /// Ordinary kriging with an exponential variogram fitted to the data.
    Kriging,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InterpolationSettings {
    pub method: InterpolationMethod,
    /// IDW distance exponent (2 is the usual choice).
    pub idw_power: f32,
    /// Number of nearest measurements used for each cell.
    pub neighbors: usize,
    /// Cells farther than this from every measurement are left without data.
    pub max_distance_m: f32,
}

impl Default for InterpolationSettings {
    fn default() -> Self {
        Self {
            method: InterpolationMethod::Idw,
            idw_power: 2.0,
            neighbors: 12,
            max_distance_m: 6.0,
        }
    }
}

/// Exponential variogram: `γ(h) = nugget + psill · (1 − exp(−3h / range))`.
/// `range` is the practical range (95 % of the sill).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Variogram {
    pub nugget: f64,
    pub psill: f64,
    pub range: f64,
}

impl Variogram {
    pub fn gamma(&self, h: f64) -> f64 {
        if h <= 0.0 {
            0.0
        } else {
            self.nugget + self.psill * (1.0 - (-3.0 * h / self.range).exp())
        }
    }

    /// Used when there is too little data to fit one.
    pub const FALLBACK: Variogram = Variogram {
        nugget: 0.0,
        psill: 1.0,
        range: 10.0,
    };
}

/// Empirical semivariogram bin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariogramBin {
    pub lag: f64,
    pub semivariance: f64,
    pub pairs: usize,
}

/// Pooled empirical semivariogram over several value sets measured at the
/// same `positions` (`values[set][point]`, `NaN` = missing).
pub fn empirical_variogram(
    positions: &[Vec2],
    values: &[Vec<f32>],
    bins: usize,
) -> Vec<VariogramBin> {
    let n = positions.len();
    if n < 2 || bins == 0 {
        return Vec::new();
    }
    let mut max_d = 0.0f64;
    for i in 0..n {
        for j in i + 1..n {
            max_d = max_d.max(positions[i].distance(positions[j]) as f64);
        }
    }
    let max_lag = max_d * 0.5;
    if max_lag <= 0.0 {
        return Vec::new();
    }
    let width = max_lag / bins as f64;
    let mut sum = vec![0.0f64; bins];
    let mut count = vec![0usize; bins];
    let mut lag_sum = vec![0.0f64; bins];
    for i in 0..n {
        for j in i + 1..n {
            let d = positions[i].distance(positions[j]) as f64;
            let b = (d / width) as usize;
            if b >= bins {
                continue;
            }
            for set in values {
                let (a, c) = (set[i], set[j]);
                if a.is_nan() || c.is_nan() {
                    continue;
                }
                let diff = (a - c) as f64;
                sum[b] += 0.5 * diff * diff;
                count[b] += 1;
                lag_sum[b] += d;
            }
        }
    }
    (0..bins)
        .filter(|&b| count[b] > 0)
        .map(|b| VariogramBin {
            lag: lag_sum[b] / count[b] as f64,
            semivariance: sum[b] / count[b] as f64,
            pairs: count[b],
        })
        .collect()
}

/// Fits an exponential variogram to empirical bins by weighted least squares
/// (weights = number of pairs). The range is searched on a grid; nugget and
/// partial sill are solved linearly for each candidate range.
pub fn fit_variogram(bins: &[VariogramBin]) -> Variogram {
    if bins.len() < 3 {
        return Variogram::FALLBACK;
    }
    let max_lag = bins.iter().map(|b| b.lag).fold(0.0, f64::max);
    let mut best: Option<(f64, Variogram)> = None;
    for step in 1..=60 {
        let range = max_lag * 2.0 * step as f64 / 60.0;
        // Basis: γ = nugget · 1 + psill · f(h).
        let (mut s11, mut s12, mut s22, mut b1, mut b2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for bin in bins {
            let w = bin.pairs as f64;
            let f = 1.0 - (-3.0 * bin.lag / range).exp();
            s11 += w;
            s12 += w * f;
            s22 += w * f * f;
            b1 += w * bin.semivariance;
            b2 += w * f * bin.semivariance;
        }
        let det = s11 * s22 - s12 * s12;
        let (mut nugget, mut psill) = if det.abs() > 1e-12 {
            ((b1 * s22 - b2 * s12) / det, (s11 * b2 - s12 * b1) / det)
        } else {
            (0.0, b2 / s22.max(1e-12))
        };
        if nugget < 0.0 {
            nugget = 0.0;
            psill = b2 / s22.max(1e-12);
        }
        if psill <= 0.0 {
            continue;
        }
        if nugget < 0.0 {
            nugget = 0.0;
        }
        let v = Variogram {
            nugget,
            psill,
            range,
        };
        let sse: f64 = bins
            .iter()
            .map(|b| b.pairs as f64 * (v.gamma(b.lag) - b.semivariance).powi(2))
            .sum();
        if best.is_none_or(|(e, _)| sse < e) {
            best = Some((sse, v));
        }
    }
    best.map_or(Variogram::FALLBACK, |(_, v)| v)
}

/// Interpolation weights for every grid cell.
#[derive(Debug, Clone)]
pub struct WeightGrid {
    pub grid: GridSpec,
    /// `offsets[c]..offsets[c + 1]` indexes `samples` / `weights` for cell `c`.
    /// An empty range means "no data" (too far from every measurement).
    offsets: Vec<u32>,
    samples: Vec<u32>,
    weights: Vec<f32>,
}

impl WeightGrid {
    pub fn build(
        grid: GridSpec,
        positions: &[Vec2],
        settings: &InterpolationSettings,
        variogram: Option<&Variogram>,
    ) -> Self {
        let k = settings.neighbors.clamp(1, 64).min(positions.len());
        let per_row: Vec<(Vec<u32>, Vec<u32>, Vec<f32>)> = (0..grid.rows)
            .into_par_iter()
            .map(|row| {
                let mut counts = Vec::with_capacity(grid.cols);
                let mut idx = Vec::new();
                let mut wts = Vec::new();
                let mut near: Vec<(f32, u32)> = Vec::with_capacity(k + 1);
                for col in 0..grid.cols {
                    let p = grid.sample(col, row);
                    nearest(positions, p, k, &mut near);
                    let within = near
                        .first()
                        .is_some_and(|(d, _)| *d <= settings.max_distance_m);
                    if !within {
                        counts.push(0);
                        continue;
                    }
                    let before = idx.len();
                    match (settings.method, variogram) {
                        (InterpolationMethod::Kriging, Some(v)) if near.len() >= 3 => {
                            if !kriging_weights(positions, &near, v, &mut idx, &mut wts) {
                                idw_weights(&near, settings.idw_power, &mut idx, &mut wts);
                            }
                        }
                        _ => idw_weights(&near, settings.idw_power, &mut idx, &mut wts),
                    }
                    counts.push((idx.len() - before) as u32);
                }
                (counts, idx, wts)
            })
            .collect();

        let mut offsets = Vec::with_capacity(grid.len() + 1);
        let mut samples = Vec::new();
        let mut weights = Vec::new();
        offsets.push(0u32);
        for (counts, idx, wts) in per_row {
            for c in counts {
                let last = *offsets.last().expect("non-empty");
                offsets.push(last + c);
            }
            samples.extend(idx);
            weights.extend(wts);
        }
        Self {
            grid,
            offsets,
            samples,
            weights,
        }
    }

    pub fn has_data(&self, cell: usize) -> bool {
        self.offsets[cell + 1] > self.offsets[cell]
    }

    /// Interpolates one value per measurement onto the grid.
    /// Cells without data get `NaN`.
    pub fn apply(&self, values: &[f32]) -> Vec<f32> {
        (0..self.grid.len())
            .into_par_iter()
            .map(|c| {
                let (a, b) = (self.offsets[c] as usize, self.offsets[c + 1] as usize);
                if a == b {
                    return f32::NAN;
                }
                (a..b)
                    .map(|e| self.weights[e] * values[self.samples[e] as usize])
                    .sum()
            })
            .collect()
    }
}

/// The `k` nearest positions to `p`, sorted by distance.
fn nearest(positions: &[Vec2], p: Vec2, k: usize, out: &mut Vec<(f32, u32)>) {
    out.clear();
    for (i, q) in positions.iter().enumerate() {
        let d = p.distance(*q);
        if out.len() < k {
            out.push((d, i as u32));
            if out.len() == k {
                out.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
        } else if d < out[k - 1].0 {
            let pos = out.partition_point(|e| e.0 <= d);
            out.insert(pos, (d, i as u32));
            out.pop();
        }
    }
    if out.len() < k {
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
}

fn idw_weights(near: &[(f32, u32)], power: f32, idx: &mut Vec<u32>, wts: &mut Vec<f32>) {
    if let Some(&(d, i)) = near.first() {
        if d < 1e-4 {
            idx.push(i);
            wts.push(1.0);
            return;
        }
    }
    let raw: Vec<f32> = near.iter().map(|(d, _)| 1.0 / d.powf(power)).collect();
    let total: f32 = raw.iter().sum();
    for ((_, i), w) in near.iter().zip(raw) {
        idx.push(*i);
        wts.push(w / total);
    }
}

/// Ordinary kriging weights. Returns false if the system is singular.
fn kriging_weights(
    positions: &[Vec2],
    near: &[(f32, u32)],
    v: &Variogram,
    idx: &mut Vec<u32>,
    wts: &mut Vec<f32>,
) -> bool {
    if let Some(&(d, i)) = near.first() {
        if d < 1e-4 && v.nugget == 0.0 {
            idx.push(i);
            wts.push(1.0);
            return true;
        }
    }
    let n = near.len();
    let m = n + 1;
    // Augmented matrix [A | b], row-major, (m) × (m + 1).
    let mut a = vec![0.0f64; m * (m + 1)];
    for r in 0..n {
        let pr = positions[near[r].1 as usize];
        for c in 0..n {
            let pc = positions[near[c].1 as usize];
            a[r * (m + 1) + c] = if r == c {
                0.0
            } else {
                v.gamma(pr.distance(pc) as f64)
            };
        }
        a[r * (m + 1) + n] = 1.0;
        a[r * (m + 1) + m] = v.gamma(near[r].0 as f64);
    }
    for c in 0..n {
        a[n * (m + 1) + c] = 1.0;
    }
    a[n * (m + 1) + n] = 0.0;
    a[n * (m + 1) + m] = 1.0;

    let Some(x) = solve(&mut a, m) else {
        return false;
    };
    for (r, (_, i)) in near.iter().enumerate() {
        idx.push(*i);
        wts.push(x[r] as f32);
    }
    true
}

/// Gaussian elimination with partial pivoting on an augmented `m × (m+1)` matrix.
fn solve(a: &mut [f64], m: usize) -> Option<Vec<f64>> {
    let w = m + 1;
    for col in 0..m {
        let pivot =
            (col..m).max_by(|&x, &y| a[x * w + col].abs().total_cmp(&a[y * w + col].abs()))?;
        if a[pivot * w + col].abs() < 1e-10 {
            return None;
        }
        if pivot != col {
            for k in 0..w {
                a.swap(pivot * w + k, col * w + k);
            }
        }
        for r in col + 1..m {
            let f = a[r * w + col] / a[col * w + col];
            if f != 0.0 {
                for k in col..w {
                    a[r * w + k] -= f * a[col * w + k];
                }
            }
        }
    }
    let mut x = vec![0.0; m];
    for r in (0..m).rev() {
        let s: f64 = (r + 1..m).map(|k| a[r * w + k] * x[k]).sum();
        x[r] = (a[r * w + m] - s) / a[r * w + r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts() -> Vec<Vec2> {
        vec![
            Vec2::new(1.0, 1.0),
            Vec2::new(9.0, 1.0),
            Vec2::new(1.0, 9.0),
            Vec2::new(9.0, 9.0),
            Vec2::new(5.0, 5.0),
            Vec2::new(3.0, 7.0),
        ]
    }

    fn cell_at(g: &GridSpec, p: Vec2) -> usize {
        (p.y / g.cell_m) as usize * g.cols + (p.x / g.cell_m) as usize
    }

    fn settings(method: InterpolationMethod) -> InterpolationSettings {
        InterpolationSettings {
            method,
            max_distance_m: 20.0,
            ..Default::default()
        }
    }

    #[test]
    fn exact_at_measurement_points() {
        // Cell size 2 m with origin 0: sample centers fall on (1,1), (9,9)...
        let g = GridSpec::covering(10.0, 10.0, 2.0);
        let values = vec![-40.0, -50.0, -60.0, -70.0, -55.0, -65.0];
        let vario = Variogram {
            nugget: 0.0,
            psill: 100.0,
            range: 8.0,
        };
        for method in [InterpolationMethod::Idw, InterpolationMethod::Kriging] {
            let w = WeightGrid::build(g, &pts(), &settings(method), Some(&vario));
            let out = w.apply(&values);
            for (p, v) in pts().iter().zip(&values).take(5) {
                let got = out[cell_at(&g, *p)];
                assert!((got - v).abs() < 1e-3, "{method:?} at {p:?}: {got} vs {v}");
            }
        }
    }

    #[test]
    fn constant_field_is_reproduced() {
        let g = GridSpec::covering(10.0, 10.0, 0.5);
        let values = vec![-63.0; 6];
        for method in [InterpolationMethod::Idw, InterpolationMethod::Kriging] {
            let w = WeightGrid::build(g, &pts(), &settings(method), Some(&Variogram::FALLBACK));
            let out = w.apply(&values);
            assert!(out.iter().all(|v| (v + 63.0).abs() < 1e-3), "{method:?}");
        }
    }

    #[test]
    fn idw_stays_within_sample_range() {
        let g = GridSpec::covering(10.0, 10.0, 0.5);
        let values = vec![-40.0, -50.0, -60.0, -70.0, -55.0, -65.0];
        let w = WeightGrid::build(g, &pts(), &settings(InterpolationMethod::Idw), None);
        for v in w.apply(&values) {
            assert!((-70.0..=-40.0).contains(&v), "{v}");
        }
    }

    #[test]
    fn far_cells_have_no_data() {
        let g = GridSpec::covering(40.0, 10.0, 1.0);
        let s = InterpolationSettings {
            max_distance_m: 3.0,
            ..Default::default()
        };
        let w = WeightGrid::build(g, &[Vec2::new(2.0, 2.0)], &s, None);
        assert!(w.has_data(cell_at(&g, Vec2::new(2.5, 2.5))));
        assert!(!w.has_data(cell_at(&g, Vec2::new(30.5, 5.5))));
        assert!(w.apply(&[-50.0])[cell_at(&g, Vec2::new(30.5, 5.5))].is_nan());
    }

    #[test]
    fn variogram_fit_recovers_synthetic_model() {
        let truth = Variogram {
            nugget: 4.0,
            psill: 60.0,
            range: 12.0,
        };
        let bins: Vec<VariogramBin> = (1..=15)
            .map(|i| {
                let lag = i as f64;
                VariogramBin {
                    lag,
                    semivariance: truth.gamma(lag),
                    pairs: 50,
                }
            })
            .collect();
        let fit = fit_variogram(&bins);
        assert!((fit.nugget - truth.nugget).abs() < 1.5, "{fit:?}");
        assert!((fit.psill - truth.psill).abs() < 6.0, "{fit:?}");
        assert!((fit.range - truth.range).abs() < 2.0, "{fit:?}");
    }

    #[test]
    fn empirical_variogram_of_linear_trend_grows_with_lag() {
        let positions: Vec<Vec2> = (0..20).map(|i| Vec2::new(i as f32, 0.0)).collect();
        let values = vec![positions.iter().map(|p| -40.0 - p.x).collect::<Vec<f32>>()];
        let bins = empirical_variogram(&positions, &values, 5);
        assert!(bins.len() >= 4);
        for w in bins.windows(2) {
            assert!(w[1].semivariance > w[0].semivariance);
        }
        // γ(h) = h² / 2 for a unit slope.
        let b = bins[0];
        assert!((b.semivariance - b.lag * b.lag / 2.0).abs() < 1.0, "{b:?}");
    }

    #[test]
    fn solver_matches_known_system() {
        // 2x + y = 5, x − y = 1  →  x = 2, y = 1
        let mut a = vec![2.0, 1.0, 5.0, 1.0, -1.0, 1.0];
        let x = solve(&mut a, 2).unwrap();
        assert!((x[0] - 2.0).abs() < 1e-12 && (x[1] - 1.0).abs() < 1e-12);
        let mut singular = vec![1.0, 1.0, 1.0, 2.0, 2.0, 2.0];
        assert!(solve(&mut singular, 2).is_none());
    }
}
