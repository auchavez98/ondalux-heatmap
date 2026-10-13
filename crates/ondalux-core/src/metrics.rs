//! Combined per-cell metrics derived from the cached radio layers:
//! best RSSI, SNR, SINR (co-/adjacent-channel interference), number of APs
//! covering each point and channel overlap.

use crate::channel::Band;
use crate::grid::{GridSpec, RadioLayer};
use crate::propagation::PropagationSettings;
use crate::units::{Db, Dbm};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Design requirements used for coverage and counting metrics.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Requirements {
    /// Minimum signal for a point to count as covered (e.g. −67 dBm for voice/video).
    pub min_rssi: Dbm,
    /// Minimum SNR for a point to count as covered.
    pub min_snr: Db,
    /// Level above which another radio on an overlapping channel is
    /// considered audible (clear channel assessment threshold).
    pub cca_threshold: Dbm,
}

impl Default for Requirements {
    fn default() -> Self {
        Self {
            min_rssi: Dbm(-67.0),
            min_snr: Db(25.0),
            cca_threshold: Dbm(-82.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricKind {
    /// Strongest received signal (dBm).
    Rssi,
    /// Strongest signal over the noise floor (dB).
    Snr,
    /// Strongest signal over noise plus interference (dB).
    Sinr,
    /// Number of APs heard at or above `min_rssi`.
    ApCount,
    /// Number of other radios on an overlapping channel heard above the CCA threshold.
    ChannelOverlap,
    /// 1 where `min_rssi` and `min_snr` are met, 0 elsewhere.
    Coverage,
}

impl MetricKind {
    pub const ALL: [MetricKind; 6] = [
        MetricKind::Rssi,
        MetricKind::Snr,
        MetricKind::Sinr,
        MetricKind::ApCount,
        MetricKind::ChannelOverlap,
        MetricKind::Coverage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MetricKind::Rssi => "Signal strength",
            MetricKind::Snr => "SNR",
            MetricKind::Sinr => "SINR",
            MetricKind::ApCount => "AP count",
            MetricKind::ChannelOverlap => "Channel overlap",
            MetricKind::Coverage => "Coverage requirement",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            MetricKind::Rssi => "dBm",
            MetricKind::Snr | MetricKind::Sinr => "dB",
            MetricKind::ApCount | MetricKind::ChannelOverlap | MetricKind::Coverage => "",
        }
    }
}

pub const NO_SERVER: u32 = u32::MAX;

/// Metrics for every grid cell, row-major.
#[derive(Debug, Clone)]
pub struct Metrics {
    pub grid: GridSpec,
    pub band: Option<Band>,
    pub requirements: Requirements,
    pub rssi: Vec<f32>,
    pub snr: Vec<f32>,
    pub sinr: Vec<f32>,
    pub ap_count: Vec<u8>,
    pub overlap: Vec<u8>,
    /// Index into the layer slice used to compute these metrics, or [`NO_SERVER`].
    pub serving: Vec<u32>,
}

impl Metrics {
    /// Aggregates `layers` (optionally only those in `band`).
    pub fn compute(
        grid: GridSpec,
        layers: &[&RadioLayer],
        band: Option<Band>,
        settings: &PropagationSettings,
        requirements: Requirements,
    ) -> Self {
        let layers: Vec<&RadioLayer> = layers
            .iter()
            .copied()
            .filter(|l| band.is_none_or(|b| l.channel.band == b))
            .collect();
        let ranges: Vec<_> = layers.iter().map(|l| l.channel.range()).collect();
        let noise: Vec<f32> = layers
            .iter()
            .map(|l| settings.noise_floor(l.channel.width).to_milliwatts())
            .collect();
        let n = grid.len();
        let mut rssi = vec![Dbm::FLOOR.0; n];
        let mut snr = vec![0.0; n];
        let mut sinr = vec![0.0; n];
        let mut ap_count = vec![0u8; n];
        let mut overlap = vec![0u8; n];
        let mut serving = vec![NO_SERVER; n];
        let default_noise_mw = settings
            .noise_floor(crate::channel::ChannelWidth::Mhz20)
            .to_milliwatts();

        (
            &mut rssi,
            &mut snr,
            &mut sinr,
            &mut ap_count,
            &mut overlap,
            &mut serving,
        )
            .into_par_iter()
            .enumerate()
            .for_each(|(i, (rssi, snr, sinr, count, ovl, srv))| {
                let mut best = NO_SERVER;
                let mut best_v = f32::NEG_INFINITY;
                let mut covered = 0u8;
                let mut last_ap = None;
                for (j, l) in layers.iter().enumerate() {
                    let v = l.rssi[i];
                    if v > best_v {
                        best_v = v;
                        best = j as u32;
                    }
                    if v >= requirements.min_rssi.0 && last_ap != Some(l.key.ap) {
                        covered = covered.saturating_add(1);
                        last_ap = Some(l.key.ap);
                    }
                }
                *count = covered;
                if best == NO_SERVER {
                    let n_db = Dbm::from_milliwatts(default_noise_mw).0;
                    *snr = Dbm::FLOOR.0 - n_db;
                    *sinr = *snr;
                    return;
                }
                let b = best as usize;
                *rssi = best_v;
                *srv = best;
                let serving_range = &ranges[b];
                let mut interference_mw = 0.0f32;
                let mut audible = 0u8;
                for (j, l) in layers.iter().enumerate() {
                    if j == b || l.key.ap == layers[b].key.ap {
                        continue;
                    }
                    let ov = serving_range.overlap_mhz(&ranges[j]);
                    if ov <= 0.0 {
                        continue;
                    }
                    let v = l.rssi[i];
                    // Only the overlapping fraction of the interferer's power
                    // falls inside the serving channel.
                    interference_mw += Dbm(v).to_milliwatts() * (ov / ranges[j].width_mhz());
                    if v >= requirements.cca_threshold.0 {
                        audible = audible.saturating_add(1);
                    }
                }
                *ovl = audible;
                let n_mw = noise[b];
                *snr = best_v - Dbm::from_milliwatts(n_mw).0;
                *sinr = best_v - Dbm::from_milliwatts(n_mw + interference_mw).0;
            });

        Self {
            grid,
            band,
            requirements,
            rssi,
            snr,
            sinr,
            ap_count,
            overlap,
            serving,
        }
    }

    pub fn meets_requirements(&self, i: usize) -> bool {
        self.serving[i] != NO_SERVER
            && self.rssi[i] >= self.requirements.min_rssi.0
            && self.snr[i] >= self.requirements.min_snr.0
    }

    /// Value of `kind` at cell `i`, as a float for coloring.
    pub fn value(&self, kind: MetricKind, i: usize) -> f32 {
        match kind {
            MetricKind::Rssi => self.rssi[i],
            MetricKind::Snr => self.snr[i],
            MetricKind::Sinr => self.sinr[i],
            MetricKind::ApCount => self.ap_count[i] as f32,
            MetricKind::ChannelOverlap => self.overlap[i] as f32,
            MetricKind::Coverage => u8::from(self.meets_requirements(i)) as f32,
        }
    }

    /// Fraction of cells meeting the requirements (0–1).
    pub fn coverage_ratio(&self) -> f32 {
        let n = self.grid.len();
        if n == 0 {
            return 0.0;
        }
        (0..n).filter(|&i| self.meets_requirements(i)).count() as f32 / n as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::tests::test_ap;
    use crate::grid::LayerCache;
    use crate::material::MaterialLibrary;
    use crate::predict::Obstacles;

    fn metrics_for(aps: &[crate::ap::AccessPoint]) -> Metrics {
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let settings = PropagationSettings::default();
        let mut cache = LayerCache::new(GridSpec::covering(40.0, 10.0, 1.0));
        cache.sync(aps, &obs, &settings);
        Metrics::compute(
            *cache.grid(),
            &cache.layers(),
            Some(Band::G5),
            &settings,
            Requirements::default(),
        )
    }

    fn cell(m: &Metrics, x: f32, y: f32) -> usize {
        (y / m.grid.cell_m) as usize * m.grid.cols + (x / m.grid.cell_m) as usize
    }

    #[test]
    fn single_ap_has_no_interference() {
        let m = metrics_for(&[test_ap(1, 5.0, 5.0, 36)]);
        let i = cell(&m, 8.0, 5.0);
        assert!((m.snr[i] - m.sinr[i]).abs() < 1e-3);
        assert_eq!(m.overlap[i], 0);
        assert_eq!(m.ap_count[i], 1);
    }

    #[test]
    fn co_channel_ap_degrades_sinr_and_counts_as_overlap() {
        let m = metrics_for(&[test_ap(1, 5.0, 5.0, 36), test_ap(2, 35.0, 5.0, 36)]);
        let i = cell(&m, 10.0, 5.0);
        assert!(m.sinr[i] < m.snr[i] - 1.0, "{} vs {}", m.sinr[i], m.snr[i]);
        assert_eq!(m.overlap[i], 1);
    }

    #[test]
    fn non_overlapping_channels_do_not_interfere() {
        let m = metrics_for(&[test_ap(1, 5.0, 5.0, 36), test_ap(2, 35.0, 5.0, 149)]);
        let i = cell(&m, 10.0, 5.0);
        assert!((m.snr[i] - m.sinr[i]).abs() < 1e-3);
        assert_eq!(m.overlap[i], 0);
    }

    #[test]
    fn serving_ap_is_the_closest() {
        let aps = [test_ap(1, 5.0, 5.0, 36), test_ap(2, 35.0, 5.0, 149)];
        let m = metrics_for(&aps);
        assert_eq!(m.serving[cell(&m, 2.0, 5.0)], 0);
        assert_eq!(m.serving[cell(&m, 38.0, 5.0)], 1);
    }

    #[test]
    fn band_filter_excludes_other_bands() {
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let settings = PropagationSettings::default();
        let mut cache = LayerCache::new(GridSpec::covering(10.0, 10.0, 1.0));
        cache.sync(&[test_ap(1, 5.0, 5.0, 36)], &obs, &settings);
        let m = Metrics::compute(
            *cache.grid(),
            &cache.layers(),
            Some(Band::G2_4),
            &settings,
            Requirements::default(),
        );
        assert!(m.serving.iter().all(|&s| s == NO_SERVER));
        assert_eq!(m.coverage_ratio(), 0.0);
    }
}
