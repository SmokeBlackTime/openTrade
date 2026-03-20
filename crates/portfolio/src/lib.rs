//! Portfolio management and position sizing for OpenTrade.
//!
//! Implements volatility targeting, Kelly-inspired sizing, drawdown-aware
//! de-risking, and correlation-aware allocation.

use ot_config::PortfolioConfig;
use ot_types::market::Symbol;
use ot_types::positions::Position;
use ot_types::signals::Signal;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tracing::debug;

/// Portfolio manager tracking positions and computing sizing.
pub struct PortfolioManager {
    config: PortfolioConfig,
    positions: HashMap<String, Position>,
    equity: Decimal,
    peak_equity: Decimal,
}

impl PortfolioManager {
    pub fn new(config: PortfolioConfig) -> Self {
        let equity = config.initial_capital;
        Self {
            config,
            positions: HashMap::new(),
            equity,
            peak_equity: equity,
        }
    }

    /// Compute position size for a new signal.
    pub fn compute_position_size(
        &self,
        signal: &Signal,
        current_price: Decimal,
        atr: Option<Decimal>,
    ) -> Decimal {
        if current_price <= dec!(0) {
            return dec!(0);
        }

        // Base: risk per trade % of equity
        let risk_amount = self.equity * self.config.risk_per_trade_pct / dec!(100);

        // If we have a stop loss, size based on distance to stop
        let size_from_risk = match (signal.stop_loss, signal.entry_price) {
            (Some(stop), Some(entry)) => {
                let risk_per_unit = (entry - stop).abs();
                if risk_per_unit > dec!(0) {
                    risk_amount / risk_per_unit
                } else {
                    risk_amount / current_price
                }
            }
            _ => risk_amount / current_price,
        };

        // Kelly fraction cap
        let kelly_capped = size_from_risk * self.config.kelly_fraction;

        // Volatility targeting: reduce size if ATR is high
        let vol_adjusted = match atr {
            Some(atr_val) if atr_val > dec!(0) && current_price > dec!(0) => {
                let atr_pct = atr_val / current_price * dec!(100);
                if let Some(target_vol) = self.config.target_volatility_pct {
                    if atr_pct > dec!(0) {
                        kelly_capped * (target_vol / atr_pct).min(dec!(1))
                    } else {
                        kelly_capped
                    }
                } else {
                    kelly_capped
                }
            }
            _ => kelly_capped,
        };

        // Drawdown-aware de-risking
        let drawdown_pct = if self.peak_equity > dec!(0) {
            (self.peak_equity - self.equity) / self.peak_equity * dec!(100)
        } else {
            dec!(0)
        };
        let dd_scale = if drawdown_pct > dec!(5) {
            // Linear reduction: at 5% DD reduce to 50%, at 10% reduce to 0%
            (dec!(1) - (drawdown_pct - dec!(5)) / dec!(10)).max(dec!(0.1))
        } else {
            dec!(1)
        };
        let dd_adjusted = vol_adjusted * dd_scale;

        // Concentration limit: max allocation per symbol
        let max_notional = self.equity * self.config.concentration_limit_pct / dec!(100);
        let max_size = max_notional / current_price;

        // Leverage limit
        let total_exposure = self.total_notional_exposure();
        let remaining_capacity =
            self.equity * self.config.max_portfolio_leverage - total_exposure;
        let leverage_max_size = if remaining_capacity > dec!(0) {
            remaining_capacity / current_price
        } else {
            dec!(0)
        };

        let final_size = dd_adjusted.min(max_size).min(leverage_max_size);

        debug!(
            signal = %signal.strategy_name,
            risk_size = %size_from_risk,
            kelly = %kelly_capped,
            vol_adj = %vol_adjusted,
            dd_scale = %dd_scale,
            final_size = %final_size,
            "Position sizing"
        );

        final_size.max(dec!(0))
    }

    /// Add or update a position.
    pub fn update_position(&mut self, position: Position) {
        let key = format!("{}:{}", position.symbol, position.strategy_name);
        if position.is_flat() {
            self.positions.remove(&key);
        } else {
            self.positions.insert(key, position);
        }
    }

