//! Paper trading adapter for OpenTrade.
//!
//! Implements the ExchangeAdapter trait using simulated fills.
//! Uses the same signal and risk path as live trading.

use async_trait::async_trait;
use chrono::Utc;
use ot_common::OtError;
use ot_execution::ExchangeAdapter;
use ot_types::market::Symbol;
use ot_types::orders::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::info;

/// Paper trading exchange adapter.
pub struct PaperExchange {
    balances: Mutex<HashMap<String, Decimal>>,
    orders: Mutex<HashMap<ClientOrderId, TrackedOrder>>,
    slippage_bps: Decimal,
    fee_bps: Decimal,
    prices: Mutex<HashMap<String, Decimal>>,
}

impl PaperExchange {
    pub fn new(initial_balance_usdt: Decimal, slippage_bps: Decimal, fee_bps: Decimal) -> Self {
        let mut balances = HashMap::new();
        balances.insert("USDT".to_string(), initial_balance_usdt);
        Self {
            balances: Mutex::new(balances),
            orders: Mutex::new(HashMap::new()),
            slippage_bps,
            fee_bps,
            prices: Mutex::new(HashMap::new()),
        }
    }

    /// Set current market price for a symbol (for paper fills).
    pub fn set_price(&self, symbol: &str, price: Decimal) {
        self.prices
            .lock()
            .unwrap()
            .insert(symbol.to_string(), price);
    }

    fn get_price(&self, symbol: &str) -> Option<Decimal> {
        self.prices.lock().unwrap().get(symbol).copied()
    }

    fn apply_slippage(&self, price: Decimal, side: Side) -> Decimal {
        let slip = price * self.slippage_bps / dec!(10000);
        match side {
            Side::Buy => price + slip,
            Side::Sell => price - slip,
        }
    }

    fn compute_fee(&self, notional: Decimal) -> Decimal {
        notional * self.fee_bps / dec!(10000)
    }
}

#[async_trait]
impl ExchangeAdapter for PaperExchange {
    async fn submit_order(&self, request: &OrderRequest) -> Result<TrackedOrder, OtError> {
        let fill_price = match request.price {
            Some(p) => p,
            None => self
                .get_price(request.symbol.as_str())
                .unwrap_or(dec!(0)),
        };

        let fill_price = self.apply_slippage(fill_price, request.side);
        let notional = request.quantity * fill_price;
        let commission = self.compute_fee(notional);

        info!(
            symbol = %request.symbol,
            side = %request.side,
            qty = %request.quantity,
            price = %fill_price,
            commission = %commission,
            "[PAPER] Order filled"
        );

        let fill = FillEvent {
            timestamp: Utc::now(),
            price: fill_price,
            quantity: request.quantity,
            commission,
            commission_asset: "USDT".to_string(),
        };

        let order = TrackedOrder {
            client_order_id: request.client_order_id.clone(),
            exchange_order_id: Some(ExchangeOrderId(format!(
                "paper_{}",
                uuid::Uuid::new_v4().simple()
            ))),
            request: request.clone(),
            status: OrderStatus::Filled,
            filled_quantity: request.quantity,
            average_fill_price: Some(fill_price),
            commission,
            commission_asset: Some("USDT".to_string()),
            submitted_at: Some(Utc::now()),
            last_update: Utc::now(),
            fill_events: vec![fill],
        };

        self.orders
            .lock()
            .unwrap()
            .insert(request.client_order_id.clone(), order.clone());

        Ok(order)
    }

    async fn cancel_order(
        &self,
        _symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<(), OtError> {
        if let Some(order) = self.orders.lock().unwrap().get_mut(client_order_id) {
            order.status = OrderStatus::Cancelled;
            order.last_update = Utc::now();
        }
        Ok(())
    }

    async fn cancel_all_orders(&self, symbol: &Symbol) -> Result<(), OtError> {
        let mut orders = self.orders.lock().unwrap();
        for order in orders.values_mut() {
            if &order.request.symbol == symbol && order.status.is_active() {
                order.status = OrderStatus::Cancelled;
                order.last_update = Utc::now();
            }
        }
        Ok(())
    }

    async fn get_order_status(
        &self,
        _symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<TrackedOrder, OtError> {
        self.orders
            .lock()
            .unwrap()
            .get(client_order_id)
            .cloned()
            .ok_or_else(|| OtError::Execution("Order not found".into()))
    }

    async fn get_balance(&self, asset: &str) -> Result<Decimal, OtError> {
        Ok(self
            .balances
            .lock()
            .unwrap()
            .get(asset)
            .copied()
            .unwrap_or(dec!(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_types::market::MarketType;

    fn test_order_request() -> OrderRequest {
        OrderRequest {
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
        }
    }

    #[tokio::test]
    async fn paper_order_fills_immediately() {
        let exchange = PaperExchange::new(dec!(100000), dec!(5), dec!(10));
        exchange.set_price("BTCUSDT", dec!(50000));

        let req = test_order_request();
        let result = exchange.submit_order(&req).await;
        assert!(result.is_ok());

        let order = result.unwrap();
        assert_eq!(order.status, OrderStatus::Filled);
        assert_eq!(order.filled_quantity, dec!(0.1));
        assert!(order.average_fill_price.unwrap() > dec!(50000)); // slippage
    }

    #[tokio::test]
    async fn paper_balance() {
        let exchange = PaperExchange::new(dec!(100000), dec!(5), dec!(10));
        let balance = exchange.get_balance("USDT").await.unwrap();
        assert_eq!(balance, dec!(100000));
    }
}
