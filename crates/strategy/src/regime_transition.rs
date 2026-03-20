//! Regime transition strategy.
//!
//! Trades the TRANSITION between market regimes, not the regime itself.
//! When regime stability drops (indicating a shift is underway), enter
//! in the direction of the emerging regime before trend-followers pile in.

use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::collections::VecDeque;

use crate::Strategy;

/// Regime state for tracking transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegimeState {
    TrendingUp,
    TrendingDown,
    Ranging,
    HighVol,
    LowVol,
}

impl RegimeState {
    fn from_features(features: &FeatureRow) -> Self {
        let trend = features.trend_strength.unwrap_or(dec!(0));
        let vol = features.realized_vol_20.unwrap_or(dec!(30));
        let bb_w = features.bb_width.unwrap_or(dec!(0.03));

        if vol > dec!(60) {
            Self::HighVol
        } else if bb_w < dec!(0.015) && vol < dec!(15) {
            Self::LowVol
        } else if trend > dec!(2) {
            Self::TrendingUp
        } else if trend < dec!(-2) {
            Self::TrendingDown
        } else {
            Self::Ranging
        }
    }
}

/// Trades regime transitions.
pub struct RegimeTransition {
    name: String,
    /// Stability threshold below which we consider a transition is happening.
    stability_threshold: Decimal,
    /// Lookback for stability calculation.
    stability_lookback: usize,
    /// Minimum trend strength to identify emerging trend.
    min_emerging_trend: Decimal,
    /// ATR-based stops.
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    cooldown: u32,
    bars_since_signal: u32,
    /// Regime history for stability tracking.
    regime_history: VecDeque<RegimeState>,
    max_history: usize,
}

impl RegimeTransition {
    pub fn new(params: &HashMap<String, serde_json::Value>) -> Self {
        let get_str_dec = |key: &str, default: Decimal| -> Decimal {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        };
        let get_u64 = |key: &str, default: u64| -> u64 {
            params
                .get(key)
                .and_then(|v| v.as_u64())
                .unwrap_or(default)
        };

        Self {
            name: params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("regime_transition")
                .to_string(),
            stability_threshold: get_str_dec("stability_threshold", dec!(0.5)),
            stability_lookback: get_u64("stability_lookback", 10) as usize,
            min_emerging_trend: get_str_dec("min_emerging_trend", dec!(0.8)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2.5)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(3.5)),
            cooldown: get_u64("cooldown", 8) as u32,
            bars_since_signal: 0,
            regime_history: VecDeque::with_capacity(50),
            max_history: 50,
        }
    }

    /// Calculate regime stability: fraction of recent bars in the current regime.
    fn stability(&self) -> Decimal {
        if self.regime_history.is_empty() {
            return dec!(1);
        }

        let current = match self.regime_history.back() {
            Some(r) => *r,
            None => return dec!(1),
        };

        let lookback_start = self.regime_history.len().saturating_sub(self.stability_lookback);
        let recent: Vec<RegimeState> = self.regime_history.iter().skip(lookback_start).copied().collect();

        if recent.is_empty() {
            return dec!(1);
        }

        let matching = recent.iter().filter(|&&r| r == current).count();
        Decimal::from(matching as u32) / Decimal::from(recent.len() as u32)
    }

    /// Detect which direction the regime is transitioning to.
    fn emerging_direction(&self, features: &FeatureRow) -> Option<SignalDirection> {
        let trend = features.trend_strength?;
        let rsi = features.rsi_14.unwrap_or(dec!(50));
        let macd = features.macd.unwrap_or(dec!(0));

        // Multiple confirmations needed for emerging direction
        let bullish_signals = (trend > self.min_emerging_trend) as u8
            + (rsi > dec!(55)) as u8
            + (macd > dec!(0)) as u8;

        let bearish_signals = (trend < -self.min_emerging_trend) as u8
            + (rsi < dec!(45)) as u8
            + (macd < dec!(0)) as u8;

        if bullish_signals >= 2 {
            Some(SignalDirection::Long)
        } else if bearish_signals >= 2 {
            Some(SignalDirection::Short)
        } else {
            None
        }
    }
}

impl Strategy for RegimeTransition {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_position: Option<&Position>,
    ) -> Option<Signal> {
        self.bars_since_signal += 1;

        // Track regime
        let current_regime = RegimeState::from_features(features);
        self.regime_history.push_back(current_regime);
        if self.regime_history.len() > self.max_history {
            self.regime_history.pop_front();
        }

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        if self.regime_history.len() < self.stability_lookback {
            return None; // Not enough history
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        if has_position {
            return None;
        }

        let atr = features.atr_14?;
        let stability = self.stability();

        // Only trade when stability is LOW (regime is changing)
        if stability >= self.stability_threshold {
            return None; // Regime is stable, no transition
        }

        // Identify the emerging direction
        let direction = self.emerging_direction(features)?;

        // Confidence inversely proportional to stability (less stable = more confident in transition)
        let transition_confidence = dec!(1) - stability;
        let confidence = (dec!(0.5) + transition_confidence * dec!(0.3)).min(dec!(0.85));

        let (stop, target) = match direction {
            SignalDirection::Long => (
                candle.close - atr * self.atr_stop_multiplier,
                candle.close + atr * self.atr_target_multiplier,
            ),
            SignalDirection::Short => (
                candle.close + atr * self.atr_stop_multiplier,
                candle.close - atr * self.atr_target_multiplier,
            ),
            _ => return None,
        };

        self.bars_since_signal = 0;

        Some(Signal {
            strategy_name: self.name.clone(),
            symbol: candle.symbol.clone(),
            market_type: candle.market_type,
            timeframe: candle.timeframe,
            timestamp: Utc::now(),
            direction,
            strength: transition_confidence,
            confidence,
            entry_price: Some(candle.close),
            stop_loss: Some(stop),
            take_profit: Some(target),
            time_stop_bars: Some(30),
            metadata: SignalMetadata {
                signal_inputs: serde_json::json!({
                    "stability": stability.to_string(),
                    "current_regime": format!("{:?}", current_regime),
                    "trend_strength": features.trend_strength.map(|t| t.to_string()),
                    "reason": "regime_transition_detected",
                }),
                model_outputs: None,
                uncertainty_score: Some(stability), // Higher stability = more uncertain about transition
                regime: Some("transitional".into()),
                risk_overrides: vec![],
                portfolio_context: None,
            },
        })
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "stability_threshold".into(),
            serde_json::json!(self.stability_threshold.to_string()),
        );
        m.insert(
            "stability_lookback".into(),
            serde_json::json!(self.stability_lookback),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.regime_history.clear();
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_construction() {
        let params = HashMap::new();
        let s = RegimeTransition::new(&params);
        assert_eq!(s.name(), "regime_transition");
        assert_eq!(s.stability(), dec!(1)); // No history yet
    }
}
