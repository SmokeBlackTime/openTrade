//! Neural delegation engine for OpenTrade.
//!
//! Provides 100% local AI inference via Ollama with:
//! - Multi-server pool with health checks and load balancing
//! - Auto-proxy middleware (OpenAI-compatible API translation)
//! - Neural delegation pipeline (classify → route → think → synthesize)
//! - Collective thinking (multi-model consensus voting)
//! - Persistent memory system for trade learning
//!
//! Inspired by the "raise" framework's thinking map architecture.
//! All inference is local — no third-party API calls.

pub mod ollama;
pub mod pool;
pub mod proxy;
pub mod pipeline;
pub mod collective;
pub mod memory;

use serde::{Deserialize, Serialize};

/// Configuration for the neural subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralConfig {
    /// Whether neural features are enabled.
    pub enabled: bool,
    /// Ollama server endpoints (supports multiple for collective thinking).
    pub ollama_servers: Vec<OllamaServerConfig>,
    /// Default model for general analysis.
    pub default_model: String,
    /// Model for fast classification tasks.
    pub classify_model: Option<String>,
    /// Model for deep reasoning/synthesis.
    pub reasoning_model: Option<String>,
    /// Temperature for trading analysis (lower = more deterministic).
    pub temperature: f64,
    /// Max tokens for responses.
    pub max_tokens: u32,
    /// Timeout per inference request in seconds.
    pub timeout_secs: u64,
    /// Enable collective thinking (multi-model consensus).
    pub collective_thinking: bool,
    /// Minimum agreement threshold for collective decisions.
    pub consensus_threshold: f64,
    /// Memory database path.
    pub memory_db_path: String,
    /// Max memory entries to retain.
    pub max_memory_entries: usize,
}

impl Default for NeuralConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ollama_servers: vec![OllamaServerConfig::default()],
            default_model: "qwen-trading:latest".into(),
            classify_model: None,
            reasoning_model: None,
            temperature: 0.3,
            max_tokens: 4096,
            timeout_secs: 30,
            collective_thinking: false,
            consensus_threshold: 0.6,
            memory_db_path: "./data/neural_memory.db".into(),
            max_memory_entries: 100_000,
        }
    }
}

/// Configuration for a single Ollama server in the pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaServerConfig {
    /// Human-readable name for this server.
    pub name: String,
    /// Base URL (e.g., "http://localhost:11434").
    pub base_url: String,
    /// Priority weight for load balancing (higher = preferred).
    pub weight: u32,
    /// Models available on this server.
    pub models: Vec<String>,
    /// Whether this server is enabled.
    pub enabled: bool,
}

impl Default for OllamaServerConfig {
    fn default() -> Self {
        Self {
            name: "local-ollama".into(),
            base_url: "http://localhost:11434".into(),
            weight: 1,
            models: vec![],
            enabled: true,
        }
    }
}

/// Stage in the neural delegation pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStage {
    /// Classify the request type (market analysis, risk assessment, etc.)
    Classify,
    /// Route to the best model/server for this task.
    Route,
    /// Main thinking/reasoning stage.
    Think,
    /// Synthesize results from multiple thinkers.
    Synthesize,
    /// Delegate to specialized sub-tasks.
    Delegate,
}

impl std::fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Classify => write!(f, "classify"),
            Self::Route => write!(f, "route"),
            Self::Think => write!(f, "think"),
            Self::Synthesize => write!(f, "synthesize"),
            Self::Delegate => write!(f, "delegate"),
        }
    }
}

/// A traced event in the thinking pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingEvent {
    pub id: String,
    pub stage: PipelineStage,
    pub model: String,
    pub endpoint: String,
    pub timestamp_ms: i64,
    pub duration_ms: Option<u64>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub status: ThinkingStatus,
    pub branch_index: u32,
    pub branch_total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingStatus {
    Queued,
    Running,
    Completed,
    Failed,
}
