//! Execution engine for OpenTrade.
//!
//! Manages order lifecycle: creation, submission, tracking, fill reconciliation.
//! Supports market, limit, stop, and take-profit orders.
//! Provides the exchange adapter trait.

use async_trait::async_trait;
use chrono::Utc;
use ot_types::market::Symbol;
use ot_types::orders::*;
use rust_decimal::Decimal;
use std::collections::HashMap;
use tracing::{debug, info};

pub use async_trait::async_trait as execution_async_trait;

/// Trait for exchange execution adapters.
/// Both live exchange and paper trading implement this.
#[async_trait]
pub trait ExchangeAdapter: Send + Sync {
    async fn submit_order(
        &self,
        request: &OrderRequest,
    ) -> Result<TrackedOrder, ot_common::OtError>;

    async fn cancel_order(
        &self,
        symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<(), ot_common::OtError>;

    async fn cancel_all_orders(&self, symbol: &Symbol) -> Result<(), ot_common::OtError>;

    async fn get_order_status(
        &self,
        symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<TrackedOrder, ot_common::OtError>;

    async fn get_balance(&self, asset: &str) -> Result<Decimal, ot_common::OtError>;
}

/// Order Management System tracking all orders.
pub struct OrderManager {
    active_orders: HashMap<ClientOrderId, TrackedOrder>,
    completed_orders: Vec<TrackedOrder>,
    max_completed_history: usize,
}

impl OrderManager {
    pub fn new(max_history: usize) -> Self {
        Self {
            active_orders: HashMap::new(),
            completed_orders: Vec::new(),
            max_completed_history: max_history,
        }
    }

    /// Track a newly submitted order.
    pub fn track_order(&mut self, order: TrackedOrder) {
        info!(
            id = %order.client_order_id,
            symbol = %order.request.symbol,
            side = %order.request.side,
            qty = %order.request.quantity,
            "Order tracked"
        );
        self.active_orders
            .insert(order.client_order_id.clone(), order);
    }

    /// Update order status from exchange.
    pub fn update_order(&mut self, order: TrackedOrder) {
        if order.status.is_terminal() {
            debug!(id = %order.client_order_id, status = ?order.status, "Order completed");
            self.active_orders.remove(&order.client_order_id);
            self.completed_orders.push(order);
            if self.completed_orders.len() > self.max_completed_history {
                self.completed_orders.remove(0);
            }
        } else {
            self.active_orders
                .insert(order.client_order_id.clone(), order);
        }
    }

    /// Get all active (non-terminal) orders.
    pub fn active_orders(&self) -> Vec<&TrackedOrder> {
        self.active_orders.values().collect()
    }

    /// Get active orders for a specific symbol.
    pub fn active_orders_for_symbol(&self, symbol: &Symbol) -> Vec<&TrackedOrder> {
        self.active_orders
            .values()
            .filter(|o| &o.request.symbol == symbol)
            .collect()
    }

    /// Check if there's an active order for a symbol+strategy.
    pub fn has_active_order(&self, symbol: &Symbol, strategy: &str) -> bool {
        self.active_orders
            .values()
            .any(|o| &o.request.symbol == symbol && o.request.strategy_name == strategy)
    }

    pub fn active_count(&self) -> usize {
        self.active_orders.len()
    }

    pub fn completed_count(&self) -> usize {
        self.completed_orders.len()
    }

    /// Recent completed orders for reporting.
    pub fn recent_completed(&self, n: usize) -> Vec<&TrackedOrder> {
        let start = self.completed_orders.len().saturating_sub(n);
        self.completed_orders[start..].iter().collect()
    }
}

/// Create a TrackedOrder from a request with initial state.
pub fn create_tracked_order(request: OrderRequest) -> TrackedOrder {
    let now = Utc::now();
    TrackedOrder {
        client_order_id: request.client_order_id.clone(),
        exchange_order_id: None,
        request,
        status: OrderStatus::Pending,
        filled_quantity: Decimal::ZERO,
        average_fill_price: None,
        commission: Decimal::ZERO,
        commission_asset: None,
        submitted_at: Some(now),
        last_update: now,
        fill_events: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_types::market::MarketType;
    use rust_decimal_macros::dec;

    fn test_order() -> TrackedOrder {
        let request = OrderRequest {
            client_order_id: ClientOrderId::generate(),
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            side: Side::Buy,
            order_type: OrderType::Market,
            quantity: dec!(0.1),
            price: Some(dec!(50000)),
            stop_price: None,
            time_in_force: None,
            reduce_only: false,
            strategy_name: "test".into(),
            reason: OrderReason {
                strategy: "test".into(),
                signal_type: "long".into(),
                confidence: dec!(0.7),
                description: "test".into(),
            },
            created_at: Utc::now(),
        };
        create_tracked_order(request)
    }

    #[test]
    fn track_and_complete_order() {
        let mut mgr = OrderManager::new(100);
        let mut order = test_order();
        let id = order.client_order_id.clone();

        mgr.track_order(order.clone());
        assert_eq!(mgr.active_count(), 1);

        order.status = OrderStatus::Filled;
        mgr.update_order(order);
        assert_eq!(mgr.active_count(), 0);
        assert_eq!(mgr.completed_count(), 1);
    }

    #[test]
    fn active_orders_by_symbol() {
        let mut mgr = OrderManager::new(100);
        mgr.track_order(test_order());
        let btc = Symbol::new("BTCUSDT");
        let eth = Symbol::new("ETHUSDT");
        assert_eq!(mgr.active_orders_for_symbol(&btc).len(), 1);
        assert_eq!(mgr.active_orders_for_symbol(&eth).len(), 0);
    }
}
