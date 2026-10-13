//! Ondalux Heatmap reports: heat map images (PNG) and PDF reports.

pub mod map;
pub mod pdfdoc;
pub mod raster;
pub mod report;

pub use report::{build_pdf, Lang, ReportOptions};
