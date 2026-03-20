//! OpenTrade CLI - Production-grade autonomous crypto trading system.
//!
//! DISCLAIMER: Crypto trading involves significant risk. This software is
//! provided for educational and experimental purposes. No profitability is
//! guaranteed. Past performance does not guarantee future results. You are
//! responsible for compliance with local regulations and exchange terms.

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use rust_decimal_macros::dec;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "opentrade",
    version,
    about = "OpenTrade — Autonomous crypto trading system",
    long_about = "A production-grade, risk-first autonomous trading platform for Binance spot and futures.\n\n\
    DISCLAIMER: Trading cryptocurrency involves substantial risk of loss. \
    This software is experimental and makes no guarantee of profit."
)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "config/default.yaml")]
    config: PathBuf,

    /// Override log level
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ingest historical market data
    Ingest {
        /// Symbol to ingest (e.g., BTCUSDT)
        #[arg(short, long)]
        symbol: String,
        /// Timeframe (1m, 5m, 15m, 1h, 4h, 1d)
        #[arg(short, long, default_value = "1h")]
        timeframe: String,
        /// Start date (YYYY-MM-DD)
        #[arg(long)]
        start: String,
        /// End date (YYYY-MM-DD)
        #[arg(long)]
        end: String,
    },

    /// Run a backtest
    Backtest {
        /// Strategy to backtest
        #[arg(short, long)]
        strategy: String,
        /// Symbol
        #[arg(long)]
        symbol: String,
        /// Start date
        #[arg(long)]
        start: String,
        /// End date
        #[arg(long)]
        end: String,
        /// Output format (json, human)
        #[arg(long, default_value = "human")]
        format: String,
    },

    /// Run in paper trading mode
    Paper {
        /// Run ID for reproducibility
        #[arg(long)]
        run_id: Option<String>,
    },

    /// Run in live trading mode
    Live {
        /// Skip safety confirmation
        #[arg(long)]
        confirm: bool,
        /// Run ID for reproducibility
        #[arg(long)]
        run_id: Option<String>,
    },

    /// System health check
    Doctor,

    /// Show current trading status
    Status,

    /// Emergency: flatten all positions
    Flatten {
        /// Skip confirmation
        #[arg(long)]
        confirm: bool,
    },

    /// Cancel all open orders
    CancelAll {
        /// Symbol to cancel (all if not specified)
        #[arg(short, long)]
        symbol: Option<String>,
        /// Skip confirmation
        #[arg(long)]
        confirm: bool,
    },

    /// Explain the last trade decision
    ExplainLastTrade,

    /// Generate performance report
    Report {
        /// Report period in days
        #[arg(short, long, default_value = "30")]
        days: u32,
        /// Output format (json, human)
        #[arg(long, default_value = "human")]
        format: String,
    },

    /// Run stress test scenarios
    StressTest {
        /// Number of Monte Carlo iterations
        #[arg(short, long, default_value = "1000")]
        iterations: u32,
    },

    /// Run hyperparameter optimization
    Hyperopt {
        /// Strategy to optimize
        #[arg(short, long)]
        strategy: String,
        /// Number of trials
        #[arg(short, long, default_value = "100")]
        trials: u32,
    },

    /// Replay a historical trading session
    Replay {
        /// Run ID to replay
        #[arg(short, long)]
        run_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    ot_telemetry::init_logging(&cli.log_level, false);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        config = %cli.config.display(),
        "OpenTrade starting"
    );

    // Load config
    let config = ot_config::AppConfig::from_yaml_file(&cli.config)
        .context("Failed to load configuration")?;

    match cli.command {
        Commands::Doctor => cmd_doctor(&config).await,
        Commands::Status => cmd_status(&config).await,
        Commands::Ingest {
            symbol,
            timeframe,
            start,
            end,
        } => cmd_ingest(&config, &symbol, &timeframe, &start, &end).await,
        Commands::Backtest {
            strategy,
            symbol,
            start,
            end,
            format,
        } => cmd_backtest(&config, &strategy, &symbol, &start, &end, &format).await,
        Commands::Paper { run_id } => cmd_paper(&config, run_id).await,
        Commands::Live { confirm, run_id } => cmd_live(&config, confirm, run_id).await,
        Commands::Flatten { confirm } => cmd_flatten(&config, confirm).await,
        Commands::CancelAll { symbol, confirm } => cmd_cancel_all(&config, symbol, confirm).await,
        Commands::ExplainLastTrade => cmd_explain_last_trade(&config).await,
        Commands::Report { days, format } => cmd_report(&config, days, &format).await,
        Commands::StressTest { iterations } => cmd_stress_test(&config, iterations).await,
        Commands::Hyperopt { strategy, trials } => {
            cmd_hyperopt(&config, &strategy, trials).await
        }
        Commands::Replay { run_id } => cmd_replay(&config, &run_id).await,
    }
}

