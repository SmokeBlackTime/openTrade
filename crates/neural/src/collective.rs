//! Collective thinking: multi-model consensus for trading decisions.
//!
//! Runs the same analysis across multiple models/servers simultaneously,
//! then combines results via weighted voting. This implements the "neural
//! delegation" concept where multiple AI agents collaborate on decisions.

use crate::ollama::{ChatMessage, OllamaOptions};
use crate::pool::OllamaPool;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// A vote from a single model in the collective.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVote {
    pub model: String,
    pub server: String,
    pub direction: TradeDirection,
    pub confidence: f64,
    pub reasoning: String,
    pub regime: String,
    pub risk_level: String,
    pub duration_ms: u64,
}

/// Trade direction for voting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeDirection {
    Long,
    Short,
    Hold,
}

impl std::fmt::Display for TradeDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Long => write!(f, "long"),
            Self::Short => write!(f, "short"),
            Self::Hold => write!(f, "hold"),
        }
    }
}

/// Result of collective deliberation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectiveDecision {
    /// The consensus direction.
    pub direction: TradeDirection,
    /// Weighted confidence (0-1).
    pub confidence: f64,
    /// Agreement level between models (0-1).
    pub agreement: f64,
    /// Number of models that participated.
    pub voter_count: usize,
    /// Individual votes for transparency.
    pub votes: Vec<ModelVote>,
    /// Aggregated reasoning.
    pub reasoning: String,
    /// Dissenting opinions (if any).
    pub dissent: Vec<String>,
    /// Total deliberation time.
    pub total_duration_ms: u64,
}

impl CollectiveDecision {
    /// Convert consensus confidence to Decimal for the trading system.
    pub fn confidence_decimal(&self) -> Decimal {
        Decimal::try_from(self.confidence).unwrap_or(dec!(0.5))
    }

    /// Whether the collective has strong enough consensus to act.
    pub fn has_consensus(&self, threshold: f64) -> bool {
        self.agreement >= threshold && self.confidence >= threshold
    }
}

/// The collective thinking engine.
///
/// Coordinates multiple models to reach consensus on trading decisions.
/// Each model votes independently, then votes are aggregated with weighted
/// scoring based on model confidence and historical accuracy.
/// A (server_name, model_name) pair representing one voter in the collective.
#[derive(Debug, Clone)]
pub struct ServerModel {
    pub server: String,
    pub model: String,
}

pub struct CollectiveThinking {
    pool: Arc<OllamaPool>,
    /// Server-model pairs: each pair gets one vote.
    voters: Vec<ServerModel>,
    /// Voter weights keyed by "server:model". Higher = more trusted.
    voter_weights: HashMap<String, f64>,
    /// Temperature for individual model inference.
    temperature: f64,
}

impl CollectiveThinking {
    pub fn new(pool: Arc<OllamaPool>, voters: Vec<ServerModel>, temperature: f64) -> Self {
        let voter_weights = voters
            .iter()
            .map(|v| (format!("{}:{}", v.server, v.model), 1.0))
            .collect();

        Self {
            pool,
            voters,
            voter_weights,
            temperature,
        }
    }

    /// Update a voter's weight based on accuracy feedback.
    pub fn update_weight(&mut self, server: &str, model: &str, accuracy: f64) {
        let key = format!("{}:{}", server, model);
        if let Some(weight) = self.voter_weights.get_mut(&key) {
            // Exponential moving average of accuracy
            *weight = *weight * 0.9 + accuracy * 0.1;
        }
    }

