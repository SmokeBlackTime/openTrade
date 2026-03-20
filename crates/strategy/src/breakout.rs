use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Breakout strategy detecting range expansion.
///
/// Enters when price breaks above/below N-bar high/low with volume confirmation.
/// Uses ATR-based stops and time-based exits.
pub struct Breakout {
    name: String,
    lookback: usize,
    volume_multiplier: Decimal,
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    recent_highs: Vec<Decimal>,
    recent_lows: Vec<Decimal>,
    bars_since_signal: u32,
    cooldown: u32,
    allow_short: bool,
}

impl Breakout {
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
                .unwrap_or("breakout")
                .to_string(),
            lookback: get_u64("lookback", 20) as usize,
            volume_multiplier: get_str_dec("volume_multiplier", dec!(1.5)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(3)),
            recent_highs: Vec::new(),
            recent_lows: Vec::new(),
            bars_since_signal: 0,
            cooldown: get_u64("cooldown", 5) as u32,
            allow_short: params
                .get("allow_short")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }
}

impl Strategy for Breakout {
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

        // Track recent highs/lows
        self.recent_highs.push(candle.high);
        self.recent_lows.push(candle.low);
        if self.recent_highs.len() > self.lookback + 1 {
            self.recent_highs.remove(0);
            self.recent_lows.remove(0);
        }

        if self.recent_highs.len() <= self.lookback {
            return None;
        }

        let atr = features.atr_14?;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        // N-bar high/low (excluding current bar to avoid lookahead)
        let range_highs = &self.recent_highs[..self.lookback];
        let range_lows = &self.recent_lows[..self.lookback];
        let n_bar_high = range_highs.iter().max().copied()?;
        let n_bar_low = range_lows.iter().min().copied()?;

        let has_position = current_position.map(|p| !p.is_flat()).unwrap_or(false);
        let volume_ok = features
            .volume_ratio
            .map(|vr| vr >= self.volume_multiplier)
            .unwrap_or(false);

        if has_position {
            return None; // Exits handled by stop/target in execution layer
        }

        // Upside breakout
        if candle.close > n_bar_high && volume_ok {
            self.bars_since_signal = 0;
            let confidence = if volume_ok { dec!(0.7) } else { dec!(0.55) };
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Long,
                strength: dec!(1),
                confidence,
                entry_price: Some(candle.close),
                stop_loss: Some(candle.close - atr * self.atr_stop_multiplier),
                take_profit: Some(candle.close + atr * self.atr_target_multiplier),
                time_stop_bars: Some(30),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "n_bar_high": n_bar_high.to_string(),
                        "volume_ok": volume_ok,
                        "reason": "breakout_long",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("breakout".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        // Downside breakout
        if self.allow_short && candle.close < n_bar_low && volume_ok {
            self.bars_since_signal = 0;
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Short,
                strength: dec!(1),
                confidence: dec!(0.65),
                entry_price: Some(candle.close),
                stop_loss: Some(candle.close + atr * self.atr_stop_multiplier),
                take_profit: Some(candle.close - atr * self.atr_target_multiplier),
                time_stop_bars: Some(30),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "n_bar_low": n_bar_low.to_string(),
                        "reason": "breakout_short",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(0.35)),
                    regime: Some("breakdown".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        None
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("lookback".into(), serde_json::json!(self.lookback));
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.recent_highs.clear();
        self.recent_lows.clear();
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}