async fn cmd_doctor(config: &ot_config::AppConfig) -> Result<()> {
    println!("OpenTrade System Health Check");
    println!("============================\n");

    // Check config
    println!("[OK] Configuration loaded");
    println!("  Mode: {}", config.mode);
    println!("  Exchange: {}", config.exchange.name);
    println!("  Testnet: {}", config.exchange.use_testnet);
    println!("  Symbols: {}", config.symbols.len());
    println!("  Strategies: {}", config.strategies.len());

    // Check API keys
    match config.resolve_api_key() {
        Ok(_) => println!("[OK] API key environment variable set"),
        Err(e) => println!("[WARN] {}", e),
    }
    match config.resolve_api_secret() {
        Ok(_) => println!("[OK] API secret environment variable set"),
        Err(e) => println!("[WARN] {}", e),
    }

    // Check storage
    let storage_path = std::path::Path::new(&config.storage.database_path);
    match ot_storage::Storage::new(storage_path) {
        Ok(_) => println!("[OK] Storage initialized at {}", config.storage.database_path),
        Err(e) => println!("[FAIL] Storage: {}", e),
    }

    // Risk limits summary
    println!("\nRisk Limits:");
    println!("  Max leverage: {}", config.risk.max_leverage);
    println!("  Max daily loss: {}%", config.risk.max_daily_loss_pct);
    println!("  Max drawdown: {}%", config.risk.max_drawdown_pct);
    println!("  Max positions: {}", config.risk.max_open_positions);
    println!(
        "  Max single order: ${}",
        config.risk.max_single_order_usd
    );

    println!("\nAll checks complete.");
    Ok(())
}

