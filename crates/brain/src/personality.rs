//! Trading personality system.
//!
//! Defines the AI's trading style, risk appetite, and behavioral traits.
//! The personality influences confidence adjustments, position sizing,
//! and stop/target placement.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

/// A trading personality profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingPersonality {
    /// Name of this personality profile.
    profile_name: String,
    /// Risk appetite: 0.0 (ultra conservative) to 1.0 (aggressive).
    risk_appetite: f64,
    /// How much to trust the AI's confidence vs. being cautious.
    confidence_bias: f64,
    /// Preferred trade duration (in bars). Shorter = scalper, longer = swing.
    preferred_duration: u32,
    /// How reactive to regime changes (0-1). Higher = faster adaptation.
    regime_sensitivity: f64,
    /// Loss aversion factor. Higher = cut losses faster.
    loss_aversion: f64,
    /// Patience factor. Higher = wait for stronger signals.
    patience: f64,
    /// Current drawdown state (affects behavior dynamically).
    current_drawdown_pct: f64,
    /// Trade count today (for risk management).
    trades_today: u32,
    /// Max trades per day for this personality.
    max_trades_day: u32,
}

impl Default for TradingPersonality {
    fn default() -> Self {
        Self {
            profile_name: "balanced".into(),
            risk_appetite: 0.5,
            confidence_bias: 0.0,
            preferred_duration: 20,
            regime_sensitivity: 0.6,
            loss_aversion: 0.7,
            patience: 0.6,
            current_drawdown_pct: 0.0,
            trades_today: 0,
            max_trades_day: 10,
        }
    }
}

impl TradingPersonality {
    /// Create a conservative personality.
    pub fn conservative() -> Self {
        Self {
            profile_name: "conservative".into(),
            risk_appetite: 0.3,
            confidence_bias: -0.1, // Under-confident, waits for strong signals
            preferred_duration: 40,
            regime_sensitivity: 0.4,
            loss_aversion: 0.9,
            patience: 0.8,
            current_drawdown_pct: 0.0,
            trades_today: 0,
            max_trades_day: 5,
        }
    }

    /// Create an aggressive personality.
    pub fn aggressive() -> Self {
        Self {
            profile_name: "aggressive".into(),
            risk_appetite: 0.8,
            confidence_bias: 0.1, // Over-confident, takes more signals
            preferred_duration: 10,
            regime_sensitivity: 0.8,
            loss_aversion: 0.4,
            patience: 0.3,
            current_drawdown_pct: 0.0,
            trades_today: 0,
            max_trades_day: 20,
        }
    }

    /// Create a scalper personality.
    pub fn scalper() -> Self {
        Self {
            profile_name: "scalper".into(),
            risk_appetite: 0.6,
            confidence_bias: 0.05,
            preferred_duration: 5,
            regime_sensitivity: 0.9,
            loss_aversion: 0.8,
            patience: 0.2,
            current_drawdown_pct: 0.0,
            trades_today: 0,
            max_trades_day: 50,
        }
    }

    /// Create from config params.
    pub fn from_params(params: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        let get_f64 = |key: &str, default: f64| -> f64 {
            params
                .get(key)
                .and_then(|v| v.as_f64())
                .unwrap_or(default)
        };
        let get_u32 = |key: &str, default: u32| -> u32 {
            params
                .get(key)
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(default)
        };

        Self {
            profile_name: params
                .get("profile_name")
                .and_then(|v| v.as_str())
                .unwrap_or("custom")
                .to_string(),
            risk_appetite: get_f64("risk_appetite", 0.5),
            confidence_bias: get_f64("confidence_bias", 0.0),
            preferred_duration: get_u32("preferred_duration", 20),
            regime_sensitivity: get_f64("regime_sensitivity", 0.6),
            loss_aversion: get_f64("loss_aversion", 0.7),
            patience: get_f64("patience", 0.6),
            current_drawdown_pct: 0.0,
            trades_today: 0,
            max_trades_day: get_u32("max_trades_day", 10),
        }
    }

    /// Personality name.
    pub fn name(&self) -> &str {
        &self.profile_name
    }

