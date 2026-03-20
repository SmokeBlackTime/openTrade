//! Core domain types for the OpenTrade trading system.
//!
//! All monetary values use `rust_decimal::Decimal` to avoid floating-point
//! representation errors in accounting-sensitive paths.

pub mod decimal_ext;
pub mod events;
pub mod market;
pub mod orders;
pub mod positions;
pub mod risk;
pub mod signals;
pub mod trade;

pub use decimal_ext::DecimalExt;
pub use market::*;
pub use orders::*;
pub use positions::*;
pub use risk::*;
pub use signals::*;
pub use trade::*;