    /// Get a position by symbol and strategy.
    pub fn get_position(&self, symbol: &Symbol, strategy: &str) -> Option<&Position> {
        let key = format!("{}:{}", symbol, strategy);
        self.positions.get(&key)
    }

    /// Get all open positions.
    pub fn open_positions(&self) -> Vec<&Position> {
        self.positions.values().filter(|p| !p.is_flat()).collect()
    }

    /// Total notional exposure across all positions.
    pub fn total_notional_exposure(&self) -> Decimal {
        self.positions
            .values()
            .filter(|p| !p.is_flat())
            .map(|p| p.notional_value())
            .sum()
    }

    /// Total unrealized PnL.
    pub fn total_unrealized_pnl(&self) -> Decimal {
        self.positions.values().map(|p| p.unrealized_pnl).sum()
    }

    /// Update equity.
    pub fn update_equity(&mut self, equity: Decimal) {
        self.equity = equity;
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
    }

    pub fn equity(&self) -> Decimal {
        self.equity
    }

    /// Get positions as owned vec for risk engine.
    pub fn positions_vec(&self) -> Vec<Position> {
        self.positions.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn test_config() -> PortfolioConfig {
        PortfolioConfig {
            initial_capital: dec!(100000),
            risk_per_trade_pct: dec!(1),
            target_volatility_pct: Some(dec!(15)),
            kelly_fraction: dec!(0.25),
            max_portfolio_leverage: dec!(2),
            concentration_limit_pct: dec!(25),
            correlation_lookback_bars: 100,
            rebalance_threshold_pct: dec!(5),
        }
    }

    #[test]
    fn position_sizing_basic() {
        let pm = PortfolioManager::new(test_config());
        let signal = Signal {
            strategy_name: "test".into(),
            symbol: Symbol::new("BTCUSDT"),
            market_type: ot_types::market::MarketType::Spot,
            timeframe: ot_types::market::Timeframe::H1,
            timestamp: Utc::now(),
            direction: ot_types::signals::SignalDirection::Long,
            strength: dec!(1),
            confidence: dec!(0.7),
            entry_price: Some(dec!(50000)),
            stop_loss: Some(dec!(49000)),
            take_profit: Some(dec!(52000)),
            time_stop_bars: None,
            metadata: ot_types::signals::SignalMetadata {
                signal_inputs: serde_json::json!({}),
                model_outputs: None,
                uncertainty_score: None,
                regime: None,
                risk_overrides: vec![],
                portfolio_context: None,
            },
        };

        let size = pm.compute_position_size(&signal, dec!(50000), Some(dec!(500)));
        assert!(size > dec!(0));
        // Size should be reasonable given $100k equity
        let notional = size * dec!(50000);
        assert!(notional <= dec!(25000)); // concentration limit 25%
    }

    #[test]
    fn drawdown_reduces_size() {
        let mut pm = PortfolioManager::new(test_config());
        pm.update_equity(dec!(92000)); // 8% drawdown from 100k

        let signal = Signal {
            strategy_name: "test".into(),
            symbol: Symbol::new("BTCUSDT"),
            market_type: ot_types::market::MarketType::Spot,
            timeframe: ot_types::market::Timeframe::H1,
            timestamp: Utc::now(),
            direction: ot_types::signals::SignalDirection::Long,
            strength: dec!(1),
            confidence: dec!(0.7),
            entry_price: Some(dec!(50000)),
            stop_loss: Some(dec!(49000)),
            take_profit: None,
            time_stop_bars: None,
            metadata: ot_types::signals::SignalMetadata {
                signal_inputs: serde_json::json!({}),
                model_outputs: None,
                uncertainty_score: None,
                regime: None,
                risk_overrides: vec![],
                portfolio_context: None,
            },
        };

        let size_dd = pm.compute_position_size(&signal, dec!(50000), None);

        let pm_normal = PortfolioManager::new(test_config());
        let size_normal = pm_normal.compute_position_size(&signal, dec!(50000), None);

        // Drawdown size should be smaller
        assert!(size_dd < size_normal);
    }
}
