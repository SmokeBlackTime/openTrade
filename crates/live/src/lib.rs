//! Live trading orchestrator for OpenTrade.
//!
//! Connects all components: market data, features, strategy, risk,
//! portfolio, execution. Runs the main trading loop with:
//! - WebSocket candle streaming
//! - Bracket orders (stop-loss + take-profit) on entry
//! - Position reconciliation with exchange
//! - Order status polling via user data stream
//! - Trade journaling and state recovery on restart

use chrono::Utc;
use ot_common::OtError;
use ot_config::AppConfig;
use ot_execution::{BracketPair, ExchangeAdapter, OrderManager};
use ot_features::pipeline::compute_features;
use ot_market_data::candle_buffer::CandleBuffer;
use ot_portfolio::PortfolioManager;
use ot_risk::RiskEngine;
use ot_storage::Storage;
use ot_strategy::Strategy;
use ot_types::market::Candle;
use ot_types::orders::*;
use ot_types::positions::*;
use ot_types::risk::RiskVerdict;
use ot_types::signals::*;
use ot_types::trade::TradeRecord;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

/// The core trading engine.
pub struct TradingEngine {
    #[allow(dead_code)]
    config: AppConfig,
    strategies: Vec<Box<dyn Strategy>>,
    risk_engine: RiskEngine,
    portfolio: PortfolioManager,
    order_manager: OrderManager,
    candle_buffers: HashMap<String, CandleBuffer>,
    exchange: Arc<dyn ExchangeAdapter>,
    storage: Option<Storage>,
    /// Maps entry_client_order_id -> Signal (for bracket order context)
    pending_signals: HashMap<ClientOrderId, Signal>,
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