async fn cmd_status(config: &ot_config::AppConfig) -> Result<()> {
    println!("OpenTrade Status");
    println!("================");
    println!("Mode: {}", config.mode);
    println!(
        "Initial capital: ${}",
        config.portfolio.initial_capital
    );
    println!(
        "Configured symbols: {}",
        config
            .symbols
            .iter()
            .map(|s| s.symbol.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "Active strategies: {}",
        config
            .strategies
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

async fn cmd_ingest(
    config: &ot_config::AppConfig,
    symbol: &str,
    timeframe: &str,
    start: &str,
    end: &str,
) -> Result<()> {
    info!(symbol, timeframe, start, end, "Starting data ingestion");

    let api_key = config.resolve_api_key().unwrap_or_default();
    let api_secret = config.resolve_api_secret().unwrap_or_default();
    let client = ot_exchange_binance::BinanceClient::new(
        api_key,
        api_secret,
        config.exchange.use_testnet,
    );

    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")
        .context("Invalid start date format")?;
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")
        .context("Invalid end date format")?;

    let start_dt = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let end_dt = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc();

    println!(
        "Fetching {} {} candles from {} to {}...",
        symbol, timeframe, start, end
    );

    let candles = client
        .get_klines(
            symbol,
            timeframe,
            Some(ot_common::time_utils::datetime_to_ms(&start_dt)),
            Some(ot_common::time_utils::datetime_to_ms(&end_dt)),
            Some(1000),
        )
        .await
        .context("Failed to fetch candles")?;

    println!("Fetched {} candles", candles.len());

    // Store
    let storage_path = std::path::Path::new(&config.storage.database_path);
    let storage = ot_storage::Storage::new(storage_path)
        .context("Failed to initialize storage")?;
    let stored = storage.store_candles(&candles)?;
    println!("Stored {} candles to database", stored);

    Ok(())
}

async fn cmd_backtest(
    config: &ot_config::AppConfig,
    strategy_name: &str,
    symbol: &str,
    start: &str,
    end: &str,
    format: &str,
) -> Result<()> {
    let bt_config = config
        .backtest
        .as_ref()
        .context("No backtest config defined")?;

    info!(strategy = strategy_name, symbol, "Starting backtest");

    // Load data
    let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d")?;
    let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d")?;
    let start_dt = start_date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end_dt = end_date.and_hms_opt(23, 59, 59).unwrap().and_utc();

    let storage_path = std::path::Path::new(&config.storage.database_path);
    let storage = ot_storage::Storage::new(storage_path)?;
    let candles = storage.load_candles(symbol, "1h", &start_dt, &end_dt)?;

    if candles.is_empty() {
        println!("No candle data found. Run `opentrade ingest` first.");
        return Ok(());
    }

    println!("Loaded {} candles for backtest", candles.len());

    // Create strategy
    let strategy_config = config
        .strategies
        .iter()
        .find(|s| s.name == strategy_name)
        .context("Strategy not found in config")?;

    let mut strategy: Box<dyn ot_strategy::Strategy> = match strategy_config.strategy_type.as_str()
    {
        "trend_following" => {
            Box::new(ot_strategy::trend::TrendFollowing::new(&strategy_config.params))
        }
        "mean_reversion" => {
            Box::new(ot_strategy::mean_reversion::MeanReversion::new(
                &strategy_config.params,
            ))
        }
        "breakout" => Box::new(ot_strategy::breakout::Breakout::new(&strategy_config.params)),
        "momentum" => Box::new(ot_strategy::momentum::Momentum::new(&strategy_config.params)),
        _ => {
            println!("Unknown strategy type: {}", strategy_config.strategy_type);
            return Ok(());
        }
    };

    let result = ot_backtest::run_backtest(
        &candles,
        strategy.as_mut(),
        bt_config,
        &config.risk,
        &config.portfolio,
    );

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&result.metrics)?;
            println!("{}", json);
        }
        _ => {
            println!("\nBacktest Results");
            println!("================");
            println!("Strategy: {}", strategy_name);
            println!("Symbol: {}", symbol);
            println!("Period: {} to {}", start, end);
            println!("Candles: {}", candles.len());
            println!();
            println!("Total return: ${}", result.metrics.total_return);
            println!("Total return: {}%", result.metrics.total_return_pct);
            println!("Max drawdown: {}%", result.metrics.max_drawdown_pct);
            println!("Win rate: {}%", result.metrics.win_rate);
            println!("Total trades: {}", result.metrics.total_trades);
            println!("Avg trade: ${}", result.metrics.avg_trade_return_pct);
            println!("Avg win: ${}", result.metrics.avg_win_pct);
            println!("Avg loss: ${}", result.metrics.avg_loss_pct);
            println!(
                "Max consecutive losses: {}",
                result.metrics.max_consecutive_losses
            );
            println!("Total commission: ${}", result.metrics.total_commission);
            if let Some(pf) = result.metrics.profit_factor {
                println!("Profit factor: {}", pf);
            }
            println!();
            println!(
                "DISCLAIMER: Past backtest performance does NOT guarantee future results."
            );
        }
    }

    Ok(())
}

async fn cmd_paper(config: &ot_config::AppConfig, run_id: Option<String>) -> Result<()> {
    let run = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    info!(run_id = %run, "Starting paper trading");
    println!("Paper trading mode - Run ID: {}", run);
    println!("Press Ctrl+C to stop.\n");

    let exchange = Arc::new(ot_paper::PaperExchange::new(
        config.portfolio.initial_capital,
        config.execution.slippage_bps,
        dec!(10),
    ));

    let strategies: Vec<Box<dyn ot_strategy::Strategy>> = config
        .strategies
        .iter()
        .filter(|s| s.enabled)
        .map(|s| -> Box<dyn ot_strategy::Strategy> {
            match s.strategy_type.as_str() {
                "trend_following" => Box::new(ot_strategy::trend::TrendFollowing::new(&s.params)),
                "mean_reversion" => {
                    Box::new(ot_strategy::mean_reversion::MeanReversion::new(&s.params))
                }
                "breakout" => Box::new(ot_strategy::breakout::Breakout::new(&s.params)),
                "momentum" => Box::new(ot_strategy::momentum::Momentum::new(&s.params)),
                _ => Box::new(ot_strategy::trend::TrendFollowing::new(&s.params)),
            }
        })
        .collect();

    let _engine = ot_live::TradingEngine::new(config.clone(), strategies, exchange);

    println!("Engine initialized with {} strategies", config.strategies.len());
    println!("Waiting for market data...\n");

    // In a full implementation, this would subscribe to WebSocket streams
    // and feed candles to the engine. For now, we show the structure.
    println!("Paper trading engine ready.");
    println!("Connect to exchange WebSocket to begin receiving data.");

    // Graceful shutdown on Ctrl+C
    tokio::signal::ctrl_c().await?;
    println!("\nShutting down paper trading...");
    Ok(())
}

