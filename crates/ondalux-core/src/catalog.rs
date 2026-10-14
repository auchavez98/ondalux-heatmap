//! Access point model catalog, grouped by vendor.
//!
//! The built-in catalog (`catalog/ap-models.json`) only contains values
//! published by each vendor, with the source URL of every model. See
//! `docs/ap-catalog.md` for how the figures are interpreted.

use crate::ap::{AccessPoint, Radio};
use crate::channel::{Band, Channel, ChannelWidth};
use crate::units::{Db, Dbm};
use serde::{Deserialize, Serialize};
use std::fmt;

/// One radio of an AP model as published in its datasheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioSpec {
    pub band: Band,
    /// Maximum transmit power published by the vendor, in dBm.
    /// See [`ApModel::tx_power_basis`] for what the figure means.
    pub max_tx_power_dbm: f32,
    /// Peak antenna gain for this band, in dBi.
    pub antenna_gain_dbi: f32,
    /// Spatial streams (MIMO chains), e.g. 4 for 4x4:4.
    pub spatial_streams: u8,
    /// Widest channel supported on this band.
    pub max_width: ChannelWidth,
}

/// How a model's `max_tx_power_dbm` figures should be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TxPowerBasis {
    /// Total conducted power summed over all chains (no antenna gain).
    AggregateConducted,
    /// Conducted power of a single chain.
    PerChainConducted,
    /// The vendor's "max TX power" without further qualification.
    #[default]
    VendorMax,
    /// Regulatory EIRP limit quoted by the vendor (already includes gain).
    Eirp,
}

impl TxPowerBasis {
    pub fn label(self) -> &'static str {
        match self {
            TxPowerBasis::AggregateConducted => "aggregate conducted",
            TxPowerBasis::PerChainConducted => "per chain, conducted",
            TxPowerBasis::VendorMax => "vendor max TX power",
            TxPowerBasis::Eirp => "EIRP limit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApModel {
    pub vendor: String,
    pub model: String,
    /// e.g. "Wi-Fi 6", "Wi-Fi 6E", "Wi-Fi 7".
    pub standard: String,
    /// "indoor" or "outdoor".
    #[serde(default = "default_environment")]
    pub environment: String,
    pub radios: Vec<RadioSpec>,
    #[serde(default)]
    pub tx_power_basis: TxPowerBasis,
    /// Where the figures come from (vendor datasheet / spec page).
    pub source: String,
    #[serde(default)]
    pub notes: String,
}

fn default_environment() -> String {
    "indoor".into()
}

impl ApModel {
    pub fn bands(&self) -> String {
        self.radios
            .iter()
            .map(|r| r.band.label())
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Transmit power to start a design with, for one radio. Per-chain
    /// figures are converted to aggregate (+10·log10(chains)) and EIRP
    /// limits to conducted power (− antenna gain), so the engine always
    /// receives a conducted total.
    pub fn design_tx_power(&self, r: &RadioSpec) -> Dbm {
        let p = match self.tx_power_basis {
            TxPowerBasis::PerChainConducted => {
                r.max_tx_power_dbm + 10.0 * f32::from(r.spatial_streams.max(1)).log10()
            }
            TxPowerBasis::Eirp => r.max_tx_power_dbm - r.antenna_gain_dbi,
            TxPowerBasis::AggregateConducted | TxPowerBasis::VendorMax => r.max_tx_power_dbm,
        };
        Dbm((p * 2.0).round() / 2.0)
    }

    /// Radios for a new AP of this model. `index` rotates the default
    /// channels so neighbouring APs do not all start on the same channel.
    pub fn build_radios(&self, index: usize) -> Vec<Radio> {
        let mut per_band = [0usize; 3];
        self.radios
            .iter()
            .map(|spec| {
                let slot = &mut per_band[spec.band as usize];
                let nth = *slot;
                *slot += 1;
                let (pool, width): (&[u16], ChannelWidth) = match spec.band {
                    Band::G2_4 => (&[1, 6, 11], ChannelWidth::Mhz20),
                    // A second 5 GHz radio (tri-radio APs) starts in UNII-3.
                    Band::G5 if nth > 0 => (&[149, 157, 165], ChannelWidth::Mhz20),
                    Band::G5 => (&[36, 52, 100, 116, 132, 149], ChannelWidth::Mhz40),
                    Band::G6 => (&[5, 21, 37, 53, 69, 85], ChannelWidth::Mhz80),
                };
                let width = width.min(spec.max_width);
                let number = pool[index % pool.len()];
                Radio {
                    channel: Channel::closest(spec.band, number, width),
                    tx_power: self.design_tx_power(spec),
                    antenna_gain: Db(spec.antenna_gain_dbi),
                    enabled: true,
                }
            })
            .collect()
    }

    /// Replaces the AP's vendor, model and radios with this model's,
    /// keeping its id, name, position, height and orientation. Channels of
    /// existing radios on the same band are kept (width clamped to what the
    /// model supports); new radios get rotating defaults.
    pub fn apply_to(&self, ap: &mut AccessPoint, index: usize) {
        let mut radios = self.build_radios(index);
        let mut used = vec![false; ap.radios.len()];
        for (new, spec) in radios.iter_mut().zip(&self.radios) {
            let old = ap
                .radios
                .iter()
                .enumerate()
                .find(|(i, r)| !used[*i] && r.channel.band == spec.band);
            if let Some((i, old)) = old {
                used[i] = true;
                let width = old.channel.width.min(spec.max_width);
                new.channel = Channel::closest(spec.band, old.channel.number, width);
                new.enabled = old.enabled;
            }
        }
        ap.vendor = self.vendor.clone();
        ap.model = self.model.clone();
        ap.radios = radios;
    }

    /// Checks the entry for obviously wrong values.
    pub fn validate(&self) -> Result<(), String> {
        if self.vendor.trim().is_empty() || self.model.trim().is_empty() {
            return Err("vendor and model are required".into());
        }
        if self.radios.is_empty() {
            return Err(format!("{}: no radios", self.model));
        }
        for r in &self.radios {
            if !(-10.0..=36.0).contains(&r.max_tx_power_dbm) {
                return Err(format!(
                    "{}: TX power {} dBm out of range",
                    self.model, r.max_tx_power_dbm
                ));
            }
            if !(-5.0..=25.0).contains(&r.antenna_gain_dbi) {
                return Err(format!(
                    "{}: antenna gain {} dBi out of range",
                    self.model, r.antenna_gain_dbi
                ));
            }
            if !(1..=16).contains(&r.spatial_streams) {
                return Err(format!(
                    "{}: {} spatial streams",
                    self.model, r.spatial_streams
                ));
            }
            if !r.band.widths().contains(&r.max_width) {
                return Err(format!(
                    "{}: {} MHz is not valid in {}",
                    self.model,
                    r.max_width.mhz(),
                    r.band
                ));
            }
        }
        Ok(())
    }
}

impl fmt::Display for ApModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.vendor, self.model)
    }
}

