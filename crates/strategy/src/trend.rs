use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::{Position, PositionSide};
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Trend-following strategy using dual moving average crossover with filters.
///
/// Entry long: fast SMA > slow SMA AND price > fast SMA AND RSI < overbought
/// Entry short: fast SMA < slow SMA AND price < fast SMA AND RSI > oversold
/// Exit: SMA crossover reversal or stop/target hit
pub struct TrendFollowing {
    name: String,
    fast_period: usize,
    slow_period: usize,
    rsi_overbought: Decimal,
    rsi_oversold: Decimal,
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    min_trend_strength: Decimal,
    bars_since_signal: u32,
    cooldown: u32,
    allow_short: bool,
}

impl TrendFollowing {
    pub fn new(params: &HashMap<String, serde_json::Value>) -> Self {
        let get_u64 = |key: &str, default: u64| -> u64 {
            params
                .get(key)
                .and_then(|v| v.as_u64())
                .unwrap_or(default)
        };
        let get_str_dec = |key: &str, default: Decimal| -> Decimal {
            params
                .get(key)
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(default)
        };
        let get_bool = |key: &str, default: bool| -> bool {
            params
                .get(key)
                .and_then(|v| v.as_bool())
                .unwrap_or(default)
        };

        Self {
            name: params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("trend_following")
                .to_string(),
            fast_period: get_u64("fast_period", 20) as usize,
            slow_period: get_u64("slow_period", 50) as usize,
            rsi_overbought: get_str_dec("rsi_overbought", dec!(70)),
            rsi_oversold: get_str_dec("rsi_oversold", dec!(30)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(2)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(3)),
            min_trend_strength: get_str_dec("min_trend_strength", dec!(0.5)),
            bars_since_signal: 0,
            cooldown: get_u64("cooldown", 5) as u32,
            allow_short: get_bool("allow_short", false),
        }
    }

    fn compute_confidence(&self, features: &FeatureRow, is_long: bool) -> Decimal {
        let mut score = dec!(0.40); // lowered base from 0.50

        // RSI confirmation
        if let Some(rsi) = features.rsi_14 {
            if (is_long && rsi < dec!(60) && rsi > dec!(40))
                || (!is_long && rsi > dec!(40) && rsi < dec!(60))
            {
                // Neutral RSI — no boost
            } else if (is_long && rsi < dec!(70)) || (!is_long && rsi > dec!(30)) {
                score += dec!(0.10);
            }
        }

        // Trend strength in direction
        if let Some(ts) = features.trend_strength {
            let directional_ts = if is_long { ts } else { -ts };
            if directional_ts > self.min_trend_strength {
                score += dec!(0.15);
            }
        }

        // Volume confirmation
        if let Some(vr) = features.volume_ratio {
            if vr > dec!(1.2) {
                score += dec!(0.10);
            }
        }

        // MACD histogram direction (new)
        if let Some(hist) = features.macd_histogram {
            let confirming = if is_long { hist > dec!(0) } else { hist < dec!(0) };
            if confirming {
                score += dec!(0.15);
            } else {
                score -= dec!(0.10); // penalize counter-direction histogram
            }
        }

        // BB squeeze (low vol → potential breakout)
        if let Some(bbw) = features.bb_width {
            if bbw < dec!(0.02) {
                score += dec!(0.05);
            }
        }

        score.max(dec!(0)).min(dec!(0.95))
    }
}

impl Strategy for TrendFollowing {
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

        let sma_fast = features.sma_20?;
        let sma_slow = features.sma_50?;
        let rsi = features.rsi_14?;
        let atr = features.atr_14?;

