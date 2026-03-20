use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::{Position, PositionSide};
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Mean reversion strategy using Bollinger Bands and RSI.
///
/// Entry long: price touches lower BB AND RSI < oversold threshold
/// Entry short: price touches upper BB AND RSI > overbought threshold
/// Exit: price returns to middle BB or opposite signal
#[allow(dead_code)]
pub struct MeanReversion {
    name: String,
    rsi_overbought: Decimal,
    rsi_oversold: Decimal,
    bb_entry_pct: Decimal,
    atr_stop_multiplier: Decimal,
    bars_since_signal: u32,
    cooldown: u32,
    allow_short: bool,
}

impl MeanReversion {
    pub fn new(params: &HashMap<String, serde_json::Value>) -> Self {
        let get_u64 = |key: &str, default: u64| -> u64 {
            params.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
        };
        let get_str_dec = |key: &str, default: Decimal| -> Decimal {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        };

        Self {
            name: params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("mean_reversion")
                .to_string(),
            rsi_overbought: get_str_dec("rsi_overbought", dec!(75)),
            rsi_oversold: get_str_dec("rsi_oversold", dec!(25)),
            bb_entry_pct: get_str_dec("bb_entry_pct", dec!(0.95)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(1.5)),
            bars_since_signal: 0,
            cooldown: get_u64("cooldown", 3) as u32,
            allow_short: params
                .get("allow_short")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }
}

impl Strategy for MeanReversion {
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

        let rsi = features.rsi_14?;
        let bb_upper = features.bb_upper?;
        let bb_lower = features.bb_lower?;
        let bb_middle = features.bb_middle?;
        let atr = features.atr_14?;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position.map(|p| !p.is_flat()).unwrap_or(false);
        let position_side = current_position
            .map(|p| p.side)
            .unwrap_or(PositionSide::Flat);

        // Exit: price returned to mean
        if has_position {
            let exit = match position_side {
                PositionSide::Long => candle.close >= bb_middle,
                PositionSide::Short => candle.close <= bb_middle,
                _ => false,
            };
            if exit {
                self.bars_since_signal = 0;
                return Some(Signal {
                    strategy_name: self.name.clone(),
                    symbol: candle.symbol.clone(),
                    market_type: candle.market_type,
                    timeframe: candle.timeframe,
                    timestamp: Utc::now(),
                    direction: SignalDirection::Flat,
                    strength: dec!(0.7),
                    confidence: dec!(0.65),
                    entry_price: Some(candle.close),
                    stop_loss: None,
                    take_profit: None,
                    time_stop_bars: None,
                    metadata: SignalMetadata {
                        signal_inputs: serde_json::json!({
                            "reason": "mean_reversion_exit",
                            "rsi": rsi.to_string(),
                        }),
                        model_outputs: None,
                        uncertainty_score: None,
                        regime: Some("ranging".into()),
                        risk_overrides: vec![],
                        portfolio_context: None,
                    },
                });
            }
            return None;
        }

        // Long entry: price near lower BB + RSI oversold
        if candle.close <= bb_lower && rsi < self.rsi_oversold {
            self.bars_since_signal = 0;
            let confidence = dec!(0.5) + (self.rsi_oversold - rsi) / dec!(100) * dec!(0.3);
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Long,
                strength: dec!(1),
                confidence: confidence.min(dec!(0.85)),
                entry_price: Some(candle.close),
                stop_loss: Some(candle.close - atr * self.atr_stop_multiplier),
                take_profit: Some(bb_middle),
                time_stop_bars: Some(20),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "rsi": rsi.to_string(),
                        "bb_lower": bb_lower.to_string(),
                        "reason": "mean_reversion_long",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("oversold".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        // Short entry: price near upper BB + RSI overbought
        if self.allow_short && candle.close >= bb_upper && rsi > self.rsi_overbought {
            self.bars_since_signal = 0;
            let confidence = dec!(0.5) + (rsi - self.rsi_overbought) / dec!(100) * dec!(0.3);
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Short,
                strength: dec!(1),
                confidence: confidence.min(dec!(0.85)),
                entry_price: Some(candle.close),
                stop_loss: Some(candle.close + atr * self.atr_stop_multiplier),
                take_profit: Some(bb_middle),
                time_stop_bars: Some(20),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "rsi": rsi.to_string(),
                        "bb_upper": bb_upper.to_string(),
                        "reason": "mean_reversion_short",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("overbought".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        None
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "rsi_overbought".into(),
            serde_json::json!(self.rsi_overbought.to_string()),
        );
        m.insert(
            "rsi_oversold".into(),
            serde_json::json!(self.rsi_oversold.to_string()),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}
