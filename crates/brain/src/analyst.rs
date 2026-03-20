//! Market analyst: uses LLM to provide deep market analysis.
//!
//! Generates structured market reports, regime assessments,
//! and strategy recommendations using the neural pipeline.

use ot_features::FeatureRow;
use ot_neural::proxy::NeuralProxy;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Structured market analysis output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketAnalysis {
    pub symbol: String,
    pub regime: String,
    pub regime_confidence: f64,
    pub trend_direction: String,
    pub trend_strength: f64,
    pub volatility_assessment: String,
    pub key_levels: KeyLevels,
    pub risk_factors: Vec<String>,
    pub opportunities: Vec<String>,
    pub recommended_strategies: Vec<String>,
    pub overall_bias: String,
    pub reasoning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyLevels {
    pub support: Option<f64>,
    pub resistance: Option<f64>,
    pub pivot: Option<f64>,
}

/// The market analyst component.
pub struct MarketAnalyst;

impl MarketAnalyst {
    /// Generate a comprehensive market analysis using the LLM.
    pub async fn analyze(
        proxy: &NeuralProxy,
        symbol: &str,
        features: &FeatureRow,
        regime: &str,
    ) -> Option<MarketAnalysis> {
        let features_summary = format!(
            "Close: {}, RSI(14): {:?}, MACD: {:?}, SMA20: {:?}, SMA50: {:?}, \
             ATR(14): {:?}, BB Width: {:?}, Vol(20): {:?}, Trend: {:?}, \
             Volume Ratio: {:?}, Return(1): {:?}, Return(5): {:?}",
            features.close,
            features.rsi_14,
            features.macd,
            features.sma_20,
            features.sma_50,
            features.atr_14,
            features.bb_width,
            features.realized_vol_20,
            features.trend_strength,
            features.volume_ratio,
            features.return_1,
            features.return_5,
        );

        let system = "You are a quantitative market analyst. Analyze technical indicators \
                      and provide structured market assessment. Output valid JSON only.";

        let prompt = format!(
            r#"Analyze {} (current regime: {}):

{}

Respond with JSON:
{{
  "regime": "trending_up|trending_down|ranging|high_volatility|low_volatility|transitional",
  "regime_confidence": 0.0-1.0,
  "trend_direction": "bullish|bearish|neutral",
  "trend_strength": 0.0-1.0,
  "volatility_assessment": "low|normal|elevated|extreme",
  "key_levels": {{"support": null_or_number, "resistance": null_or_number, "pivot": null_or_number}},
  "risk_factors": ["factor1", "factor2"],
  "opportunities": ["opportunity1"],
  "recommended_strategies": ["strategy_name"],
  "overall_bias": "bullish|bearish|neutral",
  "reasoning": "brief explanation"
}}"#,
            symbol, regime, features_summary
        );

        match proxy.ask(system, &prompt, true).await {
            Ok(response) => match serde_json::from_str::<MarketAnalysis>(&response) {
                Ok(mut analysis) => {
                    analysis.symbol = symbol.to_string();
                    Some(analysis)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to parse market analysis");
                    None
                }
            },
            Err(e) => {
                warn!(error = %e, "Market analysis LLM call failed");
                None
            }
        }
    }

    /// Generate a quick directional assessment (faster than full analysis).
    pub async fn quick_direction(
        proxy: &NeuralProxy,
        symbol: &str,
        features: &FeatureRow,
    ) -> Option<(String, f64)> {
        let prompt = format!(
            "Quick assessment for {}: RSI={:?}, Trend={:?}, MACD={:?}, Vol={:?}. \
             JSON: {{\"direction\": \"long|short|hold\", \"confidence\": 0.0-1.0}}",
            symbol,
            features.rsi_14,
            features.trend_strength,
            features.macd,
            features.realized_vol_20,
        );

        match proxy
            .ask(
                "You are a quick market scanner. Output only JSON.",
                &prompt,
                true,
            )
            .await
        {
            Ok(response) => {
                let v: serde_json::Value = serde_json::from_str(&response).ok()?;
                let direction = v["direction"].as_str()?.to_string();
                let confidence = v["confidence"].as_f64().unwrap_or(0.5);
                Some((direction, confidence))
            }
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_levels_serialization() {
        let kl = KeyLevels {
            support: Some(48000.0),
            resistance: Some(52000.0),
            pivot: Some(50000.0),
        };
        let json = serde_json::to_string(&kl).unwrap();
        assert!(json.contains("48000"));
    }
}
