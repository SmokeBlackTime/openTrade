use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Momentum continuation strategy.
///
/// Enters in the direction of recent strong returns when momentum is confirmed
/// by multiple timeframe alignment (via features).
pub struct Momentum {
    name: String,
    return_threshold_pct: Decimal,
    rsi_confirm_min: Decimal,
    rsi_confirm_max: Decimal,
    atr_stop_multiplier: Decimal,
    bars_since_signal: u32,
    cooldown: u32,
    allow_short: bool,
}

impl Momentum {
    pub fn new(params: &HashMap<String, serde_json::Value>) -> Self {
        let get_str_dec = |key: &str, default: Decimal| -> Decimal {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        };
        let get_u64 = |key: &str, default: u64| -> u64 {
            params.get(key).and_then(|v| v.as_u64()).unwrap_or(default)
        };

        Self {
            name: params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("momentum")
                .to_string(),
            return_threshold_pct: get_str_dec("return_threshold_pct", dec!(2)),
            rsi_confirm_min: get_str_dec("rsi_confirm_min", dec!(55)),
            rsi_confirm_max: get_str_dec("rsi_confirm_max", dec!(80)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2.5)),
            bars_since_signal: 0,
            cooldown: get_u64("cooldown", 5) as u32,
            allow_short: params
                .get("allow_short")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }
}

impl Strategy for Momentum {
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

        let return_5 = features.return_5?;
        let rsi = features.rsi_14?;
        let atr = features.atr_14?;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position.map(|p| !p.is_flat()).unwrap_or(false);
        if has_position {
            return None;
        }

        // Long momentum: strong positive returns + RSI confirming
        if return_5 > self.return_threshold_pct
            && rsi > self.rsi_confirm_min
            && rsi < self.rsi_confirm_max
        {
            self.bars_since_signal = 0;
            let confidence = dec!(0.6)
                + (return_5 - self.return_threshold_pct).min(dec!(5)) / dec!(50);
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
                take_profit: Some(candle.close + atr * dec!(4)),
                time_stop_bars: Some(40),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "return_5": return_5.to_string(),
                        "rsi": rsi.to_string(),
                        "reason": "momentum_long",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("momentum_up".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        // Short momentum
        if self.allow_short
            && return_5 < -self.return_threshold_pct
            && rsi < (dec!(100) - self.rsi_confirm_min)
            && rsi > (dec!(100) - self.rsi_confirm_max)
        {
            self.bars_since_signal = 0;
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Short,
                strength: dec!(1),
                confidence: dec!(0.6),
                entry_price: Some(candle.close),
                stop_loss: Some(candle.close + atr * self.atr_stop_multiplier),
                take_profit: Some(candle.close - atr * dec!(4)),
                time_stop_bars: Some(40),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "return_5": return_5.to_string(),
                        "rsi": rsi.to_string(),
                        "reason": "momentum_short",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(0.4)),
                    regime: Some("momentum_down".into()),
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
            "return_threshold_pct".into(),
            serde_json::json!(self.return_threshold_pct.to_string()),
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