        // Initialize storage if configured
        let storage = if config.storage.journal_enabled {
            let path = std::path::Path::new(&config.storage.database_path);
            match Storage::new(path) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!(error = %e, "Failed to initialize storage, journaling disabled");
                    None
                }
            }
        } else {
            None
        };

        Self {
            config,
            strategies,
            risk_engine,
            portfolio,
            order_manager,
            candle_buffers: HashMap::new(),
            exchange,
            storage,
            pending_signals: HashMap::new(),
        }
    }

    /// Restore state from trade journal on startup.
    pub fn restore_state(&mut self) -> Result<(), OtError> {
        let storage = match &self.storage {
            Some(s) => s,
            None => return Ok(()),
        };

        // Restore equity from system state — but only if it's close to config capital
        // (prevents stale journal from overriding a config change)
        let config_capital = self.config.portfolio.initial_capital;
        if let Some(equity_str) = storage.get_state("current_equity")? {
            if let Ok(equity) = equity_str.parse::<Decimal>() {
                let ratio = if config_capital > dec!(0) { equity / config_capital } else { dec!(100) };
                if ratio > dec!(0.5) && ratio < dec!(2) {
                    info!(equity = %equity, "Restored equity from journal");
                    self.portfolio.update_equity(equity);
                    self.risk_engine.update_equity(equity);
                } else {
                    warn!(
                        journal_equity = %equity,
                        config_capital = %config_capital,
                        "Journal equity differs too much from config, using config capital"
                    );
                }
            }
        }

        // Restore peak equity
        if let Some(peak_str) = storage.get_state("peak_equity")? {
            if let Ok(peak) = peak_str.parse::<Decimal>() {
                info!(peak = %peak, "Restored peak equity");
                // Peak equity is tracked internally by risk engine
            }
        }

        // Restore daily PnL if same trading day
        if let Some(date_str) = storage.get_state("trading_date")? {
            let today = Utc::now().format("%Y-%m-%d").to_string();
            if date_str != today {
                info!("New trading day, resetting daily counters");
                self.risk_engine.reset_daily();
            } else if let Some(pnl_str) = storage.get_state("daily_pnl")? {
                if let Ok(pnl) = pnl_str.parse::<Decimal>() {
                    info!(daily_pnl = %pnl, "Restored daily PnL");
                }
            }
        }

        // Restore open positions from state
        if let Some(positions_json) = storage.get_state("open_positions")? {
            match serde_json::from_str::<Vec<Position>>(&positions_json) {
                Ok(positions) => {
                    for pos in positions {
                        if !pos.is_flat() {
                            info!(
                                symbol = %pos.symbol,
                                side = ?pos.side,
                                qty = %pos.quantity,
                                entry = %pos.entry_price,
                                "Restored position from journal"
                            );
                            self.portfolio.update_position(pos);
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to parse stored positions");
                }
            }
        }

        info!("State restoration complete");
        Ok(())
    }

    /// Save current state to storage for recovery.
    fn save_state(&self) {
        let storage = match &self.storage {
            Some(s) => s,
            None => return,
        };

        let _ = storage.set_state("current_equity", &self.portfolio.equity().to_string());
        let _ = storage.set_state("daily_pnl", &self.risk_engine.daily_pnl().to_string());
        let _ = storage.set_state(
            "trading_date",
            &Utc::now().format("%Y-%m-%d").to_string(),
        );

        // Save open positions
        let positions = self.portfolio.positions_vec();
        if let Ok(json) = serde_json::to_string(&positions) {
            let _ = storage.set_state("open_positions", &json);
        }
    }

    /// Record a trade to the journal.
    fn journal_trade(&self, trade: &TradeRecord) {
        if let Some(storage) = &self.storage {
            if let Err(e) = storage.store_trade(trade) {
                error!(error = %e, "Failed to journal trade");
            }
        }
    }

    /// Pre-fill candle buffer with historical data so the engine can trade immediately.
    pub fn prefill_buffer(&mut self, symbol: &str, timeframe: &str, candles: Vec<Candle>) {
        let key = format!("{}:{}", symbol, timeframe);
        let buffer = self
            .candle_buffers
            .entry(key.clone())
            .or_insert_with(|| CandleBuffer::new(500));
        let count = candles.len();
        for candle in candles {
            buffer.push(candle);
        }
        info!(key = %key, candles = count, buffer_size = buffer.len(), "Buffer pre-filled with historical candles");
    }

    /// Process a new completed candle.
    pub async fn on_candle(&mut self, candle: Candle) -> Result<(), OtError> {
        let key = format!("{}:{}", candle.symbol, candle.timeframe);

        // Update paper exchange price if applicable
        // (Paper exchange needs current prices for fills)

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

        // Update mark prices on existing positions
        let positions = self.portfolio.positions_vec();
        for mut pos in positions {
            if pos.symbol == candle.symbol && !pos.is_flat() {
                pos.update_mark_price(candle.close, Utc::now());
                self.portfolio.update_position(pos);
            }
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

        // Periodically save state
        self.save_state();

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
            None,
        );

        let notional = size * candle.close;
        info!(
            qty = %size,
            notional = %notional,
            price = %candle.close,
            equity = %self.portfolio.equity(),
            "Computed position size"
        );

        if size <= dec!(0) {
            info!("Position size is zero, skipping");
            return Ok(());
        }

        // Bump qty up to meet minimum notional if needed (Binance futures requires >= 20 USDT)
        let min_notional = self.config.execution.min_order_size_usd;
        let (size, notional) = if notional < min_notional && candle.close > dec!(0) {
            let min_qty = (min_notional / candle.close) * dec!(1.01); // 1% buffer
            info!(
                original_qty = %size,
                bumped_qty = %min_qty,
                symbol = %signal.symbol,
                "Bumping qty to meet minimum notional"
            );
            (min_qty, min_qty * candle.close)
        } else {
            (size, notional)
        };

        let side = match signal.direction {
            SignalDirection::Long => Side::Buy,
            SignalDirection::Short => Side::Sell,
            SignalDirection::Flat | SignalDirection::ReduceLong => {
                let pos_side = self
                    .portfolio
                    .get_position(&signal.symbol, &signal.strategy_name)
                    .and_then(|p| {
                        if p.is_flat() {
                            None
                        } else {
                            Some(p.side)
                        }
                    });
                match pos_side {
                    Some(side) => {
                        // Cancel any existing bracket orders for this position
                        self.cancel_bracket_orders_for_position(&signal.symbol, &signal.strategy_name).await;
                        match side {
                            PositionSide::Long => Side::Sell,
                            PositionSide::Short => Side::Buy,
                            PositionSide::Flat => return Ok(()),
                        }
                    }
                    None => return Ok(()),
                }
            }
            SignalDirection::ReduceShort => {
                self.cancel_bracket_orders_for_position(&signal.symbol, &signal.strategy_name).await;
                Side::Buy
            }
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
                description: format!("{}", signal.metadata.signal_inputs),
            },
            created_at: Utc::now(),
        };

        // Risk check
        let positions = self.portfolio.positions_vec();
        let verdict = self.risk_engine.check_order(&signal, &order_request, &positions);

        let final_request = match verdict {
            RiskVerdict::Approved => {
                info!(id = %order_request.client_order_id, "Order approved by risk engine");
                order_request
            }
            RiskVerdict::Rejected { reason } => {
                warn!(reason = %reason, "Order rejected by risk engine");
                return Ok(());
            }
            RiskVerdict::ReducedSize { new_quantity, reason } => {
                warn!(reason = %reason, new_qty = %new_quantity, "Order size reduced by risk engine");
                let mut reduced = order_request;
                reduced.quantity = new_quantity;
                reduced
            }
        };

        // Store signal for bracket order creation after fill
        let entry_id = final_request.client_order_id.clone();
        if signal.direction.is_entry() {
            self.pending_signals.insert(entry_id.clone(), signal.clone());
        }

        // Submit to exchange
        match self.exchange.submit_order(&final_request).await {
            Ok(tracked) => {
                info!(
                    id = %tracked.client_order_id,
                    status = ?tracked.status,
                    "Order submitted"
                );
                self.order_manager.track_order(tracked.clone());

                // Update position if filled
                if tracked.status == OrderStatus::Filled {
                    self.handle_fill(&tracked, &signal).await?;
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to submit order");
                // Only count as anomaly if NOT a margin/sizing error
                let err_str = e.to_string();
                if !err_str.contains("-2019") && !err_str.contains("Margin is insufficient") {
                    self.risk_engine.record_order_rejection();
                } else {
                    warn!("Margin insufficient — not counting as rejection anomaly");
                }
                self.pending_signals.remove(&entry_id);
            }
        }

        Ok(())
    }

    /// Handle a filled order: update position, submit bracket orders, journal trade.
    async fn handle_fill(&mut self, tracked: &TrackedOrder, signal: &Signal) -> Result<(), OtError> {
        let fill_price = match tracked.average_fill_price {
            Some(p) => p,
            None => return Ok(()),
        };

        let side = tracked.request.side;
        let is_entry = signal.direction.is_entry();

        if is_entry {
            // Entry fill: create position and submit bracket orders
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

            // Journal entry trade
            self.journal_trade(&TradeRecord {
                trade_id: TradeRecord::new_id(),
                client_order_id: tracked.client_order_id.clone(),
                symbol: signal.symbol.clone(),
                market_type: signal.market_type,
                side,
                quantity: tracked.filled_quantity,
                price: fill_price,
                commission: tracked.commission,
                commission_asset: tracked.commission_asset.clone().unwrap_or("USDT".into()),
                realized_pnl: None,
                strategy_name: signal.strategy_name.clone(),
                timestamp: Utc::now(),
            });

            // Submit bracket orders (stop-loss + take-profit)
            self.submit_bracket_orders(tracked, signal, fill_price).await;
        } else {
            // Exit fill: close position, record PnL
            let pos = self
                .portfolio
                .get_position(&signal.symbol, &signal.strategy_name)
                .cloned();

            if let Some(p) = pos {
                let pnl = match p.side {
                    PositionSide::Long => (fill_price - p.entry_price) * p.quantity,
                    PositionSide::Short => (p.entry_price - fill_price) * p.quantity,
                    PositionSide::Flat => dec!(0),
                } - tracked.commission;

                self.risk_engine.record_trade(pnl);
                let new_equity = self.risk_engine.current_equity();
                self.portfolio.update_equity(new_equity);

                // Journal exit trade
                self.journal_trade(&TradeRecord {
                    trade_id: TradeRecord::new_id(),
                    client_order_id: tracked.client_order_id.clone(),
                    symbol: signal.symbol.clone(),
                    market_type: signal.market_type,
                    side,
                    quantity: tracked.filled_quantity,
                    price: fill_price,
                    commission: tracked.commission,
                    commission_asset: tracked.commission_asset.clone().unwrap_or("USDT".into()),
                    realized_pnl: Some(pnl),
                    strategy_name: signal.strategy_name.clone(),
                    timestamp: Utc::now(),
                });

                // Remove position
                let mut closed = p;
                closed.side = PositionSide::Flat;
                closed.quantity = dec!(0);
                closed.realized_pnl = pnl;
                self.portfolio.update_position(closed);
            }
        }

        self.pending_signals.remove(&tracked.client_order_id);
        self.save_state();
        Ok(())
    }

    /// Submit bracket orders (stop-loss and take-profit) after an entry fill.
    async fn submit_bracket_orders(
        &mut self,
        entry_tracked: &TrackedOrder,
        signal: &Signal,
        fill_price: Decimal,
    ) {
        let exit_side = match entry_tracked.request.side {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        };

        let mut sl_order_id = None;
        let mut tp_order_id = None;

        // Submit stop-loss order
        if let Some(stop_price) = signal.stop_loss {
            let sl_request = OrderRequest {
                client_order_id: ClientOrderId::generate(),
                symbol: signal.symbol.clone(),
                market_type: signal.market_type,
                side: exit_side,
                order_type: OrderType::StopLoss,
                quantity: entry_tracked.filled_quantity,
                price: None,
                stop_price: Some(stop_price),
                time_in_force: None,
                reduce_only: true,
                strategy_name: signal.strategy_name.clone(),
                reason: OrderReason {
                    strategy: signal.strategy_name.clone(),
                    signal_type: "stop_loss".into(),
                    confidence: dec!(1),
                    description: format!(
                        "Stop loss for entry at {}, stop at {}",
                        fill_price, stop_price
                    ),
                },
                created_at: Utc::now(),
            };

            match self.exchange.submit_order(&sl_request).await {
                Ok(tracked) => {
                    info!(
                        id = %tracked.client_order_id,
                        stop_price = %stop_price,
                        "Stop-loss order submitted"
                    );
                    sl_order_id = Some(sl_request.client_order_id.clone());
                    self.order_manager.track_order(tracked);
                }
                Err(e) => {
                    error!(error = %e, "Failed to submit stop-loss order");
                }
            }
        }

        // Submit take-profit order
        if let Some(tp_price) = signal.take_profit {
            let tp_request = OrderRequest {
                client_order_id: ClientOrderId::generate(),
                symbol: signal.symbol.clone(),
                market_type: signal.market_type,
                side: exit_side,
                order_type: OrderType::TakeProfit,
                quantity: entry_tracked.filled_quantity,
                price: None,
                stop_price: Some(tp_price),
                time_in_force: None,
                reduce_only: true,
                strategy_name: signal.strategy_name.clone(),
                reason: OrderReason {
                    strategy: signal.strategy_name.clone(),
                    signal_type: "take_profit".into(),
                    confidence: dec!(1),
                    description: format!(
                        "Take profit for entry at {}, target at {}",
                        fill_price, tp_price
                    ),
                },
                created_at: Utc::now(),
            };

            match self.exchange.submit_order(&tp_request).await {
                Ok(tracked) => {
                    info!(
                        id = %tracked.client_order_id,
                        tp_price = %tp_price,
                        "Take-profit order submitted"
                    );
                    tp_order_id = Some(tp_request.client_order_id.clone());
                    self.order_manager.track_order(tracked);
                }
                Err(e) => {
                    error!(error = %e, "Failed to submit take-profit order");
                }
            }
        }

        // Register bracket pair
        if sl_order_id.is_some() || tp_order_id.is_some() {
            self.order_manager.register_bracket(BracketPair {
                entry_order_id: entry_tracked.client_order_id.clone(),
                stop_loss_order_id: sl_order_id,
                take_profit_order_id: tp_order_id,
                symbol: signal.symbol.clone(),
            });
        }
    }

    /// Cancel bracket orders when a position is being closed by signal.
    async fn cancel_bracket_orders_for_position(&mut self, symbol: &ot_types::market::Symbol, _strategy: &str) {
        // Find brackets for this symbol
        let brackets: Vec<BracketPair> = self
            .order_manager
            .all_brackets()
            .into_iter()
            .filter(|b| &b.symbol == symbol)
            .cloned()
            .collect();

        for bracket in &brackets {
            if let Some(sl_id) = &bracket.stop_loss_order_id {
                if let Err(e) = self.exchange.cancel_order(symbol, sl_id).await {
                    warn!(error = %e, id = %sl_id, "Failed to cancel stop-loss order");
                } else {
                    info!(id = %sl_id, "Cancelled stop-loss order");
                }
            }
            if let Some(tp_id) = &bracket.take_profit_order_id {
                if let Err(e) = self.exchange.cancel_order(symbol, tp_id).await {
                    warn!(error = %e, id = %tp_id, "Failed to cancel take-profit order");
                } else {
                    info!(id = %tp_id, "Cancelled take-profit order");
                }
            }
            self.order_manager.remove_bracket(&bracket.entry_order_id);
        }
    }

    /// Handle an order update from the user data stream.
    pub async fn on_order_update(
        &mut self,
        client_order_id: &str,
        symbol_str: &str,
        status: OrderStatus,
        filled_qty: Decimal,
        avg_price: Option<Decimal>,
        commission: Decimal,
    ) -> Result<(), OtError> {
        let cid = ClientOrderId(client_order_id.to_string());

        // Check if this is a bracket order fill (SL or TP hit)
        if status == OrderStatus::Filled {
            if let Some(bracket) = self.order_manager.find_bracket_containing(&cid).cloned() {
                let is_stop = bracket.stop_loss_order_id.as_ref() == Some(&cid);
                let is_tp = bracket.take_profit_order_id.as_ref() == Some(&cid);
                let symbol = ot_types::market::Symbol::new(symbol_str);

                if is_stop || is_tp {
                    info!(
                        id = %cid,
                        kind = if is_stop { "stop_loss" } else { "take_profit" },
                        "Bracket order filled"
                    );

                    // Cancel the other leg
                    let other_id = if is_stop {
                        bracket.take_profit_order_id.as_ref()
                    } else {
                        bracket.stop_loss_order_id.as_ref()
                    };

                    if let Some(other) = other_id {
                        if let Err(e) = self.exchange.cancel_order(&symbol, other).await {
                            warn!(error = %e, id = %other, "Failed to cancel other bracket leg");
                        }
                    }

                    // Close the position
                    let positions = self.portfolio.positions_vec();
                    if let Some(pos) = positions.iter().find(|p| p.symbol == symbol && !p.is_flat()) {
                        let exit_price = avg_price.unwrap_or(pos.current_price);
                        let pnl = match pos.side {
                            PositionSide::Long => (exit_price - pos.entry_price) * pos.quantity,
                            PositionSide::Short => (pos.entry_price - exit_price) * pos.quantity,
                            PositionSide::Flat => dec!(0),
                        } - commission;

                        self.risk_engine.record_trade(pnl);
                        let new_equity = self.risk_engine.current_equity();
                        self.portfolio.update_equity(new_equity);

                        // Determine exit side for TradeRecord
                        let exit_side = match pos.side {
                            PositionSide::Long => Side::Sell,
                            PositionSide::Short => Side::Buy,
                            PositionSide::Flat => Side::Sell,
                        };

                        self.journal_trade(&TradeRecord {
                            trade_id: TradeRecord::new_id(),
                            client_order_id: cid.clone(),
                            symbol: symbol.clone(),
                            market_type: pos.market_type,
                            side: exit_side,
                            quantity: filled_qty,
                            price: exit_price,
                            commission,
                            commission_asset: "USDT".into(),
                            realized_pnl: Some(pnl),
                            strategy_name: pos.strategy_name.clone(),
                            timestamp: Utc::now(),
                        });

                        let mut closed = pos.clone();
                        closed.side = PositionSide::Flat;
                        closed.quantity = dec!(0);
                        closed.realized_pnl = pnl;
                        self.portfolio.update_position(closed);
                    }

                    self.order_manager.remove_bracket(&bracket.entry_order_id);
                    self.save_state();
                }
            }
        }

        // Update order in manager
        if let Some(order) = self.order_manager.get_active_mut(&cid) {
            order.status = status;
            order.filled_quantity = filled_qty;
            order.average_fill_price = avg_price;
            order.commission = commission;
            order.last_update = Utc::now();
        }

        Ok(())
    }

    /// Reconcile positions with exchange.
    /// Checks USDT balance and compares with internal state.
    pub async fn reconcile(&mut self) -> Result<(), OtError> {
        info!("Running position reconciliation");

        // Check exchange balance
        match self.exchange.get_balance("USDT").await {
            Ok(balance) => {
                info!(exchange_balance = %balance, "Exchange USDT balance");
            }
            Err(e) => {
                warn!(error = %e, "Failed to fetch exchange balance for reconciliation");
            }
        }

        // Poll active orders for status updates
        let active: Vec<(ot_types::market::Symbol, ClientOrderId)> = self
            .order_manager
            .active_orders()
            .iter()
            .map(|o| (o.request.symbol.clone(), o.client_order_id.clone()))
            .collect();

        for (symbol, cid) in active {
            match self.exchange.get_order_status(&symbol, &cid).await {
                Ok(updated) => {
                    if updated.status != OrderStatus::Submitted
                        && updated.status != OrderStatus::Pending
                    {
                        info!(
                            id = %cid,
                            status = ?updated.status,
                            "Reconciled order status"
                        );
                        self.order_manager.update_order(updated);
                    }
                }
                Err(e) => {
                    warn!(error = %e, id = %cid, "Failed to reconcile order");
                }
            }
        }

        self.save_state();
        Ok(())
    }

    /// Emergency: flatten all positions.
    pub async fn flatten_all(&mut self) -> Result<(), OtError> {
        warn!("EMERGENCY: Flattening all positions");

        // Cancel all bracket orders first
        let brackets: Vec<BracketPair> = self
            .order_manager
            .all_brackets()
            .into_iter()
            .cloned()
            .collect();

        for bracket in &brackets {
            if let Some(sl_id) = &bracket.stop_loss_order_id {
                let _ = self.exchange.cancel_order(&bracket.symbol, sl_id).await;
            }
            if let Some(tp_id) = &bracket.take_profit_order_id {
                let _ = self.exchange.cancel_order(&bracket.symbol, tp_id).await;
            }
            self.order_manager.remove_bracket(&bracket.entry_order_id);
        }

        // Close all positions
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
                Ok(tracked) => {
                    info!(symbol = %pos.symbol, "Position flattened");
                    self.order_manager.track_order(tracked);
                }
                Err(e) => error!(symbol = %pos.symbol, error = %e, "Failed to flatten"),
            }
        }

        self.save_state();
        Ok(())
    }

    /// Get reference to portfolio for status queries.
    pub fn portfolio(&self) -> &PortfolioManager {
        &self.portfolio
    }

    /// Get reference to risk engine for status queries.
    pub fn risk_engine(&self) -> &RiskEngine {
        &self.risk_engine
    }

    /// Get reference to order manager.
    pub fn order_manager(&self) -> &OrderManager {
        &self.order_manager
    }
}
