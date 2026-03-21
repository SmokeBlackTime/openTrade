//! Neural delegation pipeline: classify → route → think → synthesize.
//!
//! Inspired by the "raise" framework's thinking map architecture.
//! Each stage can use a different model/server optimized for its task.
//! The pipeline traces all events for observability.

use crate::ollama::{ChatMessage, OllamaOptions};
use crate::pool::OllamaPool;
use crate::{NeuralConfig, PipelineStage, ThinkingEvent, ThinkingStatus};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, info, warn};

/// Classification result from the first pipeline stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    /// Type of analysis needed.
    pub task_type: TaskType,
    /// Confidence in the classification (0-1).
    pub confidence: f64,
    /// Suggested models for this task type.
    pub suggested_models: Vec<String>,
    /// Number of branches to spawn (for collective thinking).
    pub branch_count: u32,
}

/// Types of tasks the neural pipeline can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    /// Market regime analysis.
    RegimeAnalysis,
    /// Trade signal evaluation.
    SignalEvaluation,
    /// Risk assessment.
    RiskAssessment,
    /// Portfolio rebalancing advice.
    PortfolioAdvice,
    /// Post-trade review and learning.
    TradeReview,
    /// General market commentary.
    MarketCommentary,
    /// Strategy parameter tuning.
    ParameterTuning,
}

/// Routing decision from the second pipeline stage.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// Which model to use for each branch.
    pub branch_models: Vec<String>,
    /// Which server to prefer for each branch.
    pub branch_servers: Vec<Option<String>>,
    /// Specialized system prompts per branch.
    pub branch_prompts: Vec<String>,
}

/// Result from a single thinking branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingResult {
    pub branch_index: u32,
    pub model: String,
    pub server: String,
    /// The model's reasoning (from <think> block if present).
    pub reasoning: Option<String>,
    /// The final answer/analysis.
    pub answer: String,
    /// Structured output if JSON mode was used.
    pub structured: Option<serde_json::Value>,
    pub duration_ms: u64,
}

/// Synthesized output combining multiple thinking branches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizedResult {
    /// The consensus answer.
    pub answer: String,
    /// Agreement level between branches (0-1).
    pub agreement: f64,
    /// Per-branch summaries.
    pub branch_summaries: Vec<String>,
    /// Structured consensus if available.
    pub structured: Option<serde_json::Value>,
    /// The full reasoning chain.
    pub reasoning_chain: String,
    /// Total thinking time across all branches.
    pub total_duration_ms: u64,
}

/// The neural delegation pipeline.
///
/// Processes requests through stages:
/// 1. **Classify**: Determine what kind of analysis is needed
/// 2. **Route**: Select models/servers for the task
/// 3. **Think**: Run inference on one or more branches (parallel)
/// 4. **Synthesize**: Combine branch results into consensus
pub struct NeuralPipeline {
    pool: Arc<OllamaPool>,
    config: NeuralConfig,
    /// Trace of all events for observability.
    events: Vec<ThinkingEvent>,
    /// Round-robin counter for distributing work across servers.
    server_rotation: AtomicUsize,
}

impl NeuralPipeline {
    pub fn new(pool: Arc<OllamaPool>, config: NeuralConfig) -> Self {
        Self {
            pool,
            config,
            events: Vec::new(),
            server_rotation: AtomicUsize::new(0),
        }
    }

    /// Run the full pipeline on a prompt.
    pub async fn process(
        &mut self,
        system_context: &str,
        user_prompt: &str,
    ) -> Result<SynthesizedResult, String> {
        self.events.clear();

        // Stage 1: Classify
        let classification = self.classify(user_prompt).await?;
        info!(
            task_type = ?classification.task_type,
            confidence = classification.confidence,
            branches = classification.branch_count,
            "Pipeline: classified"
        );

        // Stage 2: Route
        let route = self.route(&classification).await;
        debug!(
            branch_models = ?route.branch_models,
            "Pipeline: routed"
        );

        // Stage 3: Think (parallel branches)
        let results = self
            .think(system_context, user_prompt, &route, &classification)
            .await?;
        info!(
            branches_completed = results.len(),
            "Pipeline: thinking complete"
        );

        // Stage 4: Synthesize
        let synthesized = self.synthesize(&results, &classification).await?;
        info!(
            agreement = synthesized.agreement,
            duration_ms = synthesized.total_duration_ms,
            "Pipeline: synthesized"
        );

        Ok(synthesized)
    }

