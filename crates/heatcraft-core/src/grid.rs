//! Regular sampling grid over the floor plan and the per-radio signal cache.
//!
//! Each enabled radio gets its own RSSI layer. Layers are cached together
//! with a signature of the inputs that affect them, so moving one AP only
//! recomputes that AP's layers; combined metrics are then re-aggregated
//! (see [`crate::metrics`]).

use crate::ap::{AccessPoint, ApId};
use crate::channel::Channel;
use crate::geometry::Vec2;
use crate::predict::{rssi_at, Obstacles};
use crate::propagation::PropagationSettings;
use crate::units::Dbm;
use rayon::prelude::*;
use std::collections::HashMap;

/// Sampling grid. Sample `(c, r)` is at the center of its cell:
/// `origin + ((c + 0.5)·cell, (r + 0.5)·cell)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSpec {
    pub origin: Vec2,
    pub cell_m: f32,
    pub cols: usize,
    pub rows: usize,
}

impl GridSpec {
    /// Grid covering a `width_m × height_m` area starting at (0, 0).
    pub fn covering(width_m: f32, height_m: f32, cell_m: f32) -> Self {
        assert!(cell_m > 0.0, "cell_m must be positive");
        Self {
            origin: Vec2::default(),
            cell_m,
            cols: (width_m / cell_m).ceil().max(1.0) as usize,
            rows: (height_m / cell_m).ceil().max(1.0) as usize,
        }
    }