    /// Run collective deliberation on market data.
    pub async fn deliberate(
        &self,
        market_context: &str,
        features_json: &str,
    ) -> CollectiveDecision {
        let system_prompt = "You are a quantitative trading analyst participating in a collective \
            decision-making process. Analyze the market data independently and provide your vote.\n\n\
            Respond with JSON:\n\
            {\"direction\": \"long|short|hold\", \"confidence\": 0.0-1.0, \
            \"reasoning\": \"brief explanation\", \"regime\": \"trending|ranging|volatile|squeeze\", \
            \"risk_level\": \"low|medium|high\"}";

        let user_prompt = format!(
            "Market context:\n{}\n\nFeature data:\n{}",
            market_context, features_json
        );

        // Spawn parallel inference across all server-model voters
        let mut handles = Vec::new();
        for voter in &self.voters {
            let pool = Arc::clone(&self.pool);
            let server_name = voter.server.clone();
            let model_name = voter.model.clone();
            let system = system_prompt.to_string();
            let prompt = user_prompt.clone();
            let temp = self.temperature;

            let handle = tokio::spawn(async move {
                let start = std::time::Instant::now();
                let messages = vec![
                    ChatMessage::system(system),
                    ChatMessage::user(prompt),
                ];

                let options = OllamaOptions {
                    temperature: Some(temp),
                    num_predict: Some(512),
                    ..Default::default()
                };

                match pool
                    .chat_on_server(&server_name, &model_name, messages, Some(options), true)
                    .await
                {
                    Ok((resp, server)) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let answer = resp.answer();

                        match serde_json::from_str::<serde_json::Value>(answer) {
                            Ok(v) => Some(ModelVote {
                                model: model_name,
                                server,
                                direction: match v["direction"]
                                    .as_str()
                                    .unwrap_or("hold")
                                {
                                    "long" => TradeDirection::Long,
                                    "short" => TradeDirection::Short,
                                    _ => TradeDirection::Hold,
                                },
                                confidence: v["confidence"].as_f64().unwrap_or(0.5),
                                reasoning: v["reasoning"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                                regime: v["regime"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                risk_level: v["risk_level"]
                                    .as_str()
                                    .unwrap_or("medium")
                                    .to_string(),
                                duration_ms: duration,
                            }),
                            Err(e) => {
                                warn!(model = %model_name, error = %e, "Failed to parse vote");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        warn!(model = %model_name, error = %e, "Model inference failed");
                        None
                    }
                }
            });

            handles.push(handle);
        }

        // Collect votes
        let mut votes = Vec::new();
        for handle in handles {
            if let Ok(Some(vote)) = handle.await {
                votes.push(vote);
            }
        }

        self.aggregate_votes(votes)
    }

    /// Aggregate individual votes into a collective decision.
    fn aggregate_votes(&self, votes: Vec<ModelVote>) -> CollectiveDecision {
        if votes.is_empty() {
            return CollectiveDecision {
                direction: TradeDirection::Hold,
                confidence: 0.0,
                agreement: 0.0,
                voter_count: 0,
                votes: vec![],
                reasoning: "No models available for deliberation".into(),
                dissent: vec![],
                total_duration_ms: 0,
            };
        }

        let total_duration: u64 = votes.iter().map(|v| v.duration_ms).max().unwrap_or(0);

        // Weighted voting
        let mut direction_scores: HashMap<TradeDirection, f64> = HashMap::new();
        let mut total_weight = 0.0;

        for vote in &votes {
            let key = format!("{}:{}", vote.server, vote.model);
            let model_weight = self.voter_weights.get(&key).copied().unwrap_or(1.0);
            let weighted_conf = vote.confidence * model_weight;
            *direction_scores.entry(vote.direction).or_default() += weighted_conf;
            total_weight += model_weight;
        }

        // Normalize scores
        if total_weight > 0.0 {
            for score in direction_scores.values_mut() {
                *score /= total_weight;
            }
        }

        // Find winning direction
        let (winning_direction, winning_score) = direction_scores
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(&d, &s)| (d, s))
            .unwrap_or((TradeDirection::Hold, 0.0));

        // Compute agreement (what fraction voted for the winner)
        let winner_count = votes
            .iter()
            .filter(|v| v.direction == winning_direction)
            .count();
        let agreement = winner_count as f64 / votes.len() as f64;

        // Collect reasoning
        let winning_reasons: Vec<&str> = votes
            .iter()
            .filter(|v| v.direction == winning_direction)
            .map(|v| v.reasoning.as_str())
            .collect();

        let dissent: Vec<String> = votes
            .iter()
            .filter(|v| v.direction != winning_direction)
            .map(|v| format!("[{}] {}: {}", v.model, v.direction, v.reasoning))
            .collect();

        let reasoning = if winning_reasons.is_empty() {
            "No consensus reasoning available".into()
        } else {
            winning_reasons.join(" | ")
        };

        info!(
            direction = %winning_direction,
            confidence = winning_score,
            agreement = agreement,
            voters = votes.len(),
            dissent_count = dissent.len(),
            "Collective decision reached"
        );

        CollectiveDecision {
            direction: winning_direction,
            confidence: winning_score,
            agreement,
            voter_count: votes.len(),
            votes,
            reasoning,
            dissent,
            total_duration_ms: total_duration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vote(model: &str, direction: TradeDirection, confidence: f64) -> ModelVote {
        ModelVote {
            model: model.into(),
            server: "local".into(),
            direction,
            confidence,
            reasoning: "test".into(),
            regime: "trending".into(),
            risk_level: "medium".into(),
            duration_ms: 100,
        }
    }

    #[test]
    fn aggregate_unanimous_votes() {
        let pool = Arc::new(OllamaPool::new(&[], 5));
        let ct = CollectiveThinking::new(pool, vec!["m1".into(), "m2".into()], 0.3);

        let votes = vec![
            make_vote("m1", TradeDirection::Long, 0.8),
            make_vote("m2", TradeDirection::Long, 0.7),
        ];

        let decision = ct.aggregate_votes(votes);
        assert_eq!(decision.direction, TradeDirection::Long);
        assert_eq!(decision.agreement, 1.0);
        assert!(decision.dissent.is_empty());
    }

    #[test]
    fn aggregate_split_votes() {
        let pool = Arc::new(OllamaPool::new(&[], 5));
        let ct = CollectiveThinking::new(pool, vec!["m1".into(), "m2".into(), "m3".into()], 0.3);

        let votes = vec![
            make_vote("m1", TradeDirection::Long, 0.9),
            make_vote("m2", TradeDirection::Short, 0.3),
            make_vote("m3", TradeDirection::Long, 0.7),
        ];

        let decision = ct.aggregate_votes(votes);
        assert_eq!(decision.direction, TradeDirection::Long);
        assert!(decision.agreement > 0.5);
        assert_eq!(decision.dissent.len(), 1);
    }

    #[test]
    fn empty_votes_hold() {
        let pool = Arc::new(OllamaPool::new(&[], 5));
        let ct = CollectiveThinking::new(pool, vec![], 0.3);
        let decision = ct.aggregate_votes(vec![]);
        assert_eq!(decision.direction, TradeDirection::Hold);
        assert_eq!(decision.confidence, 0.0);
    }
}
