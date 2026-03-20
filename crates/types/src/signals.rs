use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::market::{MarketType, Symbol, Timeframe};
use crate::orders::Side;

/// A trading signal emitted by a strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub strategy_name: String,
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timeframe: Timeframe,
    pub timestamp: DateTime<Utc>,
    pub direction: SignalDirection,
    pub strength: Decimal,
    pub confidence: Decimal,
    pub entry_price: Option<Decimal>,
    pub stop_loss: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    pub time_stop_bars: Option<u32>,
    pub metadata: SignalMetadata,
}

/// Signal direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalDirection {
    Long,
    Short,
    Flat,
    ReduceLong,
    ReduceShort,
}

impl SignalDirection {
    pub fn to_side(self) -> Option<Side> {
        match self {
            Self::Long | Self::ReduceShort => Some(Side::Buy),
            Self::Short | Self::ReduceLong => Some(Side::Sell),
            Self::Flat => None,
        }
    }

    pub fn is_entry(self) -> bool {
        matches!(self, Self::Long | Self::Short)
    }
}

/// Metadata for explainability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalMetadata {
    pub signal_inputs: serde_json::Value,
    pub model_outputs: Option<serde_json::Value>,
    pub uncertainty_score: Option<Decimal>,
    pub regime: Option<String>,
    pub risk_overrides: Vec<String>,
    pub portfolio_context: Option<String>,
}

/// Trade decision combining signal + risk + execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDecision {
    pub signal: Signal,
    pub risk_approved: bool,
    pub risk_notes: Vec<String>,
    pub position_size: Decimal,
    pub notional_value: Decimal,
    pub estimated_slippage_bps: Decimal,
    pub estimated_commission: Decimal,
    pub decision_timestamp: DateTime<Utc>,
    pub execute: bool,
}
