//! Feature engineering layer for OpenTrade.
//!
//! Computes technical indicators and statistical features from candle data.
//! All computations use Decimal to avoid float representation errors.
//! Features are computed incrementally where possible.

pub mod indicators;
pub mod pipeline;

pub use indicators::*;
pub use pipeline::FeatureRow;
