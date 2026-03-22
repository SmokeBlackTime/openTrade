//! Cross-timeframe disagreement strategy.
//!
//! When indicators on different timeframes disagree, the higher timeframe
//! usually wins. This strategy detects disagreements between the primary
//! timeframe and a cached higher-timeframe view.

use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Cross-timeframe disagreement strategy.
///
/// Receives features from the primary (lower) timeframe on each bar.
/// Higher timeframe features must be updated externally via `set_higher_tf_features`.
pub struct CrossTimeframe {
    name: String,
    /// Minimum disagreement magnitude to trigger.
    min_disagreement: Decimal,
    /// Higher timeframe RSI to compare against.
    higher_tf_rsi: Option<Decimal>,
    /// Higher timeframe trend strength.
    higher_tf_trend: Option<Decimal>,
    /// Higher timeframe MACD.
    higher_tf_macd: Option<Decimal>,
    /// ATR multiplier for stops.
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    cooldown: u32,
    bars_since_signal: u32,
}

impl CrossTimeframe {
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
                .unwrap_or("cross_timeframe")
                .to_string(),
            min_disagreement: get_str_dec("min_disagreement", dec!(15)),
            higher_tf_rsi: None,
            higher_tf_trend: None,
            higher_tf_macd: None,
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(3)),
            cooldown: get_u64("cooldown", 5) as u32,
            bars_since_signal: 0,
        }
    }

    /// Update higher timeframe features (called when higher TF candle closes).
    pub fn set_higher_tf_features(&mut self, features: &FeatureRow) {
        self.higher_tf_rsi = features.rsi_14;
        self.higher_tf_trend = features.trend_strength;
        self.higher_tf_macd = features.macd;
    }

    /// Detect disagreement between timeframes.
    /// Returns (direction to take based on higher TF, disagreement magnitude).
    fn detect_disagreement(&self, lower_features: &FeatureRow) -> Option<(SignalDirection, Decimal)> {
        let lower_rsi = lower_features.rsi_14?;
        let higher_rsi = self.higher_tf_rsi?;
        let higher_trend = self.higher_tf_trend?;

        let rsi_diff = lower_rsi - higher_rsi;

        // Significant RSI disagreement
        if rsi_diff.abs() < self.min_disagreement {
            return None;
        }

        // Higher timeframe trend determines direction
        if higher_trend > dec!(1) && lower_rsi < dec!(40) {
            // Higher TF trending up, lower TF says oversold → buy the dip
            Some((SignalDirection::Long, rsi_diff.abs()))
        } else if higher_trend < dec!(-1) && lower_rsi > dec!(60) {
            // Higher TF trending down, lower TF says overbought → sell the rally
            Some((SignalDirection::Short, rsi_diff.abs()))
        } else {
            None
        }
    }
}

impl Strategy for CrossTimeframe {
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

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        if has_position {
            return None;
        }

        let atr = features.atr_14?;
        let (direction, disagreement) = self.detect_disagreement(features)?;

        // Confidence scales with disagreement magnitude
        let confidence = (dec!(0.5) + disagreement / dec!(100)).min(dec!(0.8));

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
            strength: dec!(0.8),
            confidence,
            entry_price: Some(candle.close),
            stop_loss: Some(stop),
            take_profit: Some(target),
            time_stop_bars: Some(20),
            metadata: SignalMetadata {
                signal_inputs: serde_json::json!({
                    "lower_rsi": features.rsi_14.map(|r| r.to_string()),
                    "higher_rsi": self.higher_tf_rsi.map(|r| r.to_string()),
                    "higher_trend": self.higher_tf_trend.map(|t| t.to_string()),
                    "disagreement": disagreement.to_string(),
                    "reason": "cross_timeframe_disagreement",
                }),
                model_outputs: None,
                uncertainty_score: Some(dec!(1) - confidence),
                regime: None,
                risk_overrides: vec![],
                portfolio_context: None,
            },
        })
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "min_disagreement".into(),
            serde_json::json!(self.min_disagreement.to_string()),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.higher_tf_rsi = None;
        self.higher_tf_trend = None;
        self.higher_tf_macd = None;
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
        let s = CrossTimeframe::new(&HashMap::new());
        assert_eq!(s.name(), "cross_timeframe");
    }

    #[test]
    fn no_signal_without_higher_tf() {
        let s = CrossTimeframe::new(&HashMap::new());
        // Without higher_tf features set, detect_disagreement returns None
        let features = FeatureRow {
            timestamp_ms: 0,
            close: dec!(50000),
            rsi_14: Some(dec!(30)),
            trend_strength: Some(dec!(1)),
            ..default_features()
        };
        assert!(s.detect_disagreement(&features).is_none());
    }

    fn default_features() -> FeatureRow {
        FeatureRow {
            timestamp_ms: 0,
            close: dec!(50000),
            return_1: None,
            return_5: None,
            log_return_1: None,
            sma_20: None,
            sma_50: None,
            ema_12: None,
            ema_26: None,
            macd: None,
            rsi_14: None,
            atr_14: None,
            bb_upper: None,
            bb_middle: None,
            bb_lower: None,
            realized_vol_20: None,
            bb_width: None,
            price_vs_sma20: None,
            price_vs_sma50: None,
            trend_strength: None,
            volume_sma_20: None,
            volume_ratio: None,
            macd_signal_line: None,
            macd_histogram: None,
            funding_rate: None,
        }
    }
}