/// Where a catalog entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CatalogFile {
    pub models: Vec<ApModel>,
}

/// Built-in models plus the user's own; user entries replace built-in
/// entries with the same vendor and model.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<(ApModel, Origin)>,
}

const BUILTIN_JSON: &str = include_str!("../catalog/ap-models.json");

fn key(m: &ApModel) -> (String, String) {
    (m.vendor.to_lowercase(), m.model.to_lowercase())
}

impl Catalog {
    /// The catalog shipped with the application.
    pub fn builtin() -> Self {
        let file: CatalogFile =
            serde_json::from_str(BUILTIN_JSON).expect("built-in AP catalog is valid JSON");
        Self {
            entries: file
                .models
                .into_iter()
                .map(|m| (m, Origin::BuiltIn))
                .collect(),
        }
    }

    /// Adds or replaces user models.
    pub fn merge_user(&mut self, models: Vec<ApModel>) {
        for m in models {
            self.upsert_user(m);
        }
    }

    pub fn upsert_user(&mut self, model: ApModel) {
        let k = key(&model);
        self.entries.retain(|(e, _)| key(e) != k);
        self.entries.push((model, Origin::User));
    }

    /// Removes a user model. Built-in models cannot be removed (a user model
    /// with the same name only shadows them).
    pub fn remove_user(&mut self, vendor: &str, model: &str) -> bool {
        let k = (vendor.to_lowercase(), model.to_lowercase());
        let before = self.entries.len();
        self.entries
            .retain(|(e, o)| !(*o == Origin::User && key(e) == k));
        let removed = self.entries.len() != before;
        if removed {
            // Restore the built-in entry it may have shadowed.
            if let Some(b) = Catalog::builtin()
                .entries
                .into_iter()
                .find(|(e, _)| key(e) == k)
            {
                self.entries.push(b);
            }
        }
        removed
    }

