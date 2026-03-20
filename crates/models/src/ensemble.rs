use ot_features::FeatureRow;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::ModelRegistry;

/// Ensemble scorer that combines predictions from multiple models.
pub struct EnsembleScorer {
    min_confidence: Decimal,
}

impl EnsembleScorer {
    pub fn new(min_confidence: Decimal) -> Self {
        Self { min_confidence }
    }

    /// Score using all registered models, returning weighted average prediction.
    pub fn score(
        &self,
        registry: &ModelRegistry,
        features: &FeatureRow,
    ) -> Option<EnsembleResult> {
        let predictions = registry.predict_all(features);
        if predictions.is_empty() {
            return None;
        }

        let n = Decimal::from(predictions.len() as u32);
        let mut total_up = dec!(0);
        let mut total_down = dec!(0);
        let mut total_flat = dec!(0);
        let mut total_confidence = dec!(0);

        for pred in &predictions {
            total_up += pred.direction_probs.up;
            total_down += pred.direction_probs.down;
            total_flat += pred.direction_probs.flat;
            total_confidence += pred.confidence;
        }

        let avg_up = total_up / n;
        let avg_down = total_down / n;
        let avg_flat = total_flat / n;
        let avg_confidence = total_confidence / n;

        let direction = if avg_up > avg_down && avg_up > avg_flat {
            "up"
        } else if avg_down > avg_up && avg_down > avg_flat {
            "down"
        } else {
            "flat"
        };

        let agreement = predictions
            .iter()
            .filter(|p| p.direction_probs.predicted_direction() == direction)
            .count();
        let agreement_ratio = Decimal::from(agreement as u32) / n;

        Some(EnsembleResult {
            direction: direction.to_string(),
            avg_up_prob: avg_up,
            avg_down_prob: avg_down,
            avg_flat_prob: avg_flat,
            avg_confidence,
            model_agreement: agreement_ratio,
            num_models: predictions.len(),
            meets_threshold: avg_confidence >= self.min_confidence,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EnsembleResult {
    pub direction: String,
    pub avg_up_prob: Decimal,
    pub avg_down_prob: Decimal,
    pub avg_flat_prob: Decimal,
    pub avg_confidence: Decimal,
    pub model_agreement: Decimal,
    pub num_models: usize,
    pub meets_threshold: bool,
}
