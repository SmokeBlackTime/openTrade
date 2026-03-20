//! Self-referential risk signal strategy.
//!
//! Uses the system's own risk metrics as trading signals.
//! When the risk engine detects anomalies (volatility spikes,
//! order rejections, stale data), these are market microstructure
//! signals that most systems ignore.

use chrono::Utc;
use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::{Position, PositionSide};
use ot_types::signals::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::collections::VecDeque;

use crate::Strategy;

/// Risk events tracked as signals.
#[derive(Debug, Clone)]
pub struct RiskEvent {
    pub event_type: RiskEventType,
    pub severity: Decimal, // 0-1
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskEventType {
    /// Volatility spike detected.
    VolatilitySpike,
    /// Order rejection rate increasing.
    OrderRejections,
    /// Market data staleness.
    StaleData,
    /// Spread widening significantly.
    SpreadWidening,
    /// Exchange connectivity issues.
    ConnectivityIssue,
}

/// Self-referential risk signal strategy.
pub struct RiskSignalStrategy {
    name: String,
    /// Volatility spike threshold (multiplier of recent realized vol).
    vol_spike_threshold: Decimal,
    /// How many risk events trigger a defensive signal.
    risk_event_threshold: usize,
    /// Lookback window for risk events (in bars).
    risk_event_lookback: usize,
    /// ATR multiplier for protective stops.
    atr_stop_multiplier: Decimal,
    atr_target_multiplier: Decimal,
    cooldown: u32,
    bars_since_signal: u32,
    /// Recent risk events.
    risk_events: VecDeque<RiskEvent>,
    /// Historical volatility for spike detection.
    vol_history: VecDeque<Decimal>,
}

impl RiskSignalStrategy {
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
                .unwrap_or("risk_signal")
                .to_string(),
            vol_spike_threshold: get_str_dec("vol_spike_threshold", dec!(2)),
            risk_event_threshold: get_u64("risk_event_threshold", 2) as usize,
            risk_event_lookback: get_u64("risk_event_lookback", 5) as usize,
            atr_stop_multiplier: get_str_dec("atr_stop_multiplier", dec!(1.5)),
            atr_target_multiplier: get_str_dec("atr_target_multiplier", dec!(2.5)),
            cooldown: get_u64("cooldown", 5) as u32,
            bars_since_signal: 0,
            risk_events: VecDeque::with_capacity(100),
            vol_history: VecDeque::with_capacity(50),
        }
    }

    /// Report a risk event (called from risk engine or market data layer).
    pub fn report_event(&mut self, event: RiskEvent) {
        self.risk_events.push_back(event);
        if self.risk_events.len() > 100 {
            self.risk_events.pop_front();
        }
    }

    /// Count recent risk events within the lookback window.
    fn recent_event_count(&self, bar_timestamp_ms: i64) -> usize {
        // Count events in the last N lookback periods (approximate by timestamp)
        let lookback_ms = self.risk_event_lookback as i64 * 60_000; // Approximate: 1 bar = 1 min
        self.risk_events
            .iter()
            .filter(|e| e.timestamp_ms > bar_timestamp_ms - lookback_ms)
            .count()
    }

    /// Detect a volatility spike (current vol >> recent average).
    fn detect_vol_spike(&self, current_vol: Decimal) -> bool {
        if self.vol_history.len() < 5 {
            return false;
        }

        let sum: Decimal = self.vol_history.iter().sum();
        let avg = sum / Decimal::from(self.vol_history.len() as u32);

        if avg > Decimal::ZERO {
            current_vol / avg > self.vol_spike_threshold
        } else {
            false
        }
    }
}

impl Strategy for RiskSignalStrategy {
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

        // Track volatility history
        if let Some(vol) = features.realized_vol_20 {
            self.vol_history.push_back(vol);
            if self.vol_history.len() > 50 {
                self.vol_history.pop_front();
            }
        }

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        let atr = features.atr_14?;
        let current_vol = features.realized_vol_20.unwrap_or(dec!(30));
        let timestamp_ms = features.timestamp_ms;

        // Check for volatility spike
        let vol_spike = self.detect_vol_spike(current_vol);
        if vol_spike {
            self.report_event(RiskEvent {
                event_type: RiskEventType::VolatilitySpike,
                severity: dec!(0.8),
                timestamp_ms,
            });
        }

        // Count recent risk events
        let event_count = self.recent_event_count(timestamp_ms);

        if event_count < self.risk_event_threshold {
            return None; // Not enough risk signals
        }

        let has_position = current_position
            .map(|p| !p.is_flat())
            .unwrap_or(false);

        // If we have a position and risk events spike → reduce/close position
        if has_position {
            let position_side = current_position
                .map(|p| p.side)
                .unwrap_or(PositionSide::Flat);

            // Close position in the direction of risk
            let exit_direction = match position_side {
                PositionSide::Long => SignalDirection::Flat,
                PositionSide::Short => SignalDirection::Flat,
                PositionSide::Flat => return None,
            };

            self.bars_since_signal = 0;

            return Some(Signal {
                strategy_name: self.name.clone(),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction: exit_direction,
                strength: dec!(0.9),
                confidence: dec!(0.8),
                entry_price: Some(candle.close),
                stop_loss: None,
                take_profit: None,
                time_stop_bars: None,
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "risk_event_count": event_count,
                        "vol_spike": vol_spike,
                        "current_vol": current_vol.to_string(),
                        "reason": "risk_signal_exit",
                    }),
                    model_outputs: None,
                    uncertainty_score: Some(dec!(0.2)),
                    regime: Some("risk_elevated".into()),
                    risk_overrides: vec![format!("{} risk events detected", event_count)],
                    portfolio_context: None,
                },
            });
        }

        // If no position but risk events spike → trade the volatility
        // Vol spikes often precede sharp moves; enter short as volatility expansion
        // typically favors downside in crypto
        if vol_spike {
            let rsi = features.rsi_14.unwrap_or(dec!(50));

            // Only short if RSI is elevated (confirming potential reversal)
            if rsi > dec!(55) {
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
                    strength: dec!(0.7),
                    confidence: dec!(0.6),
                    entry_price: Some(candle.close),
                    stop_loss: Some(stop),
                    take_profit: Some(target),
                    time_stop_bars: Some(10),
                    metadata: SignalMetadata {
                        signal_inputs: serde_json::json!({
                            "risk_event_count": event_count,
                            "vol_spike": true,
                            "current_vol": current_vol.to_string(),
                            "rsi": rsi.to_string(),
                            "reason": "risk_signal_vol_spike_short",
                        }),
                        model_outputs: None,
                        uncertainty_score: Some(dec!(0.4)),
                        regime: Some("high_volatility".into()),
                        risk_overrides: vec![],
                        portfolio_context: None,
                    },
                });
            }
        }

        None
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert(
            "vol_spike_threshold".into(),
            serde_json::json!(self.vol_spike_threshold.to_string()),
        );
        m.insert(
            "risk_event_threshold".into(),
            serde_json::json!(self.risk_event_threshold),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.risk_events.clear();
        self.vol_history.clear();
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
        let s = RiskSignalStrategy::new(&HashMap::new());
        assert_eq!(s.name(), "risk_signal");
    }

    #[test]
    fn vol_spike_detection() {
        let mut s = RiskSignalStrategy::new(&HashMap::new());

        // Build normal vol history
        for _ in 0..10 {
            s.vol_history.push_back(dec!(25));
        }

        // Normal vol: no spike
        assert!(!s.detect_vol_spike(dec!(30)));

        // Spike: current vol is 3x average
        assert!(s.detect_vol_spike(dec!(75)));
    }
}
