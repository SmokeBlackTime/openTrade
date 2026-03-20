use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::market::{Candle, FundingRate, MarketTrade, OrderBookSnapshot, TopOfBook};
use crate::orders::{FillEvent, TrackedOrder};
use crate::signals::{Signal, TradeDecision};

/// Unified event type for the event-driven architecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradingEvent {
    // Market data events
    CandleClosed(Candle),
    TradeReceived(MarketTrade),
    TopOfBookUpdate(TopOfBook),
    OrderBookUpdate(OrderBookSnapshot),
    FundingRateUpdate(FundingRate),

    // Signal events
    SignalGenerated(Signal),
    TradeDecisionMade(TradeDecision),

    // Order events
    OrderSubmitted(TrackedOrder),
    OrderFilled(TrackedOrder),
    OrderPartiallyFilled(TrackedOrder),
    OrderCancelled(TrackedOrder),
    OrderRejected(TrackedOrder),

    // Fill events
    FillReceived {
        order: TrackedOrder,
        fill: FillEvent,
    },

    // System events
    KillSwitchTriggered {
        switch_type: String,
        reason: String,
        timestamp: DateTime<Utc>,
    },
    HealthCheck {
        component: String,
        healthy: bool,
        timestamp: DateTime<Utc>,
    },
    SystemShutdown {
        reason: String,
        timestamp: DateTime<Utc>,
    },
}

impl TradingEvent {
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::CandleClosed(c) => c.close_time,
            Self::TradeReceived(t) => t.timestamp,
            Self::TopOfBookUpdate(t) => t.timestamp,
            Self::OrderBookUpdate(o) => o.timestamp,
            Self::FundingRateUpdate(f) => f.timestamp,
            Self::SignalGenerated(s) => s.timestamp,
            Self::TradeDecisionMade(d) => d.decision_timestamp,
            Self::OrderSubmitted(o)
            | Self::OrderFilled(o)
            | Self::OrderPartiallyFilled(o)
            | Self::OrderCancelled(o)
            | Self::OrderRejected(o) => o.last_update,
            Self::FillReceived { fill, .. } => fill.timestamp,
            Self::KillSwitchTriggered { timestamp, .. } => *timestamp,
            Self::HealthCheck { timestamp, .. } => *timestamp,
            Self::SystemShutdown { timestamp, .. } => *timestamp,
        }
    }

    pub fn event_type(&self) -> &'static str {
        match self {
            Self::CandleClosed(_) => "candle_closed",
            Self::TradeReceived(_) => "trade_received",
            Self::TopOfBookUpdate(_) => "top_of_book_update",
            Self::OrderBookUpdate(_) => "order_book_update",
            Self::FundingRateUpdate(_) => "funding_rate_update",
            Self::SignalGenerated(_) => "signal_generated",
            Self::TradeDecisionMade(_) => "trade_decision_made",
            Self::OrderSubmitted(_) => "order_submitted",
            Self::OrderFilled(_) => "order_filled",
            Self::OrderPartiallyFilled(_) => "order_partially_filled",
            Self::OrderCancelled(_) => "order_cancelled",
            Self::OrderRejected(_) => "order_rejected",
            Self::FillReceived { .. } => "fill_received",
            Self::KillSwitchTriggered { .. } => "kill_switch_triggered",
            Self::HealthCheck { .. } => "health_check",
            Self::SystemShutdown { .. } => "system_shutdown",
        }
    }
}
