use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::Signal;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;

use crate::Strategy;

/// Regime detection categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketRegime {
    Trending,
    Ranging,
    HighVolatility,
    LowVolatility,
}

impl MarketRegime {
    /// Detect regime from features.
    pub fn detect(features: &FeatureRow) -> Self {
        let trend_strong = features
            .trend_strength
            .map(|ts| ts.abs() > dec!(1))
            .unwrap_or(false);

        let high_vol = features
            .realized_vol_20
            .map(|v| v > dec!(60))
            .unwrap_or(false);

        let bb_narrow = features.bb_width.map(|w| w < dec!(0.02)).unwrap_or(false);

        if high_vol {
            MarketRegime::HighVolatility
        } else if trend_strong {
            MarketRegime::Trending
        } else if bb_narrow {
            MarketRegime::LowVolatility
        } else {
            MarketRegime::Ranging
        }
    }
}

/// Meta-strategy that allocates among sub-strategies based on regime.
pub struct MetaStrategy {
    name: String,
    strategies: Vec<Box<dyn Strategy>>,
    /// Allocation weights per regime per strategy index.
    regime_weights: HashMap<String, Vec<Decimal>>,
}

impl MetaStrategy {
    pub fn new(
        name: String,
        strategies: Vec<Box<dyn Strategy>>,
    ) -> Self {
        let n = strategies.len();
        let equal_weight = if n > 0 {
            dec!(1) / Decimal::from(n as u32)
        } else {
            dec!(0)
        };
        let equal_weights = vec![equal_weight; n];

        let mut regime_weights = HashMap::new();
        // Default: equal weighting for all regimes
        regime_weights.insert("trending".into(), equal_weights.clone());
        regime_weights.insert("ranging".into(), equal_weights.clone());
        regime_weights.insert("high_volatility".into(), equal_weights.clone());
        regime_weights.insert("low_volatility".into(), equal_weights);

        Self {
            name,
            strategies,
            regime_weights,
        }
    }

    /// Set custom weights for a regime. Weights must sum to 1.
    pub fn set_regime_weights(&mut self, regime: &str, weights: Vec<Decimal>) {
        if weights.len() == self.strategies.len() {
            self.regime_weights.insert(regime.to_string(), weights);
        }
    }

    /// Get the current regime key string.
    fn regime_key(regime: MarketRegime) -> &'static str {
        match regime {
            MarketRegime::Trending => "trending",
            MarketRegime::Ranging => "ranging",
            MarketRegime::HighVolatility => "high_volatility",
            MarketRegime::LowVolatility => "low_volatility",
        }
    }
}

impl Strategy for MetaStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_position: Option<&Position>,
    ) -> Option<Signal> {
        let regime = MarketRegime::detect(features);
        let key = Self::regime_key(regime);
        let weights = self.regime_weights.get(key)?;

        // Collect signals from all sub-strategies
        let mut best_signal: Option<(Signal, Decimal)> = None;

        for (i, strategy) in self.strategies.iter_mut().enumerate() {
            let weight = weights.get(i).copied().unwrap_or(dec!(0));
            if weight <= dec!(0) {
                continue;
            }

            if let Some(signal) = strategy.on_bar(candle, features, current_position) {
                let weighted_confidence = signal.confidence * weight;
                match &best_signal {
                    Some((_, best_score)) if weighted_confidence <= *best_score => {}
                    _ => {
                        best_signal = Some((signal, weighted_confidence));
                    }
                }
            }
        }

        best_signal.map(|(mut signal, _)| {
            signal.metadata.regime = Some(format!("{:?}", regime));
            signal
        })
    }

    fn params(&self) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        let strategy_names: Vec<&str> = self.strategies.iter().map(|s| s.name()).collect();
        m.insert("sub_strategies".into(), serde_json::json!(strategy_names));
        m
    }

    fn reset(&mut self) {
        for s in &mut self.strategies {
            s.reset();
        }
    }
}
