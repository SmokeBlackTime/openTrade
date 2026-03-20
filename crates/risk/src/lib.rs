//! Risk management engine for OpenTrade.
//!
//! Enforces hard risk limits at strategy, symbol, and portfolio levels.
//! Implements kill switches, drawdown circuit breakers, and position sizing.

use chrono::{DateTime, Utc};
use ot_config::RiskConfig;
use ot_types::orders::OrderRequest;
use ot_types::positions::Position;
use ot_types::risk::*;
use ot_types::signals::Signal;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tracing::{info, warn};

/// Central risk controller.
pub struct RiskEngine {
    config: RiskConfig,
    kill_switches: HashMap<KillSwitchType, KillSwitchState>,
    daily_pnl: Decimal,
    daily_trade_count: usize,
    peak_equity: Decimal,
    current_equity: Decimal,
    order_rejections_this_hour: u32,
    orders_this_minute: u32,
    last_minute_reset: DateTime<Utc>,
    last_hour_reset: DateTime<Utc>,
}

impl RiskEngine {
    pub fn new(config: RiskConfig, initial_equity: Decimal) -> Self {
        let mut kill_switches = HashMap::new();
        kill_switches.insert(KillSwitchType::Global, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::ExchangeConnectivity, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::StaleMarketData, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::ModelConfidence, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::ExtremeVolatility, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::OrderRejectionAnomaly, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::RunawayTrading, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::DailyLossLimit, KillSwitchState::Active);
        kill_switches.insert(KillSwitchType::MaxDrawdown, KillSwitchState::Active);

        Self {
            config,
            kill_switches,
            daily_pnl: dec!(0),
            daily_trade_count: 0,
            peak_equity: initial_equity,
            current_equity: initial_equity,
            order_rejections_this_hour: 0,
            orders_this_minute: 0,
            last_minute_reset: Utc::now(),
            last_hour_reset: Utc::now(),
        }
    }

    /// Check whether any kill switch is triggered.
    pub fn any_kill_switch_triggered(&self) -> bool {
        self.kill_switches
            .values()
            .any(|s| *s == KillSwitchState::Triggered)
    }

    /// Trigger a specific kill switch.
    pub fn trigger_kill_switch(&mut self, switch: KillSwitchType, reason: &str) {
        warn!(switch = %switch, reason = %reason, "Kill switch triggered");
        self.kill_switches.insert(switch, KillSwitchState::Triggered);
    }

    /// Reset a kill switch (manual override).
    pub fn reset_kill_switch(&mut self, switch: KillSwitchType) {
        info!(switch = %switch, "Kill switch reset");
        self.kill_switches.insert(switch, KillSwitchState::Active);
    }