    pub fn len(&self) -> usize {
        self.cols * self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn sample(&self, col: usize, row: usize) -> Vec2 {
        Vec2::new(
            self.origin.x + (col as f32 + 0.5) * self.cell_m,
            self.origin.y + (row as f32 + 0.5) * self.cell_m,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RadioKey {
    pub ap: ApId,
    pub radio: usize,
}

/// Inputs that determine a radio layer. If unchanged, the layer is reused.
#[derive(Debug, Clone, PartialEq)]
struct Signature {
    position: Vec2,
    height_m: f32,
    channel: Channel,
    eirp: Dbm,
}

#[derive(Debug, Clone)]
pub struct RadioLayer {
    pub key: RadioKey,
    pub channel: Channel,
    /// RSSI in dBm, row-major, `GridSpec::len()` values.
    pub rssi: Vec<f32>,
    signature: Signature,
}

/// Cache of per-radio RSSI layers for one grid.
#[derive(Debug, Clone)]
pub struct LayerCache {
    grid: GridSpec,
    layers: HashMap<RadioKey, RadioLayer>,
}

impl LayerCache {
    pub fn new(grid: GridSpec) -> Self {
        Self {
            grid,
            layers: HashMap::new(),
        }
    }

    pub fn grid(&self) -> &GridSpec {
        &self.grid
    }

    /// Drops every layer, e.g. after walls, materials or settings change.
    pub fn invalidate_all(&mut self) {
        self.layers.clear();
    }

    /// Layers sorted by key, so aggregation is deterministic.
    pub fn layers(&self) -> Vec<&RadioLayer> {
        let mut v: Vec<_> = self.layers.values().collect();
        v.sort_by_key(|l| l.key);
        v
    }

    /// Brings the cache in line with `aps`: computes new or changed layers,
    /// drops layers of removed APs or disabled radios.
    /// Returns how many layers were recomputed.
    pub fn sync(
        &mut self,
        aps: &[AccessPoint],
        obstacles: &Obstacles,
        settings: &PropagationSettings,
    ) -> usize {
        let mut wanted = HashMap::new();
        for ap in aps {
            for (idx, radio) in ap.enabled_radios() {
                let key = RadioKey {
                    ap: ap.id,
                    radio: idx,
                };
                let sig = Signature {
                    position: ap.position,
                    height_m: ap.height_m,
                    channel: radio.channel,
                    eirp: radio.eirp(),
                };
                wanted.insert(key, (ap, idx, sig));
            }
        }
        self.layers
            .retain(|k, l| wanted.get(k).is_some_and(|(_, _, sig)| *sig == l.signature));
        let mut recomputed = 0;
        for (key, (ap, idx, sig)) in wanted {
            if self.layers.contains_key(&key) {
                continue;
            }
            let rssi = compute_layer(&self.grid, ap, idx, obstacles, settings);
            self.layers.insert(
                key,
                RadioLayer {
                    key,
                    channel: ap.radios[idx].channel,
                    rssi,
                    signature: sig,
                },
            );
            recomputed += 1;
        }
        recomputed
    }
}

/// RSSI of one radio over the whole grid, computed in parallel by rows.
pub fn compute_layer(
    grid: &GridSpec,
    ap: &AccessPoint,
    radio_idx: usize,
    obstacles: &Obstacles,
    settings: &PropagationSettings,
) -> Vec<f32> {
    let radio = &ap.radios[radio_idx];
    let mut out = vec![0.0f32; grid.len()];
    out.par_chunks_mut(grid.cols.max(1))
        .enumerate()
        .for_each_init(
            || obstacles.scratch(),
            |scratch, (row, line)| {
                for (col, v) in line.iter_mut().enumerate() {
                    let p = grid.sample(col, row);
                    *v = rssi_at(ap, radio, p, obstacles, settings, scratch).0;
                }
            },
        );
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ap::Radio;
    use crate::channel::{Band, ChannelWidth};
    use crate::material::MaterialLibrary;
    use crate::units::Db;

    pub(crate) fn test_ap(id: u32, x: f32, y: f32, ch: u16) -> AccessPoint {
        AccessPoint {
            id: ApId(id),
            name: format!("AP{id}"),
            model: String::new(),
            position: Vec2::new(x, y),
            height_m: 3.0,
            orientation_deg: 0.0,
            radios: vec![Radio {
                channel: Channel::new(Band::G5, ch, ChannelWidth::Mhz20).unwrap(),
                tx_power: Dbm(17.0),
                antenna_gain: Db(4.0),
                enabled: true,
            }],
        }
    }

    #[test]
    fn grid_samples_cell_centers() {
        let g = GridSpec::covering(10.0, 5.0, 0.5);
        assert_eq!((g.cols, g.rows), (20, 10));
        assert_eq!(g.sample(0, 0), Vec2::new(0.25, 0.25));
    }

    #[test]
    fn moving_one_ap_recomputes_only_its_layer() {
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let settings = PropagationSettings::default();
        let mut cache = LayerCache::new(GridSpec::covering(20.0, 20.0, 1.0));
        let mut aps = vec![test_ap(1, 5.0, 5.0, 36), test_ap(2, 15.0, 15.0, 44)];
        assert_eq!(cache.sync(&aps, &obs, &settings), 2);
        assert_eq!(cache.sync(&aps, &obs, &settings), 0);
        aps[1].position = Vec2::new(10.0, 10.0);
        assert_eq!(cache.sync(&aps, &obs, &settings), 1);
        aps.remove(0);
        assert_eq!(cache.sync(&aps, &obs, &settings), 0);
        assert_eq!(cache.layers().len(), 1);
    }

    #[test]
    fn disabling_a_radio_drops_its_layer() {
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let settings = PropagationSettings::default();
        let mut cache = LayerCache::new(GridSpec::covering(10.0, 10.0, 1.0));
        let mut aps = vec![test_ap(1, 5.0, 5.0, 36)];
        cache.sync(&aps, &obs, &settings);
        aps[0].radios[0].enabled = false;
        cache.sync(&aps, &obs, &settings);
        assert!(cache.layers().is_empty());
    }

    #[test]
    fn layer_is_strongest_near_the_ap() {
        let obs = Obstacles::new(&[], &MaterialLibrary::default());
        let g = GridSpec::covering(20.0, 20.0, 1.0);
        let ap = test_ap(1, 2.5, 2.5, 36);
        let layer = compute_layer(&g, &ap, 0, &obs, &PropagationSettings::default());
        let max_idx = layer
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap()
            .0;
        assert_eq!(max_idx, 2 * g.cols + 2);
    }
}
