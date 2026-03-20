use ot_features::FeatureRow;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

/// Market regime classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Regime {
    TrendingUp,
    TrendingDown,
    Ranging,
    HighVolatility,
    LowVolatility,
    Transitional,
}

impl Regime {
    /// Simple regime detector from feature row.
    ///
    /// Uses trend strength, volatility, and RSI to classify.
    /// This is a rule-based heuristic, not an HMM.
    pub fn detect(features: &FeatureRow) -> Self {
        let trend = features.trend_strength.unwrap_or(dec!(0));
        let vol = features.realized_vol_20.unwrap_or(dec!(30));
        let rsi = features.rsi_14.unwrap_or(dec!(50));
        let bb_width = features.bb_width.unwrap_or(dec!(0.03));

        // High volatility regime
        if vol > dec!(60) {
            return Regime::HighVolatility;
        }

        // Low volatility / squeeze
        if bb_width < dec!(0.015) && vol < dec!(15) {
            return Regime::LowVolatility;
        }

        // Strong trend
        if trend > dec!(2) && rsi > dec!(55) {
            return Regime::TrendingUp;
        }
        if trend < dec!(-2) && rsi < dec!(45) {
            return Regime::TrendingDown;
        }

        // Moderate trend
        if trend.abs() > dec!(0.5) {
            return Regime::Transitional;
        }

        Regime::Ranging
    }

    /// Whether this regime favors trend-following strategies.
    pub fn favors_trend(self) -> bool {
        matches!(self, Regime::TrendingUp | Regime::TrendingDown)
    }

    /// Whether this regime favors mean-reversion strategies.
    pub fn favors_mean_reversion(self) -> bool {
        matches!(self, Regime::Ranging | Regime::LowVolatility)
    }

    /// Whether this regime warrants reduced exposure.
    pub fn reduce_exposure(self) -> bool {
        matches!(self, Regime::HighVolatility | Regime::Transitional)
    }
}

/// Track regime history for state transitions.
pub struct RegimeTracker {
    history: Vec<(i64, Regime)>,
    max_history: usize,
}

impl RegimeTracker {
    pub fn new(max_history: usize) -> Self {
        Self {
            history: Vec::new(),
            max_history,
        }
    }

    pub fn update(&mut self, timestamp_ms: i64, features: &FeatureRow) -> Regime {
        let regime = Regime::detect(features);
        self.history.push((timestamp_ms, regime));
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
        regime
    }

    pub fn current(&self) -> Option<Regime> {
        self.history.last().map(|(_, r)| *r)
    }

    /// Regime stability: what fraction of recent N observations share the same regime.
    pub fn stability(&self, lookback: usize) -> Decimal {
        if self.history.is_empty() {
            return dec!(0);
        }
        let current = match self.current() {
            Some(r) => r,
            None => return dec!(0),
        };
        let start = self.history.len().saturating_sub(lookback);
        let recent = &self.history[start..];
        let matching = recent.iter().filter(|(_, r)| *r == current).count();
        Decimal::from(matching as u32) / Decimal::from(recent.len() as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_features() -> FeatureRow {
        FeatureRow {
            timestamp_ms: 0,
            close: dec!(50000),
            return_1: Some(dec!(0.5)),
            return_5: Some(dec!(2)),
            log_return_1: Some(dec!(0.005)),
            sma_20: Some(dec!(49500)),
            sma_50: Some(dec!(48000)),
            ema_12: Some(dec!(49800)),
            ema_26: Some(dec!(49200)),
            macd: Some(dec!(600)),
            rsi_14: Some(dec!(65)),
            atr_14: Some(dec!(500)),
            bb_upper: Some(dec!(51000)),
            bb_middle: Some(dec!(49500)),
            bb_lower: Some(dec!(48000)),
            realized_vol_20: Some(dec!(25)),
            bb_width: Some(dec!(0.06)),
            price_vs_sma20: Some(dec!(1)),
            price_vs_sma50: Some(dec!(4.2)),
            trend_strength: Some(dec!(3.1)),
            volume_sma_20: Some(dec!(1000)),
            volume_ratio: Some(dec!(1.2)),
        }
    }

    #[test]
    fn detect_trending_up() {
        let f = base_features();
        assert_eq!(Regime::detect(&f), Regime::TrendingUp);
    }

    #[test]
    fn detect_high_volatility() {
        let mut f = base_features();
        f.realized_vol_20 = Some(dec!(80));
        assert_eq!(Regime::detect(&f), Regime::HighVolatility);
    }

    #[test]
    fn detect_ranging() {
        let mut f = base_features();
        f.trend_strength = Some(dec!(0.1));
        f.rsi_14 = Some(dec!(50));
        f.realized_vol_20 = Some(dec!(25));
        f.bb_width = Some(dec!(0.04));
        assert_eq!(Regime::detect(&f), Regime::Ranging);
    }

    #[test]
    fn regime_tracker_stability() {
        let mut tracker = RegimeTracker::new(100);
        let f = base_features();
        for i in 0..10 {
            tracker.update(i, &f);
        }
        assert_eq!(tracker.stability(10), dec!(1));
    }
}