    /// Pre-trade risk check. Returns a verdict on whether the order is allowed.
    pub fn check_order(
        &mut self,
        signal: &Signal,
        order: &OrderRequest,
        positions: &[Position],
    ) -> RiskVerdict {
        // Reset rate counters if needed
        let now = Utc::now();
        if (now - self.last_minute_reset).num_seconds() >= 60 {
            self.orders_this_minute = 0;
            self.last_minute_reset = now;
        }
        if (now - self.last_hour_reset).num_seconds() >= 3600 {
            self.order_rejections_this_hour = 0;
            self.last_hour_reset = now;
        }

        // 1. Global kill switch
        if self.any_kill_switch_triggered() {
            return RiskVerdict::Rejected {
                reason: "Kill switch is active".into(),
            };
        }

        // 2. Confidence threshold
        if signal.confidence < self.config.min_confidence_threshold {
            return RiskVerdict::Rejected {
                reason: format!(
                    "Confidence {} below threshold {}",
                    signal.confidence, self.config.min_confidence_threshold
                ),
            };
        }

        // 3. Daily loss limit
        let daily_loss_pct = if self.current_equity > dec!(0) {
            (self.daily_pnl / self.current_equity * dec!(100)).abs()
        } else {
            dec!(100)
        };
        if self.daily_pnl < dec!(0) && daily_loss_pct > self.config.max_daily_loss_pct {
            self.trigger_kill_switch(
                KillSwitchType::DailyLossLimit,
                &format!("Daily loss {}%", daily_loss_pct),
            );
            return RiskVerdict::Rejected {
                reason: format!("Daily loss limit breached: {}%", daily_loss_pct),
            };
        }

        // 4. Max drawdown
        let drawdown_pct = if self.peak_equity > dec!(0) {
            (self.peak_equity - self.current_equity) / self.peak_equity * dec!(100)
        } else {
            dec!(0)
        };
        if drawdown_pct > self.config.max_drawdown_pct {
            self.trigger_kill_switch(
                KillSwitchType::MaxDrawdown,
                &format!("Drawdown {}%", drawdown_pct),
            );
            return RiskVerdict::Rejected {
                reason: format!("Max drawdown breached: {}%", drawdown_pct),
            };
        }

        // 5. Max open positions
        let open_count = positions.iter().filter(|p| !p.is_flat()).count();
        if open_count >= self.config.max_open_positions {
            return RiskVerdict::Rejected {
                reason: format!(
                    "Max open positions {} reached",
                    self.config.max_open_positions
                ),
            };
        }

        // 6. Daily trade count
        if self.daily_trade_count >= self.config.max_trades_per_day {
            return RiskVerdict::Rejected {
                reason: format!(
                    "Max daily trades {} reached",
                    self.config.max_trades_per_day
                ),
            };
        }

        // 7. Rate limit
        if self.orders_this_minute >= self.config.max_orders_per_minute {
            return RiskVerdict::Rejected {
                reason: "Order rate limit exceeded".into(),
            };
        }

        // 8. Single order size
        let notional = order.quantity * order.price.unwrap_or(dec!(0));
        if notional > self.config.max_single_order_usd {
            return RiskVerdict::ReducedSize {
                new_quantity: self.config.max_single_order_usd
                    / order.price.unwrap_or(dec!(1)),
                reason: format!(
                    "Notional {} exceeds max {}",
                    notional, self.config.max_single_order_usd
                ),
            };
        }

        // 9. Max position size for this symbol
        let existing_notional: Decimal = positions
            .iter()
            .filter(|p| p.symbol == order.symbol && !p.is_flat())
            .map(|p| p.notional_value())
            .sum();
        if existing_notional + notional > self.config.max_position_size_usd {
            return RiskVerdict::Rejected {
                reason: format!(
                    "Position size would exceed max {} for {}",
                    self.config.max_position_size_usd, order.symbol
                ),
            };
        }

        // 10. Total notional exposure
        let total_exposure: Decimal = positions
            .iter()
            .filter(|p| !p.is_flat())
            .map(|p| p.notional_value())
            .sum();
        if total_exposure + notional > self.config.max_notional_exposure_usd {
            return RiskVerdict::Rejected {
                reason: format!(
                    "Total exposure would exceed max {}",
                    self.config.max_notional_exposure_usd
                ),
            };
        }

        // 11. Leverage check
        if self.current_equity > dec!(0) {
            let effective_leverage =
                (total_exposure + notional) / self.current_equity;
            if effective_leverage > self.config.max_leverage {
                return RiskVerdict::Rejected {
                    reason: format!(
                        "Leverage {} would exceed max {}",
                        effective_leverage, self.config.max_leverage
                    ),
                };
            }
        }

        // All checks passed
        self.orders_this_minute += 1;
        RiskVerdict::Approved
    }

    /// Record a completed trade.
    pub fn record_trade(&mut self, pnl: Decimal) {
        self.daily_pnl += pnl;
        self.daily_trade_count += 1;
        self.current_equity += pnl;
        if self.current_equity > self.peak_equity {
            self.peak_equity = self.current_equity;
        }
    }

    /// Record an order rejection from the exchange.
    pub fn record_order_rejection(&mut self) {
        self.order_rejections_this_hour += 1;
        if self.order_rejections_this_hour >= self.config.max_order_rejections_per_hour {
            self.trigger_kill_switch(
                KillSwitchType::OrderRejectionAnomaly,
                &format!(
                    "{} rejections in the last hour",
                    self.order_rejections_this_hour
                ),
            );
        }
    }

    /// Update equity for drawdown tracking.
    pub fn update_equity(&mut self, equity: Decimal) {
        self.current_equity = equity;
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
    }

    /// Reset daily counters (call at start of new trading day).
    pub fn reset_daily(&mut self) {
        self.daily_pnl = dec!(0);
        self.daily_trade_count = 0;
    }

