//! Strategy engine for OpenTrade.
//!
//! Provides a pluggable strategy trait and implementations for:
//! - Trend following
//! - Mean reversion
//! - Breakout
//! - Momentum
//! - Meta-strategy (regime-based allocation)

pub mod breakout;
pub mod mean_reversion;
pub mod meta;
pub mod momentum;
pub mod trend;

use ot_features::FeatureRow;
use ot_types::market::Candle;
use ot_types::positions::Position;
use ot_types::signals::Signal;
use std::collections::HashMap;

/// Core strategy trait. All strategies implement this interface.
pub trait Strategy: Send + Sync {
    /// Human-readable name.
    fn name(&self) -> &str;

    /// Process a new completed candle + features and optionally generate a signal.
    fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_position: Option<&Position>,
    ) -> Option<Signal>;

    /// Strategy-specific parameters for serialization.
    fn params(&self) -> HashMap<String, serde_json::Value>;

    /// Reset internal state (for backtesting).
    fn reset(&mut self);

    /// Cooldown: minimum bars between trades.
    fn cooldown_bars(&self) -> u32 {
        1
    }
}

/// Registry of available strategies.
pub struct StrategyRegistry {
    strategies: HashMap<String, Box<dyn Strategy>>,
}

impl StrategyRegistry {
    pub fn new() -> Self {
        Self {
            strategies: HashMap::new(),
        }
    }

    pub fn register(&mut self, strategy: Box<dyn Strategy>) {
        let name = strategy.name().to_string();
        self.strategies.insert(name, strategy);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Strategy> {
        self.strategies.get(name).map(|s| s.as_ref())
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn Strategy>> {
        self.strategies.get_mut(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.strategies.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for StrategyRegistry {
    fn default() -> Self {
        Self::new()
    }
}
