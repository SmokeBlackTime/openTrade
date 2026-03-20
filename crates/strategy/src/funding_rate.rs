//! Funding rate mean reversion strategy.
//!
//! Exploits the tendency of extreme perpetual futures funding rates to revert.
//! When funding is extremely positive (longs paying shorts), it often precedes
//! a correction. When extremely negative, short squeeze potential.
//!
//! This is one of crypto's most reliable edges — longs paying 0.3%/day in
//! funding eventually get liquidated.

use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Funding rate mean reversion strategy for perpetual futures.
pub struct FundingRateReversion {
    name: String,
    /// Funding rate threshold for entry (e.g., 0.001 = 0.1%).
    entry_threshold: Decimal,
    /// Extreme threshold for high-conviction signals.
    extreme_threshold: Decimal,
    /// RSI filter to avoid fighting strong momentum.
    rsi_confirm_min: Decimal,
    rsi_confirm_max: Decimal,
    /// ATR multiplier for stop loss.
    atr_stop_multiplier: Decimal,
    /// ATR multiplier for take profit.
    atr_target_multiplier: Decimal,
    /// Maximum bars to hold.
    max_hold_bars: u32,
    /// Cooldown between signals.
    cooldown: u32,
    bars_since_signal: u32,
    /// Current funding rate (must be fed externally).
    current_funding_rate: Option<Decimal>,
}

impl FundingRateReversion {
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
                .unwrap_or("funding_rate_reversion")
                .to_string(),
            entry_threshold: get_str_dec("entry_threshold", dec!(0.0005)),    // 0.05%
            extreme_threshold: get_str_dec("extreme_threshold", dec!(0.001)), // 0.1%
            rsi_confirm_min: get_str_dec("rsi_confirm_min", dec!(30)),
            rsi_confirm_max: get_str_dec("rsi_confirm_max", dec!(70)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(2.5)),
            max_hold_bars: get_u64("max_hold_bars", 24) as u32,
            cooldown: get_u64("cooldown", 8) as u32,
            bars_since_signal: 0,
            current_funding_rate: None,
        }
    }

    /// Update the current funding rate (called from market data layer).
    pub fn set_funding_rate(&mut self, rate: Decimal) {
        self.current_funding_rate = Some(rate);
    }
}

impl Strategy for FundingRateReversion {
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

        let funding = self.current_funding_rate?;
        let rsi = features.rsi_14?;
        let atr = features.atr_14?;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        if has_position {
            return None;
        }

        let is_extreme = funding.abs() > self.extreme_threshold;
        let is_elevated = funding.abs() > self.entry_threshold;

        if !is_elevated {
            return None;
        }

        // Funding very positive → longs paying → expect correction → go short
        // Funding very negative → shorts paying → expect squeeze → go long
        if funding > self.entry_threshold && rsi < self.rsi_confirm_max {
            // Positive funding + RSI not oversold = short opportunity
            let confidence = if is_extreme { dec!(0.8) } else { dec!(0.65) };
            let stop = candle.close + atr * self.atr_stop_multiplier;
            let target = candle.close - atr * self.atr_target_multiplier;

            self.bars_since_signal = 0;
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Short,
                strength: if is_extreme { dec!(1) } else { dec!(0.7) },
                confidence,
                entry_price: Some(candle.close),
                stop_loss: Some(stop),
                take_profit: Some(target),
                time_stop_bars: Some(self.max_hold_bars),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "funding_rate": funding.to_string(),
                        "rsi": rsi.to_string(),
                        "is_extreme": is_extreme,
                        "reason": "funding_rate_short_reversion",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("funding_extreme_positive".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        if funding < -self.entry_threshold && rsi > self.rsi_confirm_min {
            // Negative funding + RSI not overbought = long opportunity
            let confidence = if is_extreme { dec!(0.8) } else { dec!(0.65) };
            let stop = candle.close - atr * self.atr_stop_multiplier;
            let target = candle.close + atr * self.atr_target_multiplier;

            self.bars_since_signal = 0;
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Long,
                strength: if is_extreme { dec!(1) } else { dec!(0.7) },
                confidence,
                entry_price: Some(candle.close),
                stop_loss: Some(stop),
                take_profit: Some(target),
                time_stop_bars: Some(self.max_hold_bars),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "funding_rate": funding.to_string(),
                        "rsi": rsi.to_string(),
                        "is_extreme": is_extreme,
                        "reason": "funding_rate_long_squeeze",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("funding_extreme_negative".into()),
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
            "entry_threshold".into(),
            serde_json::json!(self.entry_threshold.to_string()),
        );
        m.insert(
            "extreme_threshold".into(),
            serde_json::json!(self.extreme_threshold.to_string()),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.current_funding_rate = None;
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
        let s = FundingRateReversion::new(&params);
        assert_eq!(s.name(), "funding_rate_reversion");
    }
}