    /// Adjust confidence based on personality and market conditions.
    pub fn adjust_confidence(&self, raw_confidence: f64, regime: &str) -> f64 {
        let mut adjusted = raw_confidence + self.confidence_bias;

        // Reduce confidence during drawdown (dynamic de-risking)
        if self.current_drawdown_pct > 0.02 {
            let drawdown_penalty = self.current_drawdown_pct * self.loss_aversion;
            adjusted -= drawdown_penalty;
        }

        // Regime-based adjustments
        match regime {
            "HighVolatility" | "high_volatility" => {
                // Reduce confidence in volatile markets (unless aggressive)
                adjusted *= 1.0 - (0.3 * (1.0 - self.risk_appetite));
            }
            "Transitional" | "transitional" => {
                // Extra caution during transitions
                adjusted *= 0.8;
            }
            "LowVolatility" | "low_volatility" => {
                // Slightly boost in calm markets for patient traders
                if self.patience > 0.6 {
                    adjusted *= 1.05;
                }
            }
            _ => {}
        }

        // Patience filter: require higher confidence for impatient traders
        let patience_threshold = 0.5 + (self.patience * 0.2);
        if adjusted < patience_threshold {
            adjusted *= 0.5; // Heavily penalize low-confidence signals
        }

        // Trade count limit
        if self.trades_today >= self.max_trades_day {
            adjusted = 0.0; // No more trades today
        }

        adjusted.clamp(0.0, 1.0)
    }

    /// Stop loss multiplier based on personality.
    pub fn stop_loss_multiplier(&self) -> Decimal {
        // Conservative: wider stops (2.5x ATR), Aggressive: tighter (1.5x ATR)
        let base = 2.0 - (self.risk_appetite * 0.7);
        // Loss aversion increases stop distance slightly
        let adjusted = base + (self.loss_aversion * 0.3);
        Decimal::try_from(adjusted).unwrap_or(dec!(2))
    }

    /// Take profit multiplier based on personality.
    pub fn take_profit_multiplier(&self) -> Decimal {
        // Aggressive: wider targets (4x ATR), Conservative: tighter (2x ATR)
        let base = 2.0 + (self.risk_appetite * 2.0);
        Decimal::try_from(base).unwrap_or(dec!(3))
    }

    /// Maximum bars to hold a position.
    pub fn max_hold_bars(&self) -> u32 {
        self.preferred_duration * 2
    }

    /// Update drawdown state (called by portfolio manager).
    pub fn update_drawdown(&mut self, drawdown_pct: f64) {
        self.current_drawdown_pct = drawdown_pct;
    }

    /// Increment trade count.
    pub fn record_trade(&mut self) {
        self.trades_today += 1;
    }

    /// Reset daily counters.
    pub fn reset_daily(&mut self) {
        self.trades_today = 0;
    }

    /// Describe current state (for decision logging).
    pub fn state_description(&self) -> String {
        format!(
            "Profile: {}, Risk: {:.0}%, DD: {:.1}%, Trades: {}/{}, Patience: {:.0}%",
            self.profile_name,
            self.risk_appetite * 100.0,
            self.current_drawdown_pct * 100.0,
            self.trades_today,
            self.max_trades_day,
            self.patience * 100.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_personality() {
        let p = TradingPersonality::default();
        assert_eq!(p.name(), "balanced");
        assert!(p.risk_appetite > 0.0 && p.risk_appetite < 1.0);
    }

    #[test]
    fn conservative_reduces_confidence() {
        let p = TradingPersonality::conservative();
        let adjusted = p.adjust_confidence(0.7, "HighVolatility");
        assert!(adjusted < 0.7, "Conservative should reduce confidence in high vol");
    }

    #[test]
    fn aggressive_wider_targets() {
        let agg = TradingPersonality::aggressive();
        let cons = TradingPersonality::conservative();
        assert!(agg.take_profit_multiplier() > cons.take_profit_multiplier());
    }

    #[test]
    fn drawdown_reduces_confidence() {
        let mut p = TradingPersonality::default();
        let before = p.adjust_confidence(0.8, "Ranging");
        p.update_drawdown(0.05);
        let after = p.adjust_confidence(0.8, "Ranging");
        assert!(after < before, "Drawdown should reduce confidence");
    }

    #[test]
    fn trade_limit_blocks_signals() {
        let mut p = TradingPersonality::default();
        p.max_trades_day = 2;
        p.trades_today = 2;
        let confidence = p.adjust_confidence(0.9, "TrendingUp");
        assert_eq!(confidence, 0.0, "Should block signals when at daily limit");
    }
}
