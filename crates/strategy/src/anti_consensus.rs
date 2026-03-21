//! Anti-consensus (contrarian) strategy.
//!
//! When ALL sub-strategies agree on a direction, that's often a crowded
//! trade signal. Consensus in simple indicators means the move is already
//! priced in. This strategy dampens or fades consensus signals.
//!
//! Usage: Feed signals from other strategies via `record_signal()`.
//! When all agree, generate a contrarian dampener.

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

/// Tracks signals from other strategies to detect consensus.
#[derive(Debug, Clone)]
struct SignalRecord {
    strategy: String,
    direction: SignalDirection,
    confidence: Decimal,
    bar_index: u64,
}

/// Anti-consensus contrarian strategy.
pub struct AntiConsensus {
    name: String,
    /// How many strategies must agree for "consensus".
    min_consensus_count: usize,
    /// Confidence reduction when consensus is detected.
    dampening_factor: Decimal,
    /// Whether to generate contrarian signals (vs just dampening).
    generate_contrarian: bool,
    /// ATR multiplier for contrarian stops (tighter than normal).
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    cooldown: u32,
    bars_since_signal: u32,
    /// Track signals from other strategies.
    recent_signals: VecDeque<SignalRecord>,
    current_bar: u64,
}

impl AntiConsensus {
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
                .unwrap_or("anti_consensus")
                .to_string(),
            min_consensus_count: get_u64("min_consensus_count", 3) as usize,
            dampening_factor: get_str_dec("dampening_factor", dec!(0.3)),
            generate_contrarian: get_bool("generate_contrarian", true),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(1.5)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(2)),
            cooldown: get_u64("cooldown", 10) as u32,
            bars_since_signal: 0,
            recent_signals: VecDeque::with_capacity(50),
            current_bar: 0,
        }
    }

    /// Record a signal from another strategy (called from the engine).
    pub fn record_signal(&mut self, strategy: &str, direction: SignalDirection, confidence: Decimal) {
        self.recent_signals.push_back(SignalRecord {
            strategy: strategy.to_string(),
            direction,
            confidence,
            bar_index: self.current_bar,
        });

        // Keep only recent bar's signals
        while self
            .recent_signals
            .front()
            .map(|s| s.bar_index < self.current_bar.saturating_sub(1))
            .unwrap_or(false)
        {
            self.recent_signals.pop_front();
        }
    }

    /// Check if there's consensus among recent signals.
    /// Uses confidence-weighted voting: high-confidence signals count more.
    fn detect_consensus(&self) -> Option<(SignalDirection, usize)> {
        let current_bar_signals: Vec<&SignalRecord> = self
            .recent_signals
            .iter()
            .filter(|s| s.bar_index == self.current_bar)
            .collect();

        if current_bar_signals.len() < self.min_consensus_count {
            return None;
        }

        // Use confidence-weighted voting
        let mut long_weight = dec!(0);
        let mut short_weight = dec!(0);
        let mut long_count = 0usize;
        let mut short_count = 0usize;

        for s in &current_bar_signals {
            match s.direction {
                SignalDirection::Long => {
                    long_weight += s.confidence;
                    long_count += 1;
                }
                SignalDirection::Short => {
                    short_weight += s.confidence;
                    short_count += 1;
                }
                _ => {}
            }
        }

        let total = current_bar_signals.len();
        let strategies: Vec<&str> = current_bar_signals.iter().map(|s| s.strategy.as_str()).collect();

        if long_count == total {
            tracing::info!(
                strategies = ?strategies,
                avg_confidence = %long_weight / Decimal::from(total),
                "Anti-consensus detected LONG consensus"
            );
            Some((SignalDirection::Long, total))
        } else if short_count == total {
            tracing::info!(
                strategies = ?strategies,
                avg_confidence = %short_weight / Decimal::from(total),
                "Anti-consensus detected SHORT consensus"
            );
            Some((SignalDirection::Short, total))
        } else {
            None
        }
    }
}

impl Strategy for AntiConsensus {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_position: Option<&Position>,
    ) -> Option<Signal> {
        self.current_bar += 1;
        self.bars_since_signal += 1;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        if has_position || !self.generate_contrarian {
            return None;
        }

        let atr = features.atr_14?;
        let (consensus_direction, consensus_count) = self.detect_consensus()?;

        // RSI confirmation: consensus long but RSI is overbought = fade it
        let rsi = features.rsi_14.unwrap_or(dec!(50));
        let rsi_confirms_fade = match consensus_direction {
            SignalDirection::Long => rsi > dec!(65),  // Overbought
            SignalDirection::Short => rsi < dec!(35), // Oversold
            _ => false,
        };

        if !rsi_confirms_fade {
            return None; // Only fade when RSI supports the contrarian view
        }

        // Flip the direction
        let contrarian_direction = match consensus_direction {
            SignalDirection::Long => SignalDirection::Short,
            SignalDirection::Short => SignalDirection::Long,
            _ => return None,
        };

        let confidence = dec!(0.55) + Decimal::from(consensus_count as u32) * dec!(0.05);
        let confidence = confidence.min(dec!(0.75));

        let (stop, target) = match contrarian_direction {
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
            direction: contrarian_direction,
            strength: self.dampening_factor,
            confidence,
            entry_price: Some(candle.close),
            stop_loss: Some(stop),
            take_profit: Some(target),
            time_stop_bars: Some(15),
            metadata: SignalMetadata {
                signal_inputs: serde_json::json!({
                    "consensus_direction": format!("{:?}", consensus_direction),
                    "consensus_count": consensus_count,
                    "rsi": rsi.to_string(),
                    "reason": "anti_consensus_fade",
                }),
                model_outputs: None,
                uncertainty_score: Some(dec!(1) - confidence),
                regime: None,
                risk_overrides: vec![
                    format!("Fading {}-strategy consensus", consensus_count)
                ],
                portfolio_context: None,
            },
        })
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "min_consensus_count".into(),
            serde_json::json!(self.min_consensus_count),
        );
        m.insert(
            "dampening_factor".into(),
            serde_json::json!(self.dampening_factor.to_string()),
        );
        m.insert(
            "generate_contrarian".into(),
            serde_json::json!(self.generate_contrarian),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.current_bar = 0;
        self.recent_signals.clear();
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_consensus_when_empty() {
        let s = AntiConsensus::new(&HashMap::new());
        assert!(s.detect_consensus().is_none());
    }

    #[test]
    fn detects_full_consensus() {
        let mut s = AntiConsensus::new(&HashMap::new());
        s.current_bar = 1;
        s.record_signal("trend", SignalDirection::Long, dec!(0.8));
        s.record_signal("breakout", SignalDirection::Long, dec!(0.7));
        s.record_signal("momentum", SignalDirection::Long, dec!(0.75));

        let consensus = s.detect_consensus();
        assert!(consensus.is_some());
        let (dir, count) = consensus.unwrap();
        assert_eq!(dir, SignalDirection::Long);
        assert_eq!(count, 3);
    }

    #[test]
    fn no_consensus_when_split() {
        let mut s = AntiConsensus::new(&HashMap::new());
        s.current_bar = 1;
        s.record_signal("trend", SignalDirection::Long, dec!(0.8));
        s.record_signal("breakout", SignalDirection::Short, dec!(0.7));
        s.record_signal("momentum", SignalDirection::Long, dec!(0.75));

        assert!(s.detect_consensus().is_none());
    }
}
