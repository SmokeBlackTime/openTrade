//! AI Trading Brain for OpenTrade.
//!
//! The autonomous trading controller that:
//! - Analyzes markets using local LLMs via Ollama
//! - Makes trading decisions with collective thinking
//! - Learns from every trade outcome (memory system)
//! - Adapts strategy parameters based on regime
//! - Provides explainable reasoning for all decisions
//!
//! This is "openTrade" — an AI specialized in trading.

pub mod analyst;
pub mod learner;
pub mod personality;
pub mod trader;

use chrono::{DateTime, Utc};
use ot_features::FeatureRow;
use ot_neural::collective::{CollectiveDecision, CollectiveThinking, ServerModel, TradeDirection};
use ot_neural::ollama::ChatMessage;
use ot_neural::memory::{MemoryCategory, MemoryEntry, NeuralMemory};
use ot_neural::pipeline::NeuralPipeline;
use ot_neural::pool::OllamaPool;
use ot_neural::proxy::NeuralProxy;
use ot_neural::NeuralConfig;
use ot_types::market::{Candle, Symbol};
use ot_types::signals::{Signal, SignalDirection, SignalMetadata};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{info, warn};

use crate::personality::TradingPersonality;

/// The AI trading brain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainConfig {
    pub neural: NeuralConfig,
    pub personality: TradingPersonality,
    /// How often to run deep analysis (every N candles).
    pub analysis_interval: u32,
    /// Minimum confidence from collective to generate signal.
    pub min_collective_confidence: f64,
    /// Whether to learn from every trade.
    pub learn_from_trades: bool,
    /// Maximum number of memories to provide as context.
    pub memory_context_size: usize,
}

impl Default for BrainConfig {
    fn default() -> Self {
        Self {
            neural: NeuralConfig::default(),
            personality: TradingPersonality::default(),
            analysis_interval: 5,
            min_collective_confidence: 0.6,
            learn_from_trades: true,
            memory_context_size: 10,
        }
    }
}

/// Trading decision from the AI brain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainDecision {
    /// The trading signal (if any).
    pub signal: Option<Signal>,
    /// The collective decision that informed this.
    pub collective: Option<CollectiveDecision>,
    /// The AI's reasoning chain.
    pub reasoning: String,
    /// Relevant memories that influenced the decision.
    pub relevant_memories: Vec<String>,
    /// Current personality state.
    pub personality_state: String,
    /// Timestamp of the decision.
    pub timestamp: DateTime<Utc>,
}

/// The main AI trading brain.
///
/// Combines neural inference, collective thinking, memory, and trading
/// personality into an autonomous trading controller.
pub struct TradingBrain {
    config: BrainConfig,
    pool: Arc<OllamaPool>,
    proxy: NeuralProxy,
    pipeline: NeuralPipeline,
    collective: CollectiveThinking,
    memory: NeuralMemory,
    personality: TradingPersonality,
    /// Counter for analysis interval.
    bar_count: u32,
    /// Cache of recent analysis results.
    last_analysis: Option<String>,
    /// Running performance stats.
    stats: BrainStats,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrainStats {
    pub total_decisions: u64,
    pub signals_generated: u64,
    pub correct_signals: u64,
    pub incorrect_signals: u64,
    pub total_inference_ms: u64,
    pub avg_confidence: f64,
    pub avg_agreement: f64,
}

impl TradingBrain {
    /// Create a new trading brain.
    pub fn new(config: BrainConfig) -> Result<Self, ot_common::OtError> {
        let pool = Arc::new(OllamaPool::new(
            &config.neural.ollama_servers,
            config.neural.timeout_secs,
        ));
        let proxy = NeuralProxy::new(
            Arc::clone(&pool),
            config.neural.default_model.clone(),
        );
        let pipeline = NeuralPipeline::new(
            Arc::clone(&pool),
            config.neural.clone(),
        );

        // Build voter list: one vote per (server, model) pair so every server participates
        let voters = if config.neural.collective_thinking {
            let mut voters: Vec<ServerModel> = config
                .neural
                .ollama_servers
                .iter()
                .filter(|s| s.enabled)
                .flat_map(|s| {
                    s.models.iter().map(move |m| ServerModel {
                        server: s.name.clone(),
                        model: m.clone(),
                    })
                })
                .collect();
            if voters.is_empty() {
                voters.push(ServerModel {
                    server: "default".into(),
                    model: config.neural.default_model.clone(),
                });
            }
            voters
        } else {
            vec![ServerModel {
                server: config.neural.ollama_servers.first()
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "default".into()),
                model: config.neural.default_model.clone(),
            }]
        };

