//! Live trading orchestrator for OpenTrade.
//!
//! Connects all components: market data, features, strategy, risk,
//! portfolio, execution. Runs the main trading loop.

use chrono::Utc;
use ot_common::OtError;
use ot_config::AppConfig;
use ot_execution::{ExchangeAdapter, OrderManager};
use ot_features::pipeline::compute_features;
use ot_market_data::candle_buffer::CandleBuffer;
use ot_portfolio::PortfolioManager;
use ot_risk::RiskEngine;
use ot_strategy::Strategy;
use ot_types::market::Candle;
use ot_types::orders::*;
use ot_types::positions::*;
use ot_types::risk::RiskVerdict;
use ot_types::signals::*;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

/// The core trading engine.
pub struct TradingEngine {
    config: AppConfig,
    strategies: Vec<Box<dyn Strategy>>,
    risk_engine: RiskEngine,
    portfolio: PortfolioManager,
    order_manager: OrderManager,
    candle_buffers: HashMap<String, CandleBuffer>,
    exchange: Arc<dyn ExchangeAdapter>,
}

impl TradingEngine {
    pub fn new(
        config: AppConfig,
        strategies: Vec<Box<dyn Strategy>>,
        exchange: Arc<dyn ExchangeAdapter>,
    ) -> Self {
        let risk_engine =
            RiskEngine::new(config.risk.clone(), config.portfolio.initial_capital);
        let portfolio = PortfolioManager::new(config.portfolio.clone());
        let order_manager = OrderManager::new(10000);

        Self {
            config,
            strategies,
            risk_engine,
            portfolio,
            order_manager,
            candle_buffers: HashMap::new(),
            exchange,
        }
    }

    /// Process a new completed candle.
    pub async fn on_candle(&mut self, candle: Candle) -> Result<(), OtError> {
        let key = format!("{}:{}", candle.symbol, candle.timeframe);

        // Buffer the candle
        let buffer = self
            .candle_buffers
            .entry(key.clone())
            .or_insert_with(|| CandleBuffer::new(500));
        buffer.push(candle.clone());

        // Need enough data for features
        if buffer.len() < 52 {
            return Ok(());
        }

        // Compute features
        let candles: Vec<Candle> = buffer.iter().cloned().collect();
        let features = match compute_features(&candles) {
            Some(f) => f,
            None => return Ok(()),
        };

        // Collect signals first (avoid borrowing self mutably twice)
        let mut signals = Vec::new();
        for strategy in &mut self.strategies {
            let position = self
                .portfolio
                .get_position(&candle.symbol, strategy.name())
                .cloned();

            if let Some(signal) = strategy.on_bar(&candle, &features, position.as_ref()) {
                signals.push(signal);
            }
        }

        // Process collected signals
        for signal in signals {
            self.process_signal(signal, &candle).await?;
        }

        Ok(())
    }