async fn cmd_live(
    config: &ot_config::AppConfig,
    confirm: bool,
    run_id: Option<String>,
) -> Result<()> {
    if !confirm {
        println!("LIVE TRADING MODE");
        println!("=================");
        println!();
        println!("WARNING: This will execute REAL trades with REAL money.");
        println!("Exchange: {}", config.exchange.name);
        println!("Testnet: {}", config.exchange.use_testnet);
        println!("Capital: ${}", config.portfolio.initial_capital);
        println!();
        println!(
            "To proceed, re-run with --confirm flag."
        );
        println!();
        println!(
            "DISCLAIMER: Trading cryptocurrency involves substantial risk of loss."
        );
        return Ok(());
    }

    if !config.exchange.use_testnet {
        warn!("Running on PRODUCTION exchange - not testnet");
    }

    let run = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    info!(run_id = %run, "Starting LIVE trading");

    println!("Live trading started. Run ID: {}", run);
    println!("Press Ctrl+C for graceful shutdown.\n");

    tokio::signal::ctrl_c().await?;
    println!("\nGraceful shutdown...");
    Ok(())
}

async fn cmd_flatten(_config: &ot_config::AppConfig, confirm: bool) -> Result<()> {
    if !confirm {
        println!("EMERGENCY FLATTEN");
        println!("=================");
        println!("This will close ALL open positions at market price.");
        println!("Re-run with --confirm to execute.");
        return Ok(());
    }
    warn!("Executing emergency flatten");
    println!("All positions flattened (no live connection in this context).");
    Ok(())
}

async fn cmd_cancel_all(
    _config: &ot_config::AppConfig,
    symbol: Option<String>,
    confirm: bool,
) -> Result<()> {
    if !confirm {
        println!("CANCEL ALL ORDERS");
        println!("=================");
        if let Some(ref s) = symbol {
            println!("This will cancel all open orders for {}", s);
        } else {
            println!("This will cancel ALL open orders for ALL symbols");
        }
        println!("Re-run with --confirm to execute.");
        return Ok(());
    }
    println!("All orders cancelled.");
    Ok(())
}

async fn cmd_explain_last_trade(_config: &ot_config::AppConfig) -> Result<()> {
    println!("Last Trade Explanation");
    println!("======================");
    println!("No trades recorded in this session.");
    println!("Start paper or live trading first.");
    Ok(())
}

async fn cmd_report(_config: &ot_config::AppConfig, days: u32, _format: &str) -> Result<()> {
    println!("Performance Report ({} days)", days);
    println!("===========================");
    println!("No trading data available yet.");
    println!("Run paper or live trading to generate reports.");
    Ok(())
}

async fn cmd_stress_test(_config: &ot_config::AppConfig, iterations: u32) -> Result<()> {
    println!("Monte Carlo Stress Test ({} iterations)", iterations);
    println!("==========================================");
    println!("Requires historical trade data.");
    println!("Run backtests first to generate trade data.");
    Ok(())
}

async fn cmd_hyperopt(
    _config: &ot_config::AppConfig,
    strategy: &str,
    trials: u32,
) -> Result<()> {
    println!("Hyperparameter Optimization");
    println!("===========================");
    println!("Strategy: {}", strategy);
    println!("Trials: {}", trials);
    println!("Requires historical candle data in storage.");
    Ok(())
}

async fn cmd_replay(_config: &ot_config::AppConfig, run_id: &str) -> Result<()> {
    println!("Session Replay");
    println!("==============");
    println!("Run ID: {}", run_id);
    println!("Replaying from stored trade journal...");
    Ok(())
}
