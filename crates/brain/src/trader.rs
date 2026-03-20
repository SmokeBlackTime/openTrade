//! Autonomous trader: the brain's execution interface.
//!
//! Wraps the TradingBrain as a Strategy implementation so it can
//! plug directly into the existing TradingEngine pipeline.

use crate::{BrainConfig, BrainDecision, TradingBrain};
use ot_features::FeatureRow;
use ot_models::regime::Regime;
use ot_strategy::Strategy;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::Signal;
use std::collections::HashMap;
use tokio::runtime::Handle;
use tracing::debug;

/// A Strategy adapter that wraps the TradingBrain.
///
/// This allows the AI brain to participate in the standard strategy
/// pipeline alongside rule-based strategies. The TradingEngine
/// treats it like any other strategy.
pub struct BrainStrategy {
    brain: TradingBrain,
    name: String,
    cooldown: u32,
    bars_since_signal: u32,
    last_decision: Option<BrainDecision>,
}

impl BrainStrategy {
    /// Create a new brain strategy.
    pub fn new(config: BrainConfig) -> Result<Self, ot_common::OtError> {
        let name = format!("brain_{}", config.personality.name());
        let cooldown = config.analysis_interval;
        let brain = TradingBrain::new(config)?;

        Ok(Self {
            brain,
            name,
            cooldown,
            bars_since_signal: 0,
            last_decision: None,
        })
    }

    /// Initialize the brain (run health checks).
    /// Must be called before first use in an async context.
    pub async fn initialize(&self) {
        self.brain.initialize().await;
    }

    /// Get the last decision for explainability.
    pub fn last_decision(&self) -> Option<&BrainDecision> {
        self.last_decision.as_ref()
    }

    /// Get brain stats.
    pub fn stats(&self) -> &crate::BrainStats {
        self.brain.stats()
    }

    /// Record a trade outcome for learning.
    pub fn learn_from_trade(
        &self,
        symbol: &str,
        direction: &str,
        pnl_pct: f64,
        regime: &str,
        strategy: &str,
        features: &serde_json::Value,
    ) {
        self.brain
            .learn_from_trade(symbol, direction, pnl_pct, regime, strategy, features);
    }
}

impl Strategy for BrainStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        _current_position: Option<&Position>,
    ) -> Option<Signal> {
        self.bars_since_signal += 1;

        if self.bars_since_signal < self.cooldown {
            return None;
        }

        // Detect current regime
        let regime = Regime::detect(features);
        let regime_str = format!("{:?}", regime);

        // Run the brain's analysis
        // We need to bridge async → sync since Strategy::on_bar is synchronous.
        // Use tokio::runtime::Handle to block on the async future.
        let decision = match Handle::try_current() {
            Ok(handle) => {
                // We're inside a tokio runtime, use block_in_place
                tokio::task::block_in_place(|| {
                    handle.block_on(self.brain.on_bar(candle, features, &regime_str))
                })
            }
            Err(_) => {
                // Not in a runtime, can't do async work
                debug!("No tokio runtime available for brain analysis");
                return None;
            }
        };

        self.last_decision = Some(decision.clone());

        if decision.signal.is_some() {
            self.bars_since_signal = 0;
        }

        decision.signal
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        m.insert("type".into(), serde_json::json!("neural_brain"));
        m.insert(
            "personality".into(),
            serde_json::json!(self.brain.stats()),
        );
        m
    }

    fn reset(&mut self) {
        self.bars_since_signal = 0;
        self.last_decision = None;
    }

    fn cooldown_bars(&self) -> u32 {
        self.cooldown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brain_strategy_creation() {
        let config = BrainConfig::default();
        // This may fail if no Ollama is running, which is expected in tests
        let result = BrainStrategy::new(config);
        assert!(result.is_ok());
    }
}