    async fn process_signal(&mut self, signal: Signal, candle: &Candle) -> Result<(), OtError> {
        info!(
            strategy = %signal.strategy_name,
            symbol = %signal.symbol,
            direction = ?signal.direction,
            confidence = %signal.confidence,
            "Signal generated"
        );

        // Determine position size
        let size = self.portfolio.compute_position_size(
            &signal,
            candle.close,
            None, // ATR would come from features
        );

        if size <= dec!(0) {
            info!("Position size is zero, skipping");
            return Ok(());
        }

        let side = match signal.direction {
            SignalDirection::Long => Side::Buy,
            SignalDirection::Short => Side::Sell,
            SignalDirection::Flat | SignalDirection::ReduceLong => {
                // Close position
                let pos = self
                    .portfolio
                    .get_position(&signal.symbol, &signal.strategy_name);
                if let Some(p) = pos {
                    if !p.is_flat() {
                        match p.side {
                            PositionSide::Long => Side::Sell,
                            PositionSide::Short => Side::Buy,
                            PositionSide::Flat => return Ok(()),
                        }
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
            SignalDirection::ReduceShort => Side::Buy,
        };

        // Create order request
        let order_request = OrderRequest {
            client_order_id: ClientOrderId::generate(),
            symbol: signal.symbol.clone(),
            market_type: signal.market_type,
            side,
            order_type: OrderType::Market,
            quantity: size,
            price: Some(candle.close),
            stop_price: None,
            time_in_force: None,
            reduce_only: signal.direction == SignalDirection::Flat
                || signal.direction == SignalDirection::ReduceLong
                || signal.direction == SignalDirection::ReduceShort,
            strategy_name: signal.strategy_name.clone(),
            reason: OrderReason {
                strategy: signal.strategy_name.clone(),
                signal_type: format!("{:?}", signal.direction),
                confidence: signal.confidence,
                description: format!(
                    "{}",
                    signal.metadata.signal_inputs
                ),
            },
            created_at: Utc::now(),
        };

        // Risk check
        let positions = self.portfolio.positions_vec();
        let verdict = self.risk_engine.check_order(&signal, &order_request, &positions);

        match verdict {
            RiskVerdict::Approved => {
                info!(id = %order_request.client_order_id, "Order approved by risk engine");
            }
            RiskVerdict::Rejected { reason } => {
                warn!(reason = %reason, "Order rejected by risk engine");
                return Ok(());
            }
            RiskVerdict::ReducedSize { new_quantity, reason } => {
                warn!(reason = %reason, new_qty = %new_quantity, "Order size reduced by risk engine");
                // Would create new order with reduced size here
                return Ok(());
            }
        }

        // Submit to exchange
        match self.exchange.submit_order(&order_request).await {
            Ok(tracked) => {
                info!(
                    id = %tracked.client_order_id,
                    status = ?tracked.status,
                    "Order submitted"
                );
                self.order_manager.track_order(tracked.clone());

                // Update position
                if tracked.status == OrderStatus::Filled {
                    if let Some(fill_price) = tracked.average_fill_price {
                        let pos_side = match side {
                            Side::Buy => PositionSide::Long,
                            Side::Sell => PositionSide::Short,
                        };
                        let position = Position {
                            symbol: signal.symbol.clone(),
                            market_type: signal.market_type,
                            side: pos_side,
                            quantity: tracked.filled_quantity,
                            entry_price: fill_price,
                            current_price: fill_price,
                            unrealized_pnl: dec!(0),
                            realized_pnl: dec!(0),
                            total_commission: tracked.commission,
                            leverage: dec!(1),
                            liquidation_price: None,
                            opened_at: Utc::now(),
                            last_update: Utc::now(),
                            strategy_name: signal.strategy_name.clone(),
                        };
                        self.portfolio.update_position(position);
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to submit order");
                self.risk_engine.record_order_rejection();
            }
        }

        Ok(())
    }

    /// Emergency: flatten all positions.
    pub async fn flatten_all(&mut self) -> Result<(), OtError> {
        warn!("EMERGENCY: Flattening all positions");
        let positions = self.portfolio.positions_vec();
        for pos in &positions {
            if pos.is_flat() {
                continue;
            }
            let side = match pos.side {
                PositionSide::Long => Side::Sell,
                PositionSide::Short => Side::Buy,
                PositionSide::Flat => continue,
            };
            let request = OrderRequest {
                client_order_id: ClientOrderId::generate(),
                symbol: pos.symbol.clone(),
                market_type: pos.market_type,
                side,
                order_type: OrderType::Market,
                quantity: pos.quantity,
                price: None,
                stop_price: None,
                time_in_force: None,
                reduce_only: true,
                strategy_name: "emergency_flatten".into(),
                reason: OrderReason {
                    strategy: "emergency".into(),
                    signal_type: "flatten".into(),
                    confidence: dec!(1),
                    description: "Emergency flatten all positions".into(),
                },
                created_at: Utc::now(),
            };

            match self.exchange.submit_order(&request).await {
                Ok(_) => info!(symbol = %pos.symbol, "Position flattened"),
                Err(e) => error!(symbol = %pos.symbol, error = %e, "Failed to flatten"),
            }
        }
        Ok(())
    }
}
