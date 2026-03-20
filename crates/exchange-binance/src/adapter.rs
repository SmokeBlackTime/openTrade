//! Live exchange adapter implementing ExchangeAdapter for Binance.
//!
//! Converts between internal order types and Binance REST API calls.

use async_trait::async_trait;
use chrono::Utc;
use ot_common::OtError;
use ot_execution::ExchangeAdapter;
use ot_types::market::Symbol;
use ot_types::orders::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, info};

use crate::client::BinanceClient;

/// Live Binance exchange adapter.
pub struct BinanceExchangeAdapter {
    client: BinanceClient,
}

impl BinanceExchangeAdapter {
    pub fn new(client: BinanceClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ExchangeAdapter for BinanceExchangeAdapter {
    async fn submit_order(&self, request: &OrderRequest) -> Result<TrackedOrder, OtError> {
        let start = std::time::Instant::now();

        let response = self.client.place_order(request).await?;

        let latency_ms = start.elapsed().as_millis();
        debug!(latency_ms = latency_ms, "Order submission latency");

        // Parse fills
        let mut total_qty = Decimal::ZERO;
        let mut total_quote = Decimal::ZERO;
        let mut total_commission = Decimal::ZERO;
        let mut commission_asset = None;
        let mut fill_events = Vec::new();

        for fill in &response.fills {
            let price = Decimal::from_str(&fill.price).unwrap_or(Decimal::ZERO);
            let qty = Decimal::from_str(&fill.qty).unwrap_or(Decimal::ZERO);
            let comm = Decimal::from_str(&fill.commission).unwrap_or(Decimal::ZERO);

            total_qty += qty;
            total_quote += price * qty;
            total_commission += comm;
            commission_asset = Some(fill.commission_asset.clone());

            fill_events.push(FillEvent {
                timestamp: Utc::now(),
                price,
                quantity: qty,
                commission: comm,
                commission_asset: fill.commission_asset.clone(),
            });
        }

        let avg_price = if total_qty > Decimal::ZERO {
            Some(total_quote / total_qty)
        } else {
            // For market orders that report executed_qty but no fills array
            let exec_qty = Decimal::from_str(&response.executed_qty).unwrap_or(Decimal::ZERO);
            let cum_quote = Decimal::from_str(&response.cummulative_quote_qty).unwrap_or(Decimal::ZERO);
            if exec_qty > Decimal::ZERO {
                total_qty = exec_qty;
                Some(cum_quote / exec_qty)
            } else {
                None
            }
        };

        let status = match response.status.as_str() {
            "NEW" => OrderStatus::Submitted,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            _ => OrderStatus::Submitted,
        };

        let tracked = TrackedOrder {
            client_order_id: request.client_order_id.clone(),
            exchange_order_id: Some(ExchangeOrderId(response.order_id.to_string())),
            request: request.clone(),
            status,
            filled_quantity: total_qty,
            average_fill_price: avg_price,
            commission: total_commission,
            commission_asset,
            submitted_at: Some(Utc::now()),
            last_update: Utc::now(),
            fill_events,
        };

        info!(
            id = %tracked.client_order_id,
            exchange_id = response.order_id,
            status = ?tracked.status,
            filled = %tracked.filled_quantity,
            "Order response from Binance"
        );

        Ok(tracked)
    }

    async fn cancel_order(
        &self,
        symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<(), OtError> {
        self.client
            .cancel_order(symbol.as_str(), &client_order_id.0)
            .await?;
        Ok(())
    }

    async fn cancel_all_orders(&self, symbol: &Symbol) -> Result<(), OtError> {
        self.client.cancel_all_orders(symbol.as_str()).await
    }

    async fn get_order_status(
        &self,
        symbol: &Symbol,
        client_order_id: &ClientOrderId,
    ) -> Result<TrackedOrder, OtError> {
        let response = self
            .client
            .query_order(symbol.as_str(), &client_order_id.0)
            .await?;

        let status = match response.status.as_str() {
            "NEW" => OrderStatus::Submitted,
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
            "REJECTED" => OrderStatus::Rejected,
            "EXPIRED" => OrderStatus::Expired,
            _ => OrderStatus::Submitted,
        };

        let exec_qty = Decimal::from_str(&response.executed_qty).unwrap_or(Decimal::ZERO);
        let cum_quote = Decimal::from_str(&response.cummulative_quote_qty).unwrap_or(Decimal::ZERO);
        let avg_price = if exec_qty > Decimal::ZERO {
            Some(cum_quote / exec_qty)
        } else {
            None
        };

        // Reconstruct a minimal OrderRequest for the tracked order
        let side = match response.side.as_str() {
            "BUY" => Side::Buy,
            _ => Side::Sell,
        };
        let order_type = match response.r#type.as_str() {
            "MARKET" => OrderType::Market,
            "LIMIT" => OrderType::Limit,
            "STOP_LOSS" => OrderType::StopLoss,
            "STOP_LOSS_LIMIT" => OrderType::StopLossLimit,
            "TAKE_PROFIT" => OrderType::TakeProfit,
            "TAKE_PROFIT_LIMIT" => OrderType::TakeProfitLimit,
            _ => OrderType::Market,
        };

        let orig_qty = Decimal::from_str(&response.orig_qty).unwrap_or(Decimal::ZERO);
        let price = Decimal::from_str(&response.price).ok();

        let request = OrderRequest {
            client_order_id: client_order_id.clone(),
            symbol: symbol.clone(),
            market_type: ot_types::market::MarketType::Spot,
            side,
            order_type,
            quantity: orig_qty,
            price,
            stop_price: None,
            time_in_force: None,
            reduce_only: false,
            strategy_name: "unknown".into(),
            reason: OrderReason {
                strategy: "query".into(),
                signal_type: "query".into(),
                confidence: Decimal::ZERO,
                description: "Queried from exchange".into(),
            },
            created_at: Utc::now(),
        };

        Ok(TrackedOrder {
            client_order_id: client_order_id.clone(),
            exchange_order_id: Some(ExchangeOrderId(response.order_id.to_string())),
            request,
            status,
            filled_quantity: exec_qty,
            average_fill_price: avg_price,
            commission: Decimal::ZERO, // Not available from query endpoint
            commission_asset: None,
            submitted_at: response.transact_time.map(|t| {
                ot_common::time_utils::ms_to_datetime(t)
            }),
            last_update: Utc::now(),
            fill_events: Vec::new(),
        })
    }

    async fn get_balance(&self, asset: &str) -> Result<Decimal, OtError> {
        self.client.get_account_balance(asset).await
    }
}
