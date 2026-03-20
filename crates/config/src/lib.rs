//! Configuration loading and validation for OpenTrade.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use validator::Validate;

use ot_types::market::{MarketType, Symbol, Timeframe};

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(String),
    #[error("Failed to parse config: {0}")]
    ParseError(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Safety limit exceeded: {field} value {value} exceeds absolute max {max}")]
    SafetyLimitExceeded {
        field: String,
        value: String,
        max: String,
    },
}

/// Top-level configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AppConfig {
    pub mode: TradingMode,
    pub exchange: ExchangeConfig,
    pub symbols: Vec<SymbolConfig>,
    pub strategies: Vec<StrategyConfig>,
    pub risk: RiskConfig,
    pub portfolio: PortfolioConfig,
    pub execution: ExecutionConfig,
    pub backtest: Option<BacktestConfig>,
    pub storage: StorageConfig,
    pub telemetry: TelemetryConfig,
    pub features: FeatureFlagsConfig,
    /// Neural AI brain configuration (optional).
    #[serde(default)]
    pub neural_brain: Option<NeuralBrainConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingMode {
    Backtest,
    Paper,
    Live,
}

impl std::fmt::Display for TradingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backtest => write!(f, "backtest"),
            Self::Paper => write!(f, "paper"),
            Self::Live => write!(f, "live"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeConfig {
    pub name: String,
    pub use_testnet: bool,
    /// Optional custom REST API base URL (e.g. "https://api1.binance.com").
    /// If not set, defaults to api.binance.com or testnet.binance.vision.
    #[serde(default)]
    pub base_url: Option<String>,
    /// API key loaded from env var name (not the key itself).
    pub api_key_env: String,
    /// API secret loaded from env var name (not the secret itself).
    pub api_secret_env: String,
    pub rate_limit_per_minute: u32,
    pub recv_window_ms: u64,
    pub timeout_secs: u64,
    pub ws_reconnect_delay_ms: u64,
    pub ws_max_reconnect_attempts: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolConfig {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timeframes: Vec<Timeframe>,
    pub enabled: bool,
    pub max_position_usd: Option<Decimal>,
    pub max_leverage: Option<Decimal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    pub name: String,
    pub strategy_type: String,
    pub enabled: bool,
    pub symbols: Vec<String>,
    pub timeframe: Timeframe,
    pub params: HashMap<String, serde_json::Value>,
    pub capital_allocation_pct: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_position_size_usd: Decimal,
    pub max_leverage: Decimal,
    pub max_daily_loss_pct: Decimal,
    pub max_drawdown_pct: Decimal,
    pub max_open_positions: usize,
    pub max_notional_exposure_usd: Decimal,
    pub max_single_order_usd: Decimal,
    pub max_trades_per_day: usize,
    pub max_correlated_exposure_pct: Decimal,
    pub stale_data_max_age_secs: u64,
    pub max_spread_bps: Decimal,
    pub min_confidence_threshold: Decimal,
    pub extreme_volatility_multiplier: Decimal,
    pub max_order_rejections_per_hour: u32,
    pub max_orders_per_minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioConfig {
    pub initial_capital: Decimal,
    pub risk_per_trade_pct: Decimal,
    pub target_volatility_pct: Option<Decimal>,
    pub kelly_fraction: Decimal,
    pub max_portfolio_leverage: Decimal,
    pub concentration_limit_pct: Decimal,
    pub correlation_lookback_bars: usize,
    pub rebalance_threshold_pct: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub default_order_type: String,
    pub slippage_bps: Decimal,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub cancel_timeout_secs: u64,
    pub reconciliation_interval_secs: u64,
    pub smart_order_splitting: bool,
    pub max_split_orders: u32,
    pub min_order_size_usd: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub start_date: String,
    pub end_date: String,
    pub fee_rate_bps: Decimal,
    pub slippage_bps: Decimal,
    pub initial_capital: Decimal,
    pub enable_partial_fills: bool,
    pub latency_ms: u64,
    pub use_funding_fees: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub database_path: String,
    pub data_dir: String,
    pub journal_enabled: bool,
    pub max_candle_cache_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub log_level: String,
    pub log_file: Option<String>,
    pub json_logs: bool,
    pub metrics_enabled: bool,
    pub metrics_port: u16,
    pub tracing_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlagsConfig {
    pub enable_ml_models: bool,
    pub enable_market_making: bool,
    pub enable_statistical_arbitrage: bool,
    pub enable_smart_order_routing: bool,
    pub enable_monte_carlo_stress: bool,
    /// Enable the AI neural trading brain (requires Ollama).
    #[serde(default)]
    pub enable_neural_brain: bool,
    /// Enable collective thinking (multi-model consensus).
    #[serde(default)]
    pub enable_collective_thinking: bool,
}

/// Configuration for the neural/AI subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralBrainConfig {
    /// Whether the neural brain is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Ollama server URLs.
    #[serde(default = "default_ollama_servers")]
    pub ollama_servers: Vec<OllamaServerEntry>,
    /// Default model name.
    #[serde(default = "default_model_name")]
    pub default_model: String,
    /// Model for fast classification.
    #[serde(default)]
    pub classify_model: Option<String>,
    /// Model for deep reasoning.
    #[serde(default)]
    pub reasoning_model: Option<String>,
    /// Temperature for inference.
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    /// Timeout per request.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Enable collective thinking.
    #[serde(default)]
    pub collective_thinking: bool,
    /// Consensus threshold.
    #[serde(default = "default_consensus")]
    pub consensus_threshold: f64,
    /// Memory database path.
    #[serde(default = "default_memory_path")]
    pub memory_db_path: String,
    /// AI personality profile.
    #[serde(default = "default_personality")]
    pub personality: String,
    /// Analysis interval (every N candles).
    #[serde(default = "default_analysis_interval")]
    pub analysis_interval: u32,
}

fn default_ollama_servers() -> Vec<OllamaServerEntry> {
    vec![OllamaServerEntry {
        name: "local-ollama".into(),
        base_url: "http://localhost:11434".into(),
        weight: 1,
        models: vec![],
        enabled: true,
    }]
}

fn default_model_name() -> String {
    "qwen-trading:latest".into()
}
fn default_temperature() -> f64 {
    0.3
}
fn default_timeout() -> u64 {
    30
}
fn default_consensus() -> f64 {
    0.6
}
fn default_memory_path() -> String {
    "./data/neural_memory.db".into()
}
fn default_personality() -> String {
    "balanced".into()
}
fn default_analysis_interval() -> u32 {
    5
}

impl Default for NeuralBrainConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ollama_servers: default_ollama_servers(),
            default_model: default_model_name(),
            classify_model: None,
            reasoning_model: None,
            temperature: default_temperature(),
            timeout_secs: default_timeout(),
            collective_thinking: false,
            consensus_threshold: default_consensus(),
            memory_db_path: default_memory_path(),
            personality: default_personality(),
            analysis_interval: default_analysis_interval(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaServerEntry {
    pub name: String,
    pub base_url: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_weight() -> u32 {
    1
}
fn default_enabled() -> bool {
    true
}

// ── Hardcoded absolute safety maxima ──
// These cannot be overridden by config. They are the last line of defense.
pub struct AbsoluteSafetyLimits;

impl AbsoluteSafetyLimits {
    pub const MAX_LEVERAGE: Decimal = dec!(20);
    pub const MAX_DAILY_LOSS_PCT: Decimal = dec!(10);
    pub const MAX_DRAWDOWN_PCT: Decimal = dec!(25);
    pub const MAX_OPEN_POSITIONS: usize = 50;
    pub const MAX_SINGLE_ORDER_USD: Decimal = dec!(1_000_000);
    pub const MAX_NOTIONAL_EXPOSURE_USD: Decimal = dec!(10_000_000);
    pub const MAX_TRADES_PER_DAY: usize = 5000;
}

impl AppConfig {
    /// Load configuration from a YAML file.
    pub fn from_yaml_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::ReadError(e.to_string()))?;
        let config: Self =
            serde_yaml::from_str(&content).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate_safety_limits()?;
        Ok(config)
    }

    /// Load configuration from a YAML string.
    pub fn from_yaml_str(s: &str) -> Result<Self, ConfigError> {
        let config: Self =
            serde_yaml::from_str(s).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate_safety_limits()?;
        Ok(config)
    }

    /// Enforce absolute safety limits that config values must not exceed.
    pub fn validate_safety_limits(&self) -> Result<(), ConfigError> {
        if self.risk.max_leverage > AbsoluteSafetyLimits::MAX_LEVERAGE {
            return Err(ConfigError::SafetyLimitExceeded {
                field: "max_leverage".into(),
                value: self.risk.max_leverage.to_string(),
                max: AbsoluteSafetyLimits::MAX_LEVERAGE.to_string(),
            });
        }
        if self.risk.max_daily_loss_pct > AbsoluteSafetyLimits::MAX_DAILY_LOSS_PCT {
            return Err(ConfigError::SafetyLimitExceeded {
                field: "max_daily_loss_pct".into(),
                value: self.risk.max_daily_loss_pct.to_string(),
                max: AbsoluteSafetyLimits::MAX_DAILY_LOSS_PCT.to_string(),
            });
        }
        if self.risk.max_drawdown_pct > AbsoluteSafetyLimits::MAX_DRAWDOWN_PCT {
            return Err(ConfigError::SafetyLimitExceeded {
                field: "max_drawdown_pct".into(),
                value: self.risk.max_drawdown_pct.to_string(),
                max: AbsoluteSafetyLimits::MAX_DRAWDOWN_PCT.to_string(),
            });
        }
        if self.risk.max_open_positions > AbsoluteSafetyLimits::MAX_OPEN_POSITIONS {
            return Err(ConfigError::SafetyLimitExceeded {
                field: "max_open_positions".into(),
                value: self.risk.max_open_positions.to_string(),
                max: AbsoluteSafetyLimits::MAX_OPEN_POSITIONS.to_string(),
            });
        }
        if self.risk.max_single_order_usd > AbsoluteSafetyLimits::MAX_SINGLE_ORDER_USD {
            return Err(ConfigError::SafetyLimitExceeded {
                field: "max_single_order_usd".into(),
                value: self.risk.max_single_order_usd.to_string(),
                max: AbsoluteSafetyLimits::MAX_SINGLE_ORDER_USD.to_string(),
            });
        }
        Ok(())
    }

    /// Resolve API key from environment variable.
    pub fn resolve_api_key(&self) -> Result<String, ConfigError> {
        std::env::var(&self.exchange.api_key_env).map_err(|_| {
            ConfigError::ReadError(format!(
                "Environment variable '{}' not set",
                self.exchange.api_key_env
            ))
        })
    }

    /// Resolve API secret from environment variable.
    pub fn resolve_api_secret(&self) -> Result<String, ConfigError> {
        std::env::var(&self.exchange.api_secret_env).map_err(|_| {
            ConfigError::ReadError(format!(
                "Environment variable '{}' not set",
                self.exchange.api_secret_env
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config_yaml() -> &'static str {
        r#"
mode: paper
exchange:
  name: binance
  use_testnet: true
  api_key_env: BINANCE_API_KEY
  api_secret_env: BINANCE_API_SECRET
  rate_limit_per_minute: 1200
  recv_window_ms: 5000
  timeout_secs: 30
  ws_reconnect_delay_ms: 1000
  ws_max_reconnect_attempts: 10
symbols:
  - symbol: BTCUSDT
    market_type: spot
    timeframes: ["1h"]
    enabled: true
strategies:
  - name: trend_btc
    strategy_type: trend_following
    enabled: true
    symbols: ["BTCUSDT"]
    timeframe: "1h"
    params:
      fast_period: 20
      slow_period: 50
    capital_allocation_pct: "0.5"
risk:
  max_position_size_usd: "100000"
  max_leverage: "3"
  max_daily_loss_pct: "2"
  max_drawdown_pct: "10"
  max_open_positions: 5
  max_notional_exposure_usd: "500000"
  max_single_order_usd: "50000"
  max_trades_per_day: 100
  max_correlated_exposure_pct: "40"
  stale_data_max_age_secs: 30
  max_spread_bps: "50"
  min_confidence_threshold: "0.6"
  extreme_volatility_multiplier: "3"
  max_order_rejections_per_hour: 5
  max_orders_per_minute: 30
portfolio:
  initial_capital: "100000"
  risk_per_trade_pct: "0.5"
  target_volatility_pct: "15"
  kelly_fraction: "0.25"
  max_portfolio_leverage: "2"
  concentration_limit_pct: "25"
  correlation_lookback_bars: 100
  rebalance_threshold_pct: "5"
execution:
  default_order_type: limit
  slippage_bps: "5"
  max_retries: 3
  retry_delay_ms: 500
  cancel_timeout_secs: 10
  reconciliation_interval_secs: 60
  smart_order_splitting: false
  max_split_orders: 5
  min_order_size_usd: "10"
storage:
  database_path: "./data/opentrade.db"
  data_dir: "./data"
  journal_enabled: true
  max_candle_cache_size: 100000
telemetry:
  log_level: info
  json_logs: false
  metrics_enabled: true
  metrics_port: 9090
  tracing_enabled: true
features:
  enable_ml_models: false
  enable_market_making: false
  enable_statistical_arbitrage: false
  enable_smart_order_routing: false
  enable_monte_carlo_stress: true
  enable_neural_brain: false
  enable_collective_thinking: false
"#
    }

    #[test]
    fn parse_sample_config() {
        let config = AppConfig::from_yaml_str(sample_config_yaml()).unwrap();
        assert_eq!(config.mode, TradingMode::Paper);
        assert_eq!(config.symbols.len(), 1);
        assert_eq!(config.risk.max_leverage, dec!(3));
    }

    #[test]
    fn reject_unsafe_leverage() {
        let yaml = sample_config_yaml().replace("max_leverage: \"3\"", "max_leverage: \"50\"");
        let result = AppConfig::from_yaml_str(&yaml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ConfigError::SafetyLimitExceeded { .. }));
    }

    #[test]
    fn reject_unsafe_drawdown() {
        let yaml = sample_config_yaml().replace("max_drawdown_pct: \"10\"", "max_drawdown_pct: \"30\"");
        let result = AppConfig::from_yaml_str(&yaml);
        assert!(result.is_err());
    }
}