    /// Stage 1: Classify the incoming request.
    async fn classify(&mut self, prompt: &str) -> Result<Classification, String> {
        let classify_model = self
            .config
            .classify_model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());

        let event_id = uuid::Uuid::new_v4().simple().to_string();
        self.events.push(ThinkingEvent {
            id: event_id.clone(),
            stage: PipelineStage::Classify,
            model: classify_model.clone(),
            endpoint: String::new(),
            timestamp_ms: Utc::now().timestamp_millis(),
            duration_ms: None,
            input_tokens: None,
            output_tokens: None,
            status: ThinkingStatus::Running,
            branch_index: 0,
            branch_total: 1,
        });

        let classify_prompt = format!(
            r#"Classify this trading analysis request into exactly one category.

Request: "{}"

Respond with JSON:
{{"task_type": "regime_analysis|signal_evaluation|risk_assessment|portfolio_advice|trade_review|market_commentary|parameter_tuning", "confidence": 0.0-1.0, "branch_count": 1-3}}"#,
            prompt
        );

        let messages = vec![
            ChatMessage::system("You are a request classifier for a trading system. Output only valid JSON."),
            ChatMessage::user(classify_prompt),
        ];

        let options = OllamaOptions {
            temperature: Some(0.1),
            num_predict: Some(256),
            ..Default::default()
        };

        // Route classify to a specific server using rotation
        let servers = self.pool.servers_for_model(&classify_model).await;
        let classify_result = if servers.is_empty() {
            self.pool.chat(&classify_model, messages, Some(options), true).await
        } else {
            let idx = self.server_rotation.fetch_add(1, Ordering::Relaxed);
            let server_name = &servers[idx % servers.len()];
            self.pool.chat_on_server(server_name, &classify_model, messages, Some(options), true).await
        };

        match classify_result {
            Ok((resp, server)) => {
                // Update event
                if let Some(event) = self.events.iter_mut().find(|e| e.id == event_id) {
                    event.endpoint = server;
                    event.status = ThinkingStatus::Completed;
                    event.output_tokens = resp.eval_count;
                }

                // Parse classification
                let answer = resp.answer();
                match serde_json::from_str::<serde_json::Value>(answer) {
                    Ok(v) => Ok(Classification {
                        task_type: match v["task_type"].as_str().unwrap_or("market_commentary") {
                            "regime_analysis" => TaskType::RegimeAnalysis,
                            "signal_evaluation" => TaskType::SignalEvaluation,
                            "risk_assessment" => TaskType::RiskAssessment,
                            "portfolio_advice" => TaskType::PortfolioAdvice,
                            "trade_review" => TaskType::TradeReview,
                            "parameter_tuning" => TaskType::ParameterTuning,
                            _ => TaskType::MarketCommentary,
                        },
                        confidence: v["confidence"].as_f64().unwrap_or(0.5),
                        suggested_models: vec![self.config.default_model.clone()],
                        branch_count: if self.config.collective_thinking {
                            v["branch_count"].as_u64().unwrap_or(1).min(3) as u32
                        } else {
                            1
                        },
                    }),
                    Err(_) => {
                        // Fallback classification
                        Ok(Classification {
                            task_type: TaskType::MarketCommentary,
                            confidence: 0.3,
                            suggested_models: vec![self.config.default_model.clone()],
                            branch_count: 1,
                        })
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Classification failed, using fallback");
                Ok(Classification {
                    task_type: TaskType::MarketCommentary,
                    confidence: 0.3,
                    suggested_models: vec![self.config.default_model.clone()],
                    branch_count: 1,
                })
            }
        }
    }

    /// Stage 2: Route to the best model/server for each branch.
    async fn route(&self, classification: &Classification) -> RouteDecision {
        let branch_count = classification.branch_count as usize;
        let reasoning_model = self
            .config
            .reasoning_model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());

        // System prompts specialized per task type
        let base_prompt = match classification.task_type {
            TaskType::RegimeAnalysis => {
                "You are a quantitative market regime analyst. Classify market conditions precisely. \
                 Consider trend, volatility, momentum, and structural factors."
            }
            TaskType::SignalEvaluation => {
                "You are a trading signal evaluator. Assess signal quality, conviction, and timing. \
                 Consider false positive risk and current market regime."
            }
            TaskType::RiskAssessment => {
                "You are a risk analyst. Evaluate portfolio risk, concentration, correlation, \
                 and tail risk scenarios. Be conservative."
            }
            TaskType::PortfolioAdvice => {
                "You are a portfolio strategist. Advise on position sizing, rebalancing, \
                 and capital allocation across strategies."
            }
            TaskType::TradeReview => {
                "You are a trade reviewer. Analyze completed trades for lessons learned. \
                 Identify what worked, what didn't, and actionable improvements."
            }
            TaskType::ParameterTuning => {
                "You are a strategy optimization expert. Suggest parameter adjustments \
                 based on recent performance data. Avoid overfitting."
            }
            TaskType::MarketCommentary => {
                "You are a market analyst. Provide clear, data-driven market analysis \
                 with actionable insights for algorithmic trading."
            }
        };

        // For collective thinking, vary the prompts slightly per branch
        let branch_prompts: Vec<String> = (0..branch_count)
            .map(|i| {
                if branch_count == 1 {
                    base_prompt.to_string()
                } else {
                    match i {
                        0 => format!("{} Focus on the BULLISH case and upside scenarios.", base_prompt),
                        1 => format!("{} Focus on the BEARISH case and downside risks.", base_prompt),
                        _ => format!("{} Take a NEUTRAL, balanced perspective weighing both sides.", base_prompt),
                    }
                }
            })
            .collect();

        // Distribute branches across available servers in round-robin fashion.
        // The rotation counter persists across calls so single-branch requests
        // alternate between servers instead of always hitting servers[0].
        let servers = self.pool.servers_for_model(&reasoning_model).await;
        let branch_servers: Vec<Option<String>> = if servers.is_empty() {
            vec![None; branch_count]
        } else {
            let start = self.server_rotation.fetch_add(branch_count, Ordering::Relaxed);
            (0..branch_count)
                .map(|i| Some(servers[(start + i) % servers.len()].clone()))
                .collect()
        };

        RouteDecision {
            branch_models: vec![reasoning_model; branch_count],
            branch_servers,
            branch_prompts,
        }
    }

