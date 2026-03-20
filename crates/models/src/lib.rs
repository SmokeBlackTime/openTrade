//! ML/AI model layer for OpenTrade.
//!
//! Provides model trait, feature normalization, regime detection,
//! and ensemble scoring. Models predict direction probabilities,
//! expected returns, and volatility forecasts.

pub mod ensemble;
pub mod regime;

use chrono::{DateTime, Utc};
use ot_features::FeatureRow;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Prediction output from a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrediction {
    pub model_id: String,
    pub model_version: String,
    pub timestamp: DateTime<Utc>,
    pub direction_probs: DirectionProbs,
    pub expected_return: Option<Decimal>,
    pub volatility_forecast: Option<Decimal>,
    pub confidence: Decimal,
    pub feature_importance: Option<HashMap<String, Decimal>>,
}

/// Direction probabilities (must sum to ~1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectionProbs {
    pub up: Decimal,
    pub down: Decimal,
    pub flat: Decimal,
}

impl DirectionProbs {
    pub fn predicted_direction(&self) -> &str {
        if self.up > self.down && self.up > self.flat {
            "up"
        } else if self.down > self.up && self.down > self.flat {
            "down"
        } else {
            "flat"
        }
    }

    pub fn max_prob(&self) -> Decimal {
        self.up.max(self.down).max(self.flat)
    }
}

/// Trait for predictive models.
pub trait PredictionModel: Send + Sync {
    fn model_id(&self) -> &str;
    fn version(&self) -> &str;
    fn predict(&self, features: &FeatureRow) -> Option<ModelPrediction>;
}

/// Model registry for champion/challenger framework.
pub struct ModelRegistry {
    models: HashMap<String, Box<dyn PredictionModel>>,
    champion: Option<String>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            champion: None,
        }
    }

    pub fn register(&mut self, model: Box<dyn PredictionModel>) {
        let id = model.model_id().to_string();
        self.models.insert(id, model);
    }

    pub fn set_champion(&mut self, model_id: &str) {
        if self.models.contains_key(model_id) {
            self.champion = Some(model_id.to_string());
        }
    }

    pub fn predict_champion(&self, features: &FeatureRow) -> Option<ModelPrediction> {
        let champion_id = self.champion.as_ref()?;
        self.models.get(champion_id)?.predict(features)
    }

    pub fn predict_all(
        &self,
        features: &FeatureRow,
    ) -> Vec<ModelPrediction> {
        self.models
            .values()
            .filter_map(|m| m.predict(features))
            .collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple threshold-based classifier (baseline model).
/// This is NOT a real ML model. It's a rule-based baseline for testing.
pub struct ThresholdClassifier {
    id: String,
    version: String,
    rsi_long_threshold: Decimal,
    rsi_short_threshold: Decimal,
    trend_threshold: Decimal,
}

impl ThresholdClassifier {
    pub fn new() -> Self {
        Self {
            id: format!("threshold_{}", Uuid::new_v4().simple()),
            version: "1.0.0".into(),
            rsi_long_threshold: dec!(35),
            rsi_short_threshold: dec!(65),
            trend_threshold: dec!(0.5),
        }
    }
}

impl Default for ThresholdClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl PredictionModel for ThresholdClassifier {
    fn model_id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn predict(&self, features: &FeatureRow) -> Option<ModelPrediction> {
        let rsi = features.rsi_14?;
        let trend = features.trend_strength.unwrap_or(dec!(0));

        let (up, down, flat) = if rsi < self.rsi_long_threshold && trend > self.trend_threshold {
            (dec!(0.55), dec!(0.20), dec!(0.25))
        } else if rsi > self.rsi_short_threshold && trend < -self.trend_threshold {
            (dec!(0.20), dec!(0.55), dec!(0.25))
        } else if rsi < dec!(40) {
            (dec!(0.40), dec!(0.25), dec!(0.35))
        } else if rsi > dec!(60) {
            (dec!(0.25), dec!(0.40), dec!(0.35))
        } else {
            (dec!(0.30), dec!(0.30), dec!(0.40))
        };

        let probs = DirectionProbs { up, down, flat };
        let confidence = probs.max_prob();

        Some(ModelPrediction {
            model_id: self.id.clone(),
            model_version: self.version.clone(),
            timestamp: Utc::now(),
            direction_probs: probs,
            expected_return: features.return_1,
            volatility_forecast: features.realized_vol_20,
            confidence,
            feature_importance: None,
        })
    }
}
