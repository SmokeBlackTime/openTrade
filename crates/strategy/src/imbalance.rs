//! Order book imbalance strategy.
//!
//! Uses the bid/ask imbalance as an entry filter and signal source.
//! Persistent imbalance in one direction is a strong short-term predictor.
//! Combined with price action for confirmation.

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

/// Order book imbalance-driven strategy.
pub struct ImbalanceStrategy {
    name: String,
    /// Minimum imbalance ratio to consider (-1 to 1).
    min_imbalance: Decimal,
    /// Number of bars of persistent imbalance required.
    persistence_bars: usize,
    /// Volume confirmation multiplier.
    volume_confirm: Decimal,
    /// ATR multiplier for stop.
    atr_stop_multiplier: Decimal,
    /// ATR multiplier for target.
    atr_target_multiplier: Decimal,
    cooldown: u32,
    bars_since_signal: u32,
    /// Rolling window of imbalance values.
    imbalance_history: VecDeque<Decimal>,
    /// Current order book imbalance (fed externally from TopOfBook).
    current_imbalance: Option<Decimal>,
}

impl ImbalanceStrategy {
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
                .unwrap_or("imbalance")
                .to_string(),
            min_imbalance: get_str_dec("min_imbalance", dec!(0.2)),
            persistence_bars: get_u64("persistence_bars", 3) as usize,
            volume_confirm: get_str_dec("volume_confirm", dec!(1.2)),
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(1.5)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(2)),
            cooldown: get_u64("cooldown", 3) as u32,
            bars_since_signal: 0,
            imbalance_history: VecDeque::with_capacity(20),
            current_imbalance: None,
        }
    }

    /// Update current order book imbalance (called from market data layer).
    /// imbalance = (bid_qty - ask_qty) / (bid_qty + ask_qty), range [-1, 1].
    pub fn set_imbalance(&mut self, imbalance: Decimal) {
        self.current_imbalance = Some(imbalance);
        self.imbalance_history.push_back(imbalance);
        if self.imbalance_history.len() > 20 {
            self.imbalance_history.pop_front();
        }
    }

    /// Check if imbalance has been persistently in one direction.
    fn persistent_imbalance(&self) -> Option<SignalDirection> {
        if self.imbalance_history.len() < self.persistence_bars {
            return None;
        }

        let recent: Vec<&Decimal> = self
            .imbalance_history
            .iter()
            .rev()
            .take(self.persistence_bars)
            .collect();

        let all_bid_heavy = recent.iter().all(|&&v| v > self.min_imbalance);
        let all_ask_heavy = recent.iter().all(|&&v| v < -self.min_imbalance);

        if all_bid_heavy {
            Some(SignalDirection::Long)
        } else if all_ask_heavy {
            Some(SignalDirection::Short)
        } else {
            None
        }
    }
}

impl Strategy for ImbalanceStrategy {
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

        let imbalance = self.current_imbalance?;
        let atr = features.atr_14?;
        let volume_ratio = features.volume_ratio.unwrap_or(dec!(1));

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        if has_position {
            return None;
        }

        // Need persistent imbalance + volume confirmation
        let direction = self.persistent_imbalance()?;

        if volume_ratio < self.volume_confirm {
            return None; // Not enough volume to trust the imbalance
        }

        // Confidence scales with imbalance magnitude
        let imbalance_magnitude = imbalance.abs();
        let base_confidence = dec!(0.5) + imbalance_magnitude * dec!(0.3);
        let confidence = base_confidence.min(dec!(0.85));

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
            time_stop_bars: Some(10),
            metadata: SignalMetadata {
                signal_inputs: serde_json::json!({
                    "imbalance": imbalance.to_string(),
                    "persistence_bars": self.persistence_bars,
                    "volume_ratio": volume_ratio.to_string(),
                    "reason": "persistent_orderbook_imbalance",
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
            "min_imbalance".into(),
            serde_json::json!(self.min_imbalance.to_string()),
        );
        m.insert(
            "persistence_bars".into(),
            serde_json::json!(self.persistence_bars),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.imbalance_history.clear();
        self.current_imbalance = None;
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_imbalance_detection() {
        let params = HashMap::new();
        let mut s = ImbalanceStrategy::new(&params);

        // Feed persistent bid-heavy imbalance
        s.set_imbalance(dec!(0.3));
        s.set_imbalance(dec!(0.35));
        s.set_imbalance(dec!(0.4));

        let dir = s.persistent_imbalance();
        assert_eq!(dir, Some(SignalDirection::Long));
    }

    #[test]
    fn no_imbalance_when_mixed() {
        let params = HashMap::new();
        let mut s = ImbalanceStrategy::new(&params);

        s.set_imbalance(dec!(0.3));
        s.set_imbalance(dec!(-0.1));
        s.set_imbalance(dec!(0.2));

        let dir = s.persistent_imbalance();
        assert!(dir.is_none());
    }
}