    /// Stage 3: Run thinking on one or more branches in parallel.
    async fn think(
        &mut self,
        system_context: &str,
        user_prompt: &str,
        route: &RouteDecision,
        classification: &Classification,
    ) -> Result<Vec<ThinkingResult>, String> {
        let branch_count = route.branch_models.len();
        let mut handles = Vec::new();

        for i in 0..branch_count {
            let pool = Arc::clone(&self.pool);
            let model = route.branch_models[i].clone();
            let server = route.branch_servers[i].clone();
            let system_prompt = format!("{}\n\n{}", route.branch_prompts[i], system_context);
            let prompt = user_prompt.to_string();
            let temperature = self.config.temperature;
            let max_tokens = self.config.max_tokens;
            let json_mode = matches!(
                classification.task_type,
                TaskType::RegimeAnalysis | TaskType::SignalEvaluation | TaskType::RiskAssessment
            );

            let handle = tokio::spawn(async move {
                let start = std::time::Instant::now();
                let messages = vec![
                    ChatMessage::system(system_prompt),
                    ChatMessage::user(prompt),
                ];

                let options = OllamaOptions {
                    temperature: Some(temperature),
                    num_predict: Some(max_tokens),
                    ..Default::default()
                };

                // Route to specific server if assigned, otherwise let pool decide
                let result = if let Some(ref server_name) = server {
                    pool.chat_on_server(server_name, &model, messages, Some(options), json_mode).await
                } else {
                    pool.chat(&model, messages, Some(options), json_mode).await
                };

                match result {
                    Ok((resp, server)) => {
                        let duration = start.elapsed().as_millis() as u64;
                        let reasoning = resp.thinking().map(String::from);
                        let answer = resp.answer().to_string();
                        let structured = if json_mode {
                            serde_json::from_str(&answer).ok()
                        } else {
                            None
                        };

                        Ok(ThinkingResult {
                            branch_index: i as u32,
                            model: resp.model,
                            server,
                            reasoning,
                            answer,
                            structured,
                            duration_ms: duration,
                        })
                    }
                    Err(e) => Err(format!("Branch {} failed: {}", i, e)),
                }
            });

            handles.push(handle);
        }

        // Collect results
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => results.push(result),
                Ok(Err(e)) => warn!(error = %e, "Thinking branch failed"),
                Err(e) => warn!(error = %e, "Thinking branch panicked"),
            }
        }

        if results.is_empty() {
            return Err("All thinking branches failed".into());
        }

        Ok(results)
    }

    /// Stage 4: Synthesize results from multiple branches.
    async fn synthesize(
        &self,
        results: &[ThinkingResult],
        _classification: &Classification,
    ) -> Result<SynthesizedResult, String> {
        let total_duration: u64 = results.iter().map(|r| r.duration_ms).sum();
        let branch_summaries: Vec<String> = results
            .iter()
            .map(|r| {
                format!(
                    "Branch {} ({}@{}, {}ms): {}",
                    r.branch_index,
                    r.model,
                    r.server,
                    r.duration_ms,
                    if r.answer.len() > 200 {
                        format!("{}...", &r.answer[..200])
                    } else {
                        r.answer.clone()
                    }
                )
            })
            .collect();

        if results.len() == 1 {
            // Single branch — no synthesis needed
            let r = &results[0];
            return Ok(SynthesizedResult {
                answer: r.answer.clone(),
                agreement: 1.0,
                branch_summaries,
                structured: r.structured.clone(),
                reasoning_chain: r
                    .reasoning
                    .clone()
                    .unwrap_or_else(|| r.answer.clone()),
                total_duration_ms: total_duration,
            });
        }

        // Multi-branch synthesis: combine structured outputs or synthesize via LLM
        if let Some(structured_results) = self.try_merge_structured(results) {
            let reasoning_chain = results
                .iter()
                .filter_map(|r| {
                    r.reasoning.as_ref().map(|reasoning| {
                        format!("[Branch {}] {}", r.branch_index, reasoning)
                    })
                })
                .collect::<Vec<_>>()
                .join("\n\n");

            return Ok(SynthesizedResult {
                answer: serde_json::to_string_pretty(&structured_results)
                    .unwrap_or_default(),
                agreement: self.compute_agreement(results),
                branch_summaries,
                structured: Some(structured_results),
                reasoning_chain,
                total_duration_ms: total_duration,
            });
        }

        // Fallback: use LLM to synthesize
        let synthesis_prompt = format!(
            "Synthesize these {} analysis branches into a single consensus:\n\n{}",
            results.len(),
            results
                .iter()
                .map(|r| format!("Branch {}: {}", r.branch_index, r.answer))
                .collect::<Vec<_>>()
                .join("\n\n")
        );

        let messages = vec![
            ChatMessage::system(
                "You are a synthesis engine. Combine multiple analyses into a balanced consensus. \
                 Note areas of agreement and disagreement.",
            ),
            ChatMessage::user(synthesis_prompt),
        ];

        let options = OllamaOptions {
            temperature: Some(0.2),
            num_predict: Some(self.config.max_tokens),
            ..Default::default()
        };

        // Route synthesis to a specific server using rotation
        let synth_servers = self.pool.servers_for_model(&self.config.default_model).await;
        let synth_result = if synth_servers.is_empty() {
            self.pool.chat(&self.config.default_model, messages, Some(options), false).await
        } else {
            let idx = self.server_rotation.fetch_add(1, Ordering::Relaxed);
            let server_name = &synth_servers[idx % synth_servers.len()];
            self.pool.chat_on_server(server_name, &self.config.default_model, messages, Some(options), false).await
        };

        match synth_result {
            Ok((resp, _)) => {
                let reasoning_chain = results
                    .iter()
                    .filter_map(|r| {
                        r.reasoning.as_ref().map(|reasoning| {
                            format!("[Branch {}] {}", r.branch_index, reasoning)
                        })
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");

                Ok(SynthesizedResult {
                    answer: resp.answer().to_string(),
                    agreement: self.compute_agreement(results),
                    branch_summaries,
                    structured: None,
                    reasoning_chain,
                    total_duration_ms: total_duration,
                })
            }
            Err(e) => {
                // Fallback: just use the first result
                warn!(error = %e, "Synthesis LLM call failed, using first branch");
                let r = &results[0];
                Ok(SynthesizedResult {
                    answer: r.answer.clone(),
                    agreement: 0.5,
                    branch_summaries,
                    structured: r.structured.clone(),
                    reasoning_chain: r.reasoning.clone().unwrap_or_default(),
                    total_duration_ms: total_duration,
                })
            }
        }
    }

    /// Try to merge structured JSON results from multiple branches.
    fn try_merge_structured(&self, results: &[ThinkingResult]) -> Option<serde_json::Value> {
        let structured: Vec<&serde_json::Value> = results
            .iter()
            .filter_map(|r| r.structured.as_ref())
            .collect();

        if structured.len() < 2 {
            return None;
        }

        // Merge by averaging numeric fields and collecting string fields
        let mut merged = serde_json::Map::new();
        merged.insert(
            "branch_count".into(),
            serde_json::json!(structured.len()),
        );

        // Collect all keys from all branches
        let mut all_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        for s in &structured {
            if let Some(obj) = s.as_object() {
                for key in obj.keys() {
                    all_keys.insert(key.clone());
                }
            }
        }

        for key in &all_keys {
            let values: Vec<&serde_json::Value> = structured
                .iter()
                .filter_map(|s| s.get(key))
                .collect();

            if values.is_empty() {
                continue;
            }

            // If all values are numbers, average them
            let numbers: Vec<f64> = values
                .iter()
                .filter_map(|v| v.as_f64())
                .collect();

            if numbers.len() == values.len() && !numbers.is_empty() {
                let avg = numbers.iter().sum::<f64>() / numbers.len() as f64;
                merged.insert(key.clone(), serde_json::json!(avg));
            } else {
                // Take majority vote for strings, or first value
                merged.insert(key.clone(), values[0].clone());
            }
        }

        Some(serde_json::Value::Object(merged))
    }

    /// Compute agreement level between branches (0-1).
    fn compute_agreement(&self, results: &[ThinkingResult]) -> f64 {
        if results.len() <= 1 {
            return 1.0;
        }

        let structured: Vec<&serde_json::Value> = results
            .iter()
            .filter_map(|r| r.structured.as_ref())
            .collect();

        if structured.len() < 2 {
            return 0.5; // Can't measure agreement without structured output
        }

        // Compare key fields across branches
        let mut agreements = 0u32;
        let mut comparisons = 0u32;

        // Check if "direction" or "regime" fields agree
        for key in &["direction", "regime", "signal", "action"] {
            let values: Vec<String> = structured
                .iter()
                .filter_map(|s| s.get(*key).and_then(|v| v.as_str()).map(String::from))
                .collect();

            if values.len() >= 2 {
                comparisons += 1;
                let first = &values[0];
                if values.iter().all(|v| v == first) {
                    agreements += 1;
                }
            }
        }

        // Check numeric fields (confidence, score) for closeness
        for key in &["confidence", "score", "probability"] {
            let values: Vec<f64> = structured
                .iter()
                .filter_map(|s| s.get(*key).and_then(|v| v.as_f64()))
                .collect();

            if values.len() >= 2 {
                comparisons += 1;
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                let max_dev = values
                    .iter()
                    .map(|v| (v - mean).abs())
                    .fold(0.0f64, f64::max);
                if max_dev < 0.2 {
                    agreements += 1;
                }
            }
        }

        if comparisons == 0 {
            return 0.5;
        }

        agreements as f64 / comparisons as f64
    }

    /// Get the trace of all events.
    pub fn events(&self) -> &[ThinkingEvent] {
        &self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_type_serialization() {
        let tt = TaskType::RegimeAnalysis;
        let json = serde_json::to_string(&tt).unwrap();
        assert_eq!(json, "\"regime_analysis\"");
    }

    #[test]
    fn compute_agreement_single_branch() {
        let pool = Arc::new(OllamaPool::new(&[], 5));
        let config = NeuralConfig::default();
        let pipeline = NeuralPipeline::new(pool, config);
        let results = vec![ThinkingResult {
            branch_index: 0,
            model: "test".into(),
            server: "local".into(),
            reasoning: None,
            answer: "test".into(),
            structured: None,
            duration_ms: 100,
        }];
        assert_eq!(pipeline.compute_agreement(&results), 1.0);
    }
}