    /// Get current risk snapshot.
    pub fn snapshot(&self, positions: &[Position]) -> PortfolioRiskSnapshot {
        let total_unrealized: Decimal =
            positions.iter().map(|p| p.unrealized_pnl).sum();
        let total_notional: Decimal = positions
            .iter()
            .filter(|p| !p.is_flat())
            .map(|p| p.notional_value())
            .sum();
        let leverage = if self.current_equity > dec!(0) {
            total_notional / self.current_equity
        } else {
            dec!(0)
        };
        let drawdown = if self.peak_equity > dec!(0) {
            (self.peak_equity - self.current_equity) / self.peak_equity * dec!(100)
        } else {
            dec!(0)
        };

        PortfolioRiskSnapshot {
            total_equity: self.current_equity,
            total_notional_exposure: total_notional,
            total_unrealized_pnl: total_unrealized,
            total_realized_pnl_today: self.daily_pnl,
            current_drawdown_pct: drawdown,
            peak_equity: self.peak_equity,
            open_position_count: positions.iter().filter(|p| !p.is_flat()).count(),
            leverage_ratio: leverage,
            var_95: None,
            cvar_95: None,
        }
    }

    pub fn current_equity(&self) -> Decimal {
        self.current_equity
    }

    pub fn daily_pnl(&self) -> Decimal {
        self.daily_pnl
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ot_types::market::{MarketType, Symbol, Timeframe};
    use ot_types::orders::*;
    use ot_types::signals::*;

    fn test_config() -> RiskConfig {
        RiskConfig {
            max_position_size_usd: dec!(100000),
            max_leverage: dec!(3),
            max_daily_loss_pct: dec!(2),
            max_drawdown_pct: dec!(10),
            max_open_positions: 5,
            max_notional_exposure_usd: dec!(500000),
            max_single_order_usd: dec!(50000),
            max_trades_per_day: 100,
            max_correlated_exposure_pct: dec!(40),
            stale_data_max_age_secs: 30,
            max_spread_bps: dec!(50),
            min_confidence_threshold: dec!(0.5),
            extreme_volatility_multiplier: dec!(3),
            max_order_rejections_per_hour: 5,
            max_orders_per_minute: 30,
        }
    }

    fn test_signal(confidence: Decimal) -> Signal {
        Signal {
            strategy_name: "test".into(),
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            timeframe: Timeframe::H1,
            timestamp: Utc::now(),
            direction: SignalDirection::Long,
            strength: dec!(1),
            confidence,
            entry_price: Some(dec!(50000)),
            stop_loss: Some(dec!(49000)),
            take_profit: Some(dec!(52000)),
            time_stop_bars: None,
            metadata: SignalMetadata {
                signal_inputs: serde_json::json!({}),
                model_outputs: None,
                uncertainty_score: None,
                regime: None,
                risk_overrides: vec![],
                portfolio_context: None,
            },
        }
    }

    fn test_order() -> OrderRequest {
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
                description: "test order".into(),
            },
            created_at: Utc::now(),
        }
    }

    #[test]
    fn approve_valid_order() {
        let mut engine = RiskEngine::new(test_config(), dec!(100000));
        let verdict = engine.check_order(&test_signal(dec!(0.7)), &test_order(), &[]);
        assert!(matches!(verdict, RiskVerdict::Approved));
    }

    #[test]
    fn reject_low_confidence() {
        let mut engine = RiskEngine::new(test_config(), dec!(100000));
        let verdict = engine.check_order(&test_signal(dec!(0.3)), &test_order(), &[]);
        assert!(matches!(verdict, RiskVerdict::Rejected { .. }));
    }

    #[test]
    fn reject_when_kill_switch_triggered() {
        let mut engine = RiskEngine::new(test_config(), dec!(100000));
        engine.trigger_kill_switch(KillSwitchType::Global, "test");
        let verdict = engine.check_order(&test_signal(dec!(0.7)), &test_order(), &[]);
        assert!(matches!(verdict, RiskVerdict::Rejected { .. }));
    }

    #[test]
    fn drawdown_tracking() {
        let mut engine = RiskEngine::new(test_config(), dec!(100000));
        engine.record_trade(dec!(-5000));
        assert_eq!(engine.current_equity(), dec!(95000));
        let snap = engine.snapshot(&[]);
        assert_eq!(snap.current_drawdown_pct, dec!(5));
    }

    #[test]
    fn daily_loss_triggers_kill_switch() {
        let mut engine = RiskEngine::new(test_config(), dec!(100000));
        engine.record_trade(dec!(-3000));
        // Daily loss is 3% which exceeds 2% limit
        let verdict = engine.check_order(&test_signal(dec!(0.7)), &test_order(), &[]);
        assert!(matches!(verdict, RiskVerdict::Rejected { .. }));
    }
}
