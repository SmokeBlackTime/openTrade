use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::market::Symbol;

/// Risk limits that can be applied at strategy, symbol, or portfolio level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLimits {
    pub max_position_size_usd: Decimal,
    pub max_leverage: Decimal,
    pub max_daily_loss_pct: Decimal,
    pub max_drawdown_pct: Decimal,
    pub max_open_positions: usize,
    pub max_notional_exposure_usd: Decimal,
    pub max_single_order_usd: Decimal,
    pub max_trades_per_day: usize,
    pub max_correlated_exposure_pct: Decimal,
}

/// Kill switch states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KillSwitchState {
    Active,
    Triggered,
}

/// Types of kill switches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillSwitchType {
    Global,
    PerSymbol(Symbol),
    ExchangeConnectivity,
    StaleMarketData,
    ModelConfidence,
    ExtremeVolatility,
    OrderRejectionAnomaly,
    RunawayTrading,
    DailyLossLimit,
    MaxDrawdown,
}

impl std::fmt::Display for KillSwitchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Global => write!(f, "global"),
            Self::PerSymbol(s) => write!(f, "symbol:{}", s),
            Self::ExchangeConnectivity => write!(f, "exchange_connectivity"),
            Self::StaleMarketData => write!(f, "stale_market_data"),
            Self::ModelConfidence => write!(f, "model_confidence"),
            Self::ExtremeVolatility => write!(f, "extreme_volatility"),
            Self::OrderRejectionAnomaly => write!(f, "order_rejection_anomaly"),
            Self::RunawayTrading => write!(f, "runaway_trading"),
            Self::DailyLossLimit => write!(f, "daily_loss_limit"),
            Self::MaxDrawdown => write!(f, "max_drawdown"),
        }
    }
}

/// Risk check verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RiskVerdict {
    Approved,
    Rejected { reason: String },
    ReducedSize { new_quantity: Decimal, reason: String },
}

/// Portfolio-level risk snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioRiskSnapshot {
    pub total_equity: Decimal,
    pub total_notional_exposure: Decimal,
    pub total_unrealized_pnl: Decimal,
    pub total_realized_pnl_today: Decimal,
    pub current_drawdown_pct: Decimal,
    pub peak_equity: Decimal,
    pub open_position_count: usize,
    pub leverage_ratio: Decimal,
    pub var_95: Option<Decimal>,
    pub cvar_95: Option<Decimal>,
}
