use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::market::{MarketType, Symbol};
use crate::orders::{ClientOrderId, Side};

/// A completed trade record for journaling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub trade_id: String,
    pub client_order_id: ClientOrderId,
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub side: Side,
    pub quantity: Decimal,
    pub price: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
    pub realized_pnl: Option<Decimal>,
    pub strategy_name: String,
    pub timestamp: DateTime<Utc>,
}

impl TradeRecord {
    pub fn new_id() -> String {
        Uuid::new_v4().to_string()
    }

    pub fn notional(&self) -> Decimal {
        self.quantity * self.price
    }
}

/// Performance metrics over a period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub total_return: Decimal,
    pub total_return_pct: Decimal,
    pub annualized_return_pct: Option<Decimal>,
    pub max_drawdown_pct: Decimal,
    pub sharpe_ratio: Option<Decimal>,
    pub sortino_ratio: Option<Decimal>,
    pub calmar_ratio: Option<Decimal>,
    pub win_rate: Decimal,
    pub profit_factor: Option<Decimal>,
    pub total_trades: usize,
    pub avg_trade_return_pct: Decimal,
    pub avg_win_pct: Decimal,
    pub avg_loss_pct: Decimal,
    pub max_consecutive_losses: usize,
    pub exposure_pct: Decimal,
    pub total_commission: Decimal,
}