        let collective = CollectiveThinking::new(
            Arc::clone(&pool),
            voters,
            config.neural.temperature,
        );

        let memory_path = std::path::Path::new(&config.neural.memory_db_path);
        let memory = NeuralMemory::new(memory_path, config.neural.max_memory_entries)?;

        let personality = config.personality.clone();

        info!(
            model = %config.neural.default_model,
            servers = config.neural.ollama_servers.len(),
            collective = config.neural.collective_thinking,
            "Trading brain initialized"
        );

        Ok(Self {
            config,
            pool,
            proxy,
            pipeline,
            collective,
            memory,
            personality,
            bar_count: 0,
            last_analysis: None,
            stats: BrainStats::default(),
        })
    }

    /// Initialize the brain — run health checks and warm up models.
    pub async fn initialize(&self) {
        info!("Warming up neural pool...");
        self.pool.health_check_all().await;

        let healthy = self.pool.healthy_count().await;
        let total = self.pool.total_count().await;
        info!(
            healthy = healthy,
            total = total,
            "Neural pool health check complete"
        );

        if healthy == 0 {
            warn!("No healthy Ollama servers available! Neural features will be limited.");
            return;
        }

        // Warm up all configured models by sending a tiny request to each.
        // This forces Ollama to load the model into memory on each server.
        let all_models: Vec<String> = self
            .config
            .neural
            .ollama_servers
            .iter()
            .filter(|s| s.enabled)
            .flat_map(|s| s.models.iter().cloned())
            .collect();

        for model in &all_models {
            info!(model = %model, "Warming up model (preloading into memory)...");
            let messages = vec![ChatMessage::user("ping".to_string())];
            match self.pool.chat(model, messages, None, false).await {
                Ok((_, server)) => {
                    info!(model = %model, server = %server, "Model warmed up successfully");
                }
                Err(e) => {
                    warn!(model = %model, error = %e, "Failed to warm up model");
                }
            }
        }
    }

    /// Process a new candle and features, potentially generating a trading signal.
    ///
    /// The brain runs deep analysis every `analysis_interval` candles.
    /// Between analyses, it uses cached insights to make faster decisions.
    pub async fn on_bar(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_regime: &str,
    ) -> BrainDecision {
        self.bar_count += 1;
        self.stats.total_decisions += 1;

        let should_analyze = self.bar_count % self.config.analysis_interval == 0;
        let healthy = self.pool.healthy_count().await;

        info!(
            bar_count = self.bar_count,
            should_analyze = should_analyze,
            healthy_servers = healthy,
            symbol = %candle.symbol,
            "Brain on_bar"
        );

        if should_analyze && healthy > 0 {
            info!("Starting deep analysis with Ollama");
            self.deep_analysis(candle, features, current_regime).await
        } else {
            self.quick_decision(candle, features, current_regime)
        }
    }