    pub fn user_models(&self) -> Vec<ApModel> {
        self.entries
            .iter()
            .filter(|(_, o)| *o == Origin::User)
            .map(|(m, _)| m.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Vendor names, sorted case-insensitively.
    pub fn vendors(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entries.iter().map(|(m, _)| m.vendor.clone()).collect();
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        v
    }

    /// Models of one vendor, sorted by model name.
    pub fn models_of(&self, vendor: &str) -> Vec<(&ApModel, Origin)> {
        let mut v: Vec<_> = self
            .entries
            .iter()
            .filter(|(m, _)| m.vendor.eq_ignore_ascii_case(vendor))
            .map(|(m, o)| (m, *o))
            .collect();
        v.sort_by_key(|(m, _)| m.model.to_lowercase());
        v
    }

    pub fn find(&self, vendor: &str, model: &str) -> Option<&ApModel> {
        let k = (vendor.to_lowercase(), model.to_lowercase());
        self.entries
            .iter()
            .find(|(m, _)| key(m) == k)
            .map(|(m, _)| m)
    }

    /// Case-insensitive search over vendor, model and standard.
    pub fn search(&self, query: &str) -> Vec<(&ApModel, Origin)> {
        let q = query.trim().to_lowercase();
        let mut v: Vec<_> = self
            .entries
            .iter()
            .filter(|(m, _)| {
                q.is_empty()
                    || format!("{} {} {}", m.vendor, m.model, m.standard)
                        .to_lowercase()
                        .contains(&q)
            })
            .map(|(m, o)| (m, *o))
            .collect();
        v.sort_by_key(|(m, _)| (m.vendor.to_lowercase(), m.model.to_lowercase()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ap::ApId;
    use crate::geometry::Vec2;

    #[test]
    fn builtin_catalog_is_valid() {
        let c = Catalog::builtin();
        assert!(!c.is_empty());
        let mut seen = std::collections::HashSet::new();
        for (m, _) in &c.entries {
            m.validate().unwrap_or_else(|e| panic!("{m}: {e}"));
            assert!(
                m.source.starts_with("https://"),
                "{m}: source must be a URL"
            );
            assert!(seen.insert(key(m)), "{m}: duplicate entry");
            // Every model's default radios must be buildable.
            assert_eq!(m.build_radios(3).len(), m.radios.len());
        }
    }

    fn sample(basis: TxPowerBasis) -> ApModel {
        ApModel {
            vendor: "Acme".into(),
            model: "X1".into(),
            standard: "Wi-Fi 6".into(),
            environment: "indoor".into(),
            radios: vec![
                RadioSpec {
                    band: Band::G2_4,
                    max_tx_power_dbm: 20.0,
                    antenna_gain_dbi: 4.0,
                    spatial_streams: 2,
                    max_width: ChannelWidth::Mhz40,
                },
                RadioSpec {
                    band: Band::G5,
                    max_tx_power_dbm: 21.0,
                    antenna_gain_dbi: 5.0,
                    spatial_streams: 4,
                    max_width: ChannelWidth::Mhz160,
                },
            ],
            tx_power_basis: basis,
            source: "https://example.com".into(),
            notes: String::new(),
        }
    }

    #[test]
    fn design_power_conversions() {
        let r = &sample(TxPowerBasis::VendorMax).radios[1];
        assert_eq!(
            sample(TxPowerBasis::VendorMax).design_tx_power(r),
            Dbm(21.0)
        );
        // 4 chains: +6 dB
        assert_eq!(
            sample(TxPowerBasis::PerChainConducted).design_tx_power(r),
            Dbm(27.0)
        );
        assert_eq!(sample(TxPowerBasis::Eirp).design_tx_power(r), Dbm(16.0));
    }

    #[test]
    fn apply_sets_radios_and_keeps_identity() {
        let m = sample(TxPowerBasis::VendorMax);
        let mut ap = AccessPoint {
            id: ApId(9),
            name: "Lobby".into(),
            vendor: String::new(),
            model: "Generic".into(),
            position: Vec2::new(1.0, 2.0),
            height_m: 4.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: Vec::new(),
        };
        m.apply_to(&mut ap, 1);
        assert_eq!((ap.vendor.as_str(), ap.model.as_str()), ("Acme", "X1"));
        assert_eq!(ap.name, "Lobby");
        assert_eq!(ap.radios.len(), 2);
        assert_eq!(ap.radios[0].channel.number, 6);
        assert_eq!(ap.radios[0].antenna_gain, Db(4.0));
        assert_eq!(ap.radios[1].channel.width, ChannelWidth::Mhz40);

        // Re-applying keeps the user's channels.
        ap.radios[1].channel = Channel::new(Band::G5, 149, ChannelWidth::Mhz80).unwrap();
        ap.radios[0].enabled = false;
        m.apply_to(&mut ap, 0);
        assert_eq!(ap.radios[1].channel.number, 149);
        assert_eq!(ap.radios[1].channel.width, ChannelWidth::Mhz80);
        assert!(!ap.radios[0].enabled);
        assert_eq!(ap.radios[0].channel.number, 6);
    }

    #[test]
    fn user_models_shadow_and_restore_builtin() {
        let mut c = Catalog::builtin();
        let (vendor, model) = {
            let (m, _) = &c.entries[0];
            (m.vendor.clone(), m.model.clone())
        };
        let n = c.len();
        let mut custom = c.find(&vendor, &model).unwrap().clone();
        custom.notes = "site-specific".into();
        c.upsert_user(custom);
        assert_eq!(c.len(), n);
        assert_eq!(c.find(&vendor, &model).unwrap().notes, "site-specific");
        assert!(c.remove_user(&vendor, &model));
        assert_ne!(c.find(&vendor, &model).unwrap().notes, "site-specific");
        assert_eq!(c.len(), n);
        c.upsert_user(sample(TxPowerBasis::VendorMax));
        assert!(c.vendors().iter().any(|v| v == "Acme"));
        assert_eq!(c.search("acme x1").len(), 1);
    }

    #[test]
    fn validation_rejects_bad_values() {
        let mut m = sample(TxPowerBasis::VendorMax);
        m.radios[0].max_width = ChannelWidth::Mhz160;
        assert!(m.validate().is_err());
        let mut m = sample(TxPowerBasis::VendorMax);
        m.radios[0].max_tx_power_dbm = 60.0;
        assert!(m.validate().is_err());
    }
}
