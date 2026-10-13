//! HeatCraft core: data model and RF propagation engine.
//!
//! This crate has no UI, file I/O or OS dependencies.
//! World coordinates are in meters; image pixels are mapped through the
//! floor plan calibration.

pub mod ap;
pub mod channel;
pub mod geometry;
pub mod material;
pub mod predict;
pub mod propagation;
pub mod units;
