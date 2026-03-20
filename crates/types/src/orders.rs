use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::market::{MarketType, Symbol};

/// Side of a trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }

    pub fn sign(self) -> Decimal {
        match self {
            Self::Buy => Decimal::ONE,
            Self::Sell => -Decimal::ONE,
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buy => write!(f, "BUY"),
            Self::Sell => write!(f, "SELL"),
        }
    }
}

/// Order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
    StopLossLimit,
    TakeProfit,
    TakeProfitLimit,
    PostOnly,
    ReduceOnly,
}

/// Time in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TimeInForce {
    Gtc,
    Ioc,
    Fok,
}

/// Internal order ID (client-generated).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientOrderId(pub String);

impl ClientOrderId {
    pub fn generate() -> Self {
        Self(format!("ot_{}", Uuid::new_v4().simple()))
    }
}

impl std::fmt::Display for ClientOrderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Exchange-assigned order ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExchangeOrderId(pub String);

/// An order request to be sent to the exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRequest {
    pub client_order_id: ClientOrderId,
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub side: Side,
    pub order_type: OrderType,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub stop_price: Option<Decimal>,
    pub time_in_force: Option<TimeInForce>,
    pub reduce_only: bool,
    pub strategy_name: String,
    pub reason: OrderReason,
    pub created_at: DateTime<Utc>,
}

/// Why this order was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderReason {
    pub strategy: String,
    pub signal_type: String,
    pub confidence: Decimal,
    pub description: String,
}

/// Current state of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Pending,
    Submitted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
}

impl OrderStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Cancelled | Self::Rejected | Self::Expired
        )
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Submitted | Self::PartiallyFilled)
    }
}

/// Tracked order with full lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedOrder {
    pub client_order_id: ClientOrderId,
    pub exchange_order_id: Option<ExchangeOrderId>,
    pub request: OrderRequest,
    pub status: OrderStatus,
    pub filled_quantity: Decimal,
    pub average_fill_price: Option<Decimal>,
    pub commission: Decimal,
    pub commission_asset: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub last_update: DateTime<Utc>,
    pub fill_events: Vec<FillEvent>,
}

/// A single fill event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillEvent {
    pub timestamp: DateTime<Utc>,
    pub price: Decimal,
    pub quantity: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_opposite() {
        assert_eq!(Side::Buy.opposite(), Side::Sell);
        assert_eq!(Side::Sell.opposite(), Side::Buy);
    }

    #[test]
    fn order_status_terminal() {
        assert!(OrderStatus::Filled.is_terminal());
        assert!(!OrderStatus::Submitted.is_terminal());
        assert!(OrderStatus::Submitted.is_active());
    }

    #[test]
    fn client_order_id_format() {
        let id = ClientOrderId::generate();
        assert!(id.0.starts_with("ot_"));
    }
}