    /// Deep analysis using the full neural pipeline.
    async fn deep_analysis(
        &mut self,
        candle: &Candle,
        features: &FeatureRow,
        current_regime: &str,
    ) -> BrainDecision {
        let start = std::time::Instant::now();

        // Build context from memory
        let relevant_memories = self.build_memory_context(&candle.symbol, current_regime);
        let memory_context = if relevant_memories.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nRelevant past observations:\n{}",
                relevant_memories.join("\n")
            )
        };

        // Build feature summary
        let features_json = serde_json::to_string_pretty(features).unwrap_or_default();
        let market_context = format!(
            "Symbol: {} | Regime: {} | Close: {} | RSI: {:?} | Trend: {:?} | Vol: {:?} | BB Width: {:?}{}",
            candle.symbol.as_str(),
            current_regime,
            candle.close,
            features.rsi_14,
            features.trend_strength,
            features.realized_vol_20,
            features.bb_width,
            memory_context,
        );

        // Run pipeline pre-analysis for richer context
        let pipeline_context = match self
            .pipeline
            .process(&market_context, &features_json)
            .await
        {
            Ok(result) => {
                info!(
                    agreement = result.agreement,
                    duration_ms = result.total_duration_ms,
                    "Pipeline pre-analysis complete"
                );
                result.reasoning_chain
            }
            Err(e) => {
                warn!(error = %e, "Pipeline pre-analysis failed, proceeding with collective only");
                String::new()
            }
        };

        // Enrich context with pipeline analysis
        let enriched_context = if pipeline_context.is_empty() {
            market_context.clone()
        } else {
            format!("{}\n\nPipeline analysis:\n{}", market_context, pipeline_context)
        };

        // Run collective deliberation with enriched context
        let decision = self
            .collective
            .deliberate(&enriched_context, &features_json)
            .await;

        let duration = start.elapsed().as_millis() as u64;
        self.stats.total_inference_ms += duration;

        // Update running averages
        let n = self.stats.total_decisions as f64;
        self.stats.avg_confidence =
            (self.stats.avg_confidence * (n - 1.0) + decision.confidence) / n;
        self.stats.avg_agreement =
            (self.stats.avg_agreement * (n - 1.0) + decision.agreement) / n;

        // Cache the analysis
        self.last_analysis = Some(decision.reasoning.clone());

        // Apply personality filter
        let personality_state = self.personality.state_description();
        let adjusted_confidence = self.personality.adjust_confidence(
            decision.confidence,
            current_regime,
        );

        // Generate signal if confidence meets threshold
        let signal = if adjusted_confidence >= self.config.min_collective_confidence
            && decision.has_consensus(self.config.neural.consensus_threshold)
            && decision.direction != TradeDirection::Hold
        {
            self.stats.signals_generated += 1;
            let atr = features.atr_14.unwrap_or(dec!(0));
            let stop_mult = self.personality.stop_loss_multiplier();
            let target_mult = self.personality.take_profit_multiplier();

            let (direction, stop, target) = match decision.direction {
                TradeDirection::Long => (
                    SignalDirection::Long,
                    Some(candle.close - atr * stop_mult),
                    Some(candle.close + atr * target_mult),
                ),
                TradeDirection::Short => (
                    SignalDirection::Short,
                    Some(candle.close + atr * stop_mult),
                    Some(candle.close - atr * target_mult),
                ),
                TradeDirection::Hold => unreachable!(),
            };

            Some(Signal {
                strategy_name: format!("brain_{}", self.personality.name()),
                symbol: candle.symbol.clone(),
                market_type: candle.market_type,
                timeframe: candle.timeframe,
                timestamp: Utc::now(),
                direction,
                strength: Decimal::try_from(adjusted_confidence).unwrap_or(dec!(0.5)),
                confidence: Decimal::try_from(adjusted_confidence).unwrap_or(dec!(0.5)),
                entry_price: Some(candle.close),
                stop_loss: stop,
                take_profit: target,
                time_stop_bars: Some(self.personality.max_hold_bars()),
                metadata: SignalMetadata {
                    signal_inputs: serde_json::json!({
                        "source": "neural_brain",
                        "collective_direction": decision.direction.to_string(),
                        "voter_count": decision.voter_count,
                        "agreement": decision.agreement,
                        "personality": self.personality.name(),
                        "inference_ms": duration,
                    }),
                    model_outputs: Some(serde_json::to_value(&decision).unwrap_or_default()),
                    uncertainty_score: Some(
                        Decimal::try_from(1.0 - adjusted_confidence).unwrap_or(dec!(0.5))
                    ),
                    regime: Some(current_regime.to_string()),
                    risk_overrides: decision.dissent.clone(),
                    portfolio_context: self.last_analysis.clone(),
                },
            })
        } else {
            None
        };

        BrainDecision {
            signal,
            collective: Some(decision),
            reasoning: self.last_analysis.clone().unwrap_or_default(),
            relevant_memories,
            personality_state,
            timestamp: Utc::now(),
        }
    }

    /// Quick decision without neural inference (between deep analyses).
    fn quick_decision(
        &self,
        _candle: &Candle,
        _features: &FeatureRow,
        _current_regime: &str,
    ) -> BrainDecision {
        BrainDecision {
            signal: None,
            collective: None,
            reasoning: self
                .last_analysis
                .clone()
                .unwrap_or_else(|| "Awaiting deep analysis cycle".into()),
            relevant_memories: vec![],
            personality_state: self.personality.state_description(),
            timestamp: Utc::now(),
        }
    }

    /// Build memory context for the current analysis.
    fn build_memory_context(&self, symbol: &Symbol, regime: &str) -> Vec<String> {
        let mut memories = Vec::new();
        let limit = self.config.memory_context_size;

        // Recent trade outcomes for this symbol
        if let Ok(trades) = self.memory.recall(
            MemoryCategory::TradeOutcome,
            Some(symbol.as_str()),
            limit / 2,
        ) {
            for trade in trades {
                let outcome = trade.outcome.map(|o| format!("{:.2}%", o * 100.0)).unwrap_or_else(|| "?".into());
                memories.push(format!("[Trade] {} — outcome: {}", trade.content, outcome));
            }
        }

        // Lessons learned
        if let Ok(lessons) = self.memory.recall(
            MemoryCategory::LessonLearned,
            None,
            limit / 4,
        ) {
            for lesson in lessons {
                memories.push(format!("[Lesson] {}", lesson.content));
            }
        }

        // Regime-specific insights
        if let Ok(insights) = self.memory.search(regime, limit / 4) {
            for insight in insights {
                memories.push(format!("[Insight] {}", insight.content));
            }
        }

        memories.truncate(self.config.memory_context_size);
        memories
    }

    /// Record a trade outcome for learning.
    pub fn learn_from_trade(
        &self,
        symbol: &str,
        direction: &str,
        pnl_pct: f64,
        regime: &str,
        strategy: &str,
        features_at_entry: &serde_json::Value,
    ) {
        if !self.config.learn_from_trades {
            return;
        }

        let outcome_description = if pnl_pct > 0.0 {
            format!(
                "{} {} trade on {} in {} regime yielded {:.2}% profit using {}",
                direction, symbol, symbol, regime, pnl_pct * 100.0, strategy
            )
        } else {
            format!(
                "{} {} trade on {} in {} regime lost {:.2}% using {}",
                direction, symbol, symbol, regime, pnl_pct.abs() * 100.0, strategy
            )
        };

        let importance = if pnl_pct.abs() > 0.05 {
            0.9 // Large moves are important
        } else if pnl_pct < -0.01 {
            0.8 // Losses are important to remember
        } else {
            0.5
        };

        let category = if pnl_pct < -0.02 {
            MemoryCategory::LessonLearned
        } else {
            MemoryCategory::TradeOutcome
        };

        let entry = MemoryEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            timestamp: Utc::now(),
            category,
            symbol: symbol.to_string(),
            content: outcome_description,
            context: serde_json::json!({
                "direction": direction,
                "pnl_pct": pnl_pct,
                "regime": regime,
                "strategy": strategy,
                "features": features_at_entry,
            }),
            importance,
            recall_count: 0,
            outcome: Some(pnl_pct),
            tags: vec![
                direction.to_string(),
                regime.to_string(),
                strategy.to_string(),
            ],
        };

        if let Err(e) = self.memory.remember(&entry) {
            warn!(error = %e, "Failed to store trade memory");
        }
    }

    /// Ask the brain a free-form question about the market.
    pub async fn ask(&self, question: &str) -> Result<String, String> {
        let recent_memories = self
            .memory
            .recall_recent(5)
            .unwrap_or_default();

        let memory_context = if recent_memories.is_empty() {
            String::new()
        } else {
            let mem_strs: Vec<String> = recent_memories
                .iter()
                .map(|m| format!("- {}", m.content))
                .collect();
            format!("\n\nRecent observations:\n{}", mem_strs.join("\n"))
        };

        let system = format!(
            "You are openTrade, an AI trading specialist. Your personality: {}. \
             Answer questions about markets with precision and data-driven insights.{}",
            self.personality.name(),
            memory_context,
        );

        self.proxy.ask(&system, question, false).await
    }

    /// Get the brain's current performance statistics.
    pub fn stats(&self) -> &BrainStats {
        &self.stats
    }

    /// Get the memory store (for direct memory operations).
    pub fn memory(&self) -> &NeuralMemory {
        &self.memory
    }

    /// Get the neural proxy (for direct API access).
    pub fn proxy(&self) -> &NeuralProxy {
        &self.proxy
    }
}