        // Check cooldown
        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let is_uptrend = sma_fast > sma_slow;
        let price_above_fast = candle.close > sma_fast;
        let price_below_fast = candle.close < sma_fast;

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);
        let position_side = current_position
            .map(|p| p.side)
            .unwrap_or(PositionSide::Flat);

        // Exit signals
        if has_position {
            match position_side {
                PositionSide::Long if !is_uptrend => {
                    self.bars_since_signal = 0;
                    return Some(Signal {
                        strategy_name: self.name.clone(),
                        symbol: candle.symbol.clone(),
                        market_type: candle.market_type,
                        timeframe: candle.timeframe,
                        timestamp: Utc::now(),
                        direction: SignalDirection::Flat,
                        strength: dec!(0.8),
                        confidence: dec!(0.7),
                        entry_price: Some(candle.close),
                        stop_loss: None,
                        take_profit: None,
                        time_stop_bars: None,
                        metadata: SignalMetadata {
                            signal_inputs: serde_json::json!({
                                "sma_fast": sma_fast.to_string(),
                                "sma_slow": sma_slow.to_string(),
                                "rsi": rsi.to_string(),
                                "reason": "trend_reversal_exit",
                            }),
                            model_outputs: None,
                            uncertainty_score: None,
                            regime: None,
                            risk_overrides: vec![],
                            portfolio_context: None,
                        },
                    });
                }
                PositionSide::Short if is_uptrend => {
                    self.bars_since_signal = 0;
                    return Some(Signal {
                        strategy_name: self.name.clone(),
                        symbol: candle.symbol.clone(),
                        market_type: candle.market_type,
                        timeframe: candle.timeframe,
                        timestamp: Utc::now(),
                        direction: SignalDirection::Flat,
                        strength: dec!(0.8),
                        confidence: dec!(0.7),
                        entry_price: Some(candle.close),
                        stop_loss: None,
                        take_profit: None,
                        time_stop_bars: None,
                        metadata: SignalMetadata {
                            signal_inputs: serde_json::json!({
                                "reason": "trend_reversal_exit_short",
                            }),
                            model_outputs: None,
                            uncertainty_score: None,
                            regime: None,
                            risk_overrides: vec![],
                            portfolio_context: None,
                        },
                    });
                }
                _ => return None,
            }
        }

        // Long entry
        if is_uptrend && price_above_fast && rsi < self.rsi_overbought && !has_position {
            let confidence = self.compute_confidence(features, true);
            self.bars_since_signal = 0;
            let stop = candle.close - atr * self.atr_stop_multiplier;
            let target = candle.close + atr * self.atr_target_multiplier;
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
                stop_loss: Some(stop),
                take_profit: Some(target),
                time_stop_bars: Some(50),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "sma_fast": sma_fast.to_string(),
                        "sma_slow": sma_slow.to_string(),
                        "rsi": rsi.to_string(),
                        "atr": atr.to_string(),
                        "reason": "trend_long_entry",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("trending_up".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        // Short entry
        if self.allow_short
            && !is_uptrend
            && price_below_fast
            && rsi > self.rsi_oversold
            && !has_position
        {
            let confidence = self.compute_confidence(features, false);
            self.bars_since_signal = 0;
            let stop = candle.close + atr * self.atr_stop_multiplier;
            let target = candle.close - atr * self.atr_target_multiplier;
            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: SignalDirection::Short,
                strength: dec!(1),
                confidence,
                entry_price: Some(candle.close),
                stop_loss: Some(stop),
                take_profit: Some(target),
                time_stop_bars: Some(50),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "sma_fast": sma_fast.to_string(),
                        "sma_slow": sma_slow.to_string(),
                        "rsi": rsi.to_string(),
                        "atr": atr.to_string(),
                        "reason": "trend_short_entry",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(1) - confidence),
                    regime: Some("trending_down".into()),
                    risk_overrides: vec![],
                    portfolio_context: None,
                },
            });
        }

        None
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("fast_period".into(), serde_json::json!(self.fast_period));
        m.insert("slow_period".into(), serde_json::json!(self.slow_period));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_construction() {
        let params = HashMap::new();
        let s = TrendFollowing::new(&params);
        assert_eq!(s.name(), "trend_following");
        assert_eq!(s.fast_period, 20);
        assert_eq!(s.slow_period, 50);
    }

    #[test]
    fn confidence_macd_histogram_bonus() {
        use ot_features::pipeline::FeatureRow;
        use rust_decimal_macros::dec;
        let params = std::collections::HashMap::new();
        let s = TrendFollowing::new(&params);

        // Build a minimal FeatureRow
        let base_row = FeatureRow {
            timestamp_ms: 0,
            close: dec!(50000),
            return_1: None, return_5: None, log_return_1: None,
            sma_20: None, sma_50: None, ema_12: None, ema_26: None,
            macd: None, macd_signal_line: None,
            rsi_14: None, atr_14: None,
            bb_upper: None, bb_middle: None, bb_lower: None,
            realized_vol_20: None, bb_width: None,
            price_vs_sma20: None, price_vs_sma50: None,
            trend_strength: None, volume_sma_20: None, volume_ratio: None,
            funding_rate: None,
            macd_histogram: Some(dec!(50)), // positive = bullish
        };

        let conf_confirming = s.compute_confidence(&base_row, true);

        let mut opposing_row = base_row.clone();
        opposing_row.macd_histogram = Some(dec!(-50)); // negative = bearish
        let conf_opposing = s.compute_confidence(&opposing_row, true);

        assert!(
            conf_confirming > conf_opposing,
            "Confirming histogram ({}) should give higher confidence than opposing ({})",
            conf_confirming, conf_opposing
        );
    }

    #[test]
    fn confidence_base_score_is_lower() {
        use ot_features::pipeline::FeatureRow;
        use rust_decimal_macros::dec;
        let params = std::collections::HashMap::new();
        let s = TrendFollowing::new(&params);

        // With no features at all, base score should be 0.40
        let empty_row = FeatureRow {
            timestamp_ms: 0, close: dec!(50000),
            return_1: None, return_5: None, log_return_1: None,
            sma_20: None, sma_50: None, ema_12: None, ema_26: None,
            macd: None, macd_signal_line: None, rsi_14: None, atr_14: None,
            bb_upper: None, bb_middle: None, bb_lower: None,
            realized_vol_20: None, bb_width: None,
            price_vs_sma20: None, price_vs_sma50: None,
            trend_strength: None, volume_sma_20: None, volume_ratio: None,
            funding_rate: None, macd_histogram: None,
        };

        let conf = s.compute_confidence(&empty_row, true);
        assert_eq!(conf, dec!(0.40), "Base score with no features should be 0.40");
    }
}
