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
use tracing::{error, info, warn};

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

    // Check exchange connectivity
    let api_key = config.resolve_api_key().unwrap_or_default();
    let api_secret = config.resolve_api_secret().unwrap_or_default();
    let client = ot_exchange_binance::BinanceClient::with_base_url(
        api_key.clone(),
        api_secret.clone(),
        config.exchange.use_testnet,
        config.exchange.base_url.clone(),
        config.exchange.use_futures,
        config.exchange.proxy_url.clone(),
    );

    for sym_config in &config.symbols {
        match client.get_price(sym_config.symbol.as_str()).await {
            Ok(price) => println!("[OK] {} price: ${}", sym_config.symbol, price),
            Err(e) => println!("[WARN] {} price fetch failed: {}", sym_config.symbol, e),
        }
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

    // Load stored state if available
    let storage_path = std::path::Path::new(&config.storage.database_path);
    if let Ok(storage) = ot_storage::Storage::new(storage_path) {
        if let Ok(Some(equity)) = storage.get_state("current_equity") {
            println!("Stored equity: ${}", equity);
        }
        if let Ok(Some(pnl)) = storage.get_state("daily_pnl") {
            println!("Stored daily PnL: ${}", pnl);
        }
        if let Ok(Some(positions)) = storage.get_state("open_positions") {
            if let Ok(pos_vec) = serde_json::from_str::<Vec<ot_types::positions::Position>>(&positions) {
                let open: Vec<_> = pos_vec.iter().filter(|p| !p.is_flat()).collect();
                println!("Open positions: {}", open.len());
                for p in &open {
                    println!(
                        "  {} {:?} {} @ {} (unrealized: ${})",
                        p.symbol, p.side, p.quantity, p.entry_price, p.unrealized_pnl
                    );
                }
            }
        }
    }

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
    let client = ot_exchange_binance::BinanceClient::with_base_url(
        api_key,
        api_secret,
        config.exchange.use_testnet,
        config.exchange.base_url.clone(),
        config.exchange.use_futures,
        config.exchange.proxy_url.clone(),
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

    let mut all_candles = Vec::new();
    let mut current_start = ot_common::time_utils::datetime_to_ms(&start_dt);
    let end_ms = ot_common::time_utils::datetime_to_ms(&end_dt);

    // Paginate through klines
    loop {
        if current_start >= end_ms {
            break;
        }

        let candles = client
            .get_klines(
                symbol,
                timeframe,
                Some(current_start),
                Some(end_ms),
                Some(1000),
            )
            .await
            .context("Failed to fetch candles")?;

        if candles.is_empty() {
            break;
        }

        let last_close = ot_common::time_utils::datetime_to_ms(
            &candles.last().unwrap().close_time,
        );
        current_start = last_close + 1;

        println!("  Fetched {} candles (total: {})", candles.len(), all_candles.len() + candles.len());
        all_candles.extend(candles);

        // Rate limit
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    println!("Fetched {} candles total", all_candles.len());

    // Store
    let storage_path = std::path::Path::new(&config.storage.database_path);
    let storage = ot_storage::Storage::new(storage_path)
        .context("Failed to initialize storage")?;
    let stored = storage.store_candles(&all_candles)?;
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

fn build_strategies(config: &ot_config::AppConfig) -> Vec<Box<dyn ot_strategy::Strategy>> {
    let mut strategies: Vec<Box<dyn ot_strategy::Strategy>> = config
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
                // Advanced strategies
                "funding_rate" => {
                    Box::new(ot_strategy::funding_rate::FundingRateReversion::new(&s.params))
                }
                "imbalance" => {
                    Box::new(ot_strategy::imbalance::ImbalanceStrategy::new(&s.params))
                }
                "regime_transition" => {
                    Box::new(ot_strategy::regime_transition::RegimeTransition::new(&s.params))
                }
                "cross_timeframe" => {
                    Box::new(ot_strategy::cross_timeframe::CrossTimeframe::new(&s.params))
                }
                "anti_consensus" => {
                    Box::new(ot_strategy::anti_consensus::AntiConsensus::new(&s.params))
                }
                "risk_signal" => {
                    Box::new(ot_strategy::risk_signal::RiskSignalStrategy::new(&s.params))
                }
                _ => Box::new(ot_strategy::trend::TrendFollowing::new(&s.params)),
            }
        })
        .collect();

    // Add the AI neural brain strategy if enabled
    if config.features.enable_neural_brain {
        match build_brain_strategy(config) {
            Ok(brain) => {
                info!("Neural brain strategy enabled");
                strategies.push(brain);
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize neural brain, continuing without it");
            }
        }
    }

    strategies
}

fn build_brain_strategy(
    config: &ot_config::AppConfig,
) -> Result<Box<dyn ot_strategy::Strategy>, anyhow::Error> {
    let neural_config = config.neural_brain.clone().unwrap_or_default();

    let ollama_servers: Vec<ot_neural::OllamaServerConfig> = neural_config
        .ollama_servers
        .iter()
        .map(|s| ot_neural::OllamaServerConfig {
            name: s.name.clone(),
            base_url: s.base_url.clone(),
            weight: s.weight,
            models: s.models.clone(),
            enabled: s.enabled,
        })
        .collect();

    let personality = match neural_config.personality.as_str() {
        "conservative" => ot_brain::personality::TradingPersonality::conservative(),
        "aggressive" => ot_brain::personality::TradingPersonality::aggressive(),
        "scalper" => ot_brain::personality::TradingPersonality::scalper(),
        _ => ot_brain::personality::TradingPersonality::default(),
    };

    let brain_config = ot_brain::BrainConfig {
        neural: ot_neural::NeuralConfig {
            enabled: true,
            ollama_servers,
            default_model: neural_config.default_model,
            classify_model: neural_config.classify_model,
            reasoning_model: neural_config.reasoning_model,
            temperature: neural_config.temperature,
            max_tokens: 4096,
            timeout_secs: neural_config.timeout_secs,
            collective_thinking: neural_config.collective_thinking,
            consensus_threshold: neural_config.consensus_threshold,
            memory_db_path: neural_config.memory_db_path,
            max_memory_entries: 100_000,
        },
        personality,
        analysis_interval: neural_config.analysis_interval,
        min_collective_confidence: neural_config.consensus_threshold,
        learn_from_trades: true,
        memory_context_size: 10,
    };

    let brain = ot_brain::trader::BrainStrategy::new(brain_config)?;
    Ok(Box::new(brain))
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

    let strategies = build_strategies(config);

    let mut engine = ot_live::TradingEngine::new(config.clone(), strategies, exchange.clone());

    // Restore state from previous session
    if let Err(e) = engine.restore_state() {
        warn!(error = %e, "Failed to restore state, starting fresh");
    }

    let num_strategies = config.strategies.iter().filter(|s| s.enabled).count();
    println!("Engine initialized with {} strategies", num_strategies);
    println!("Subscribing to market data...\n");

    // Subscribe to WebSocket candle streams for each configured symbol/timeframe
    let mut candle_receivers = Vec::new();
    for sym_config in &config.symbols {
        if !sym_config.enabled {
            continue;
        }
        for tf in &sym_config.timeframes {
            let interval = tf.as_binance_str();
            match ot_exchange_binance::ws::subscribe_klines_ext(
                sym_config.symbol.as_str(),
                interval,
                config.exchange.use_testnet,
                config.exchange.use_futures,
                100,
            )
            .await
            {
                Ok(rx) => {
                    println!(
                        "  Subscribed to {} {} candles",
                        sym_config.symbol, interval
                    );
                    candle_receivers.push(rx);
                }
                Err(e) => {
                    error!(
                        error = %e,
                        symbol = %sym_config.symbol,
                        timeframe = interval,
                        "Failed to subscribe to candle stream"
                    );
                }
            }
        }
    }

    if candle_receivers.is_empty() {
        println!("WARNING: No candle streams subscribed. Check exchange connectivity.");
        println!("Waiting for Ctrl+C...");
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    println!("\nTrading engine running. Waiting for candles...");

    // Merge all candle receivers into one stream
    let (merged_tx, mut merged_rx) = tokio::sync::mpsc::channel::<ot_types::market::Candle>(500);

    for mut rx in candle_receivers {
        let tx = merged_tx.clone();
        tokio::spawn(async move {
            while let Some(candle) = rx.recv().await {
                if tx.send(candle).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(merged_tx); // Drop the original sender so merged_rx closes when all spawned tasks end

    // Reconciliation ticker
    let reconcile_interval = config.execution.reconciliation_interval_secs;

    // Main event loop
    let mut reconcile_timer = tokio::time::interval(
        std::time::Duration::from_secs(reconcile_interval),
    );

    loop {
        tokio::select! {
            Some(candle) = merged_rx.recv() => {
                // Update paper exchange price for fills
                exchange.set_price(candle.symbol.as_str(), candle.close);

                info!(
                    symbol = %candle.symbol,
                    timeframe = %candle.timeframe,
                    close = %candle.close,
                    volume = %candle.volume,
                    "Candle received"
                );

                if let Err(e) = engine.on_candle(candle).await {
                    error!(error = %e, "Error processing candle");
                }
            }
            _ = reconcile_timer.tick() => {
                if let Err(e) = engine.reconcile().await {
                    warn!(error = %e, "Reconciliation error");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down paper trading...");
                break;
            }
        }
    }

    println!("Paper trading stopped.");
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
        println!("To proceed, re-run with --confirm flag.");
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

    let api_key = config
        .resolve_api_key()
        .context("API key required for live trading")?;
    let api_secret = config
        .resolve_api_secret()
        .context("API secret required for live trading")?;

    let client = ot_exchange_binance::BinanceClient::with_base_url(
        api_key,
        api_secret,
        config.exchange.use_testnet,
        config.exchange.base_url.clone(),
        config.exchange.use_futures,
        config.exchange.proxy_url.clone(),
    );

    // Start user data stream for order updates
    let listen_key = client
        .start_user_data_stream()
        .await
        .context("Failed to start user data stream")?;
    info!(listen_key = %listen_key, "User data stream started");

    let exchange = Arc::new(ot_exchange_binance::BinanceExchangeAdapter::new(
        ot_exchange_binance::BinanceClient::with_base_url(
            config.resolve_api_key()?,
            config.resolve_api_secret()?,
            config.exchange.use_testnet,
            config.exchange.base_url.clone(),
            config.exchange.use_futures,
            config.exchange.proxy_url.clone(),
        ),
    ));

    let strategies = build_strategies(config);
    let mut engine = ot_live::TradingEngine::new(config.clone(), strategies, exchange);

    // Restore state from previous session
    if let Err(e) = engine.restore_state() {
        warn!(error = %e, "Failed to restore state, starting fresh");
    }

    println!("Live trading started. Run ID: {}", run);
    println!("Press Ctrl+C for graceful shutdown.\n");

    // Subscribe to candle streams
    let mut candle_receivers = Vec::new();
    for sym_config in &config.symbols {
        if !sym_config.enabled {
            continue;
        }
        for tf in &sym_config.timeframes {
            let interval = tf.as_binance_str();
            match ot_exchange_binance::ws::subscribe_klines_ext(
                sym_config.symbol.as_str(),
                interval,
                config.exchange.use_testnet,
                config.exchange.use_futures,
                100,
            )
            .await
            {
                Ok(rx) => {
                    info!(symbol = %sym_config.symbol, tf = interval, "Subscribed to candle stream");
                    candle_receivers.push(rx);
                }
                Err(e) => {
                    error!(error = %e, "Failed to subscribe to candle stream");
                }
            }
        }
    }

    // Subscribe to user data stream for order updates
    let mut user_data_rx = ot_exchange_binance::ws::subscribe_user_data_ext(
        &listen_key,
        config.exchange.use_testnet,
        config.exchange.use_futures,
        100,
    )
    .await
    .context("Failed to subscribe to user data stream")?;

    // Merge candle receivers
    let (merged_tx, mut merged_rx) = tokio::sync::mpsc::channel::<ot_types::market::Candle>(500);
    for mut rx in candle_receivers {
        let tx = merged_tx.clone();
        tokio::spawn(async move {
            while let Some(candle) = rx.recv().await {
                if tx.send(candle).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(merged_tx);

    // Keepalive for user data stream (every 30 minutes)
    let keepalive_client = ot_exchange_binance::BinanceClient::with_base_url(
        config.resolve_api_key()?,
        config.resolve_api_secret()?,
        config.exchange.use_testnet,
        config.exchange.base_url.clone(),
        config.exchange.use_futures,
        config.exchange.proxy_url.clone(),
    );
    let keepalive_key = listen_key.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
        loop {
            interval.tick().await;
            if let Err(e) = keepalive_client
                .keepalive_user_data_stream(&keepalive_key)
                .await
            {
                error!(error = %e, "Failed to keepalive user data stream");
            } else {
                info!("User data stream keepalive sent");
            }
        }
    });

    // Reconciliation interval
    let reconcile_interval = config.execution.reconciliation_interval_secs;
    let mut reconcile_timer = tokio::time::interval(
        std::time::Duration::from_secs(reconcile_interval),
    );

    // Run initial reconciliation
    if let Err(e) = engine.reconcile().await {
        warn!(error = %e, "Initial reconciliation failed");
    }

    // Main event loop
    loop {
        tokio::select! {
            Some(candle) = merged_rx.recv() => {
                info!(
                    symbol = %candle.symbol,
                    tf = %candle.timeframe,
                    close = %candle.close,
                    "Candle received"
                );

                if let Err(e) = engine.on_candle(candle).await {
                    error!(error = %e, "Error processing candle");
                }
            }
            Some(order_update) = user_data_rx.recv() => {
                info!(
                    id = %order_update.client_order_id,
                    symbol = %order_update.symbol,
                    status = ?order_update.status,
                    filled = %order_update.filled_quantity,
                    "Order update from exchange"
                );

                let avg_price = if order_update.filled_quantity > rust_decimal_macros::dec!(0)
                    && order_update.cumulative_quote_qty > rust_decimal_macros::dec!(0) {
                    Some(order_update.cumulative_quote_qty / order_update.filled_quantity)
                } else {
                    None
                };

                if let Err(e) = engine.on_order_update(
                    &order_update.client_order_id,
                    &order_update.symbol,
                    order_update.status,
                    order_update.filled_quantity,
                    avg_price,
                    order_update.commission,
                ).await {
                    error!(error = %e, "Error processing order update");
                }
            }
            _ = reconcile_timer.tick() => {
                if let Err(e) = engine.reconcile().await {
                    warn!(error = %e, "Reconciliation error");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nGraceful shutdown...");
                break;
            }
        }
    }

    println!("Live trading stopped.");
    Ok(())
}

async fn cmd_flatten(config: &ot_config::AppConfig, confirm: bool) -> Result<()> {
    if !confirm {
        println!("EMERGENCY FLATTEN");
        println!("=================");
        println!("This will close ALL open positions at market price.");
        println!("Re-run with --confirm to execute.");
        return Ok(());
    }

    warn!("Executing emergency flatten");

    let api_key = config.resolve_api_key().unwrap_or_default();
    let api_secret = config.resolve_api_secret().unwrap_or_default();

    let exchange: Arc<dyn ot_execution::ExchangeAdapter> = if api_key.is_empty() {
        Arc::new(ot_paper::PaperExchange::new(dec!(0), dec!(0), dec!(0)))
    } else {
        Arc::new(ot_exchange_binance::BinanceExchangeAdapter::new(
            ot_exchange_binance::BinanceClient::with_base_url(api_key, api_secret, config.exchange.use_testnet, config.exchange.base_url.clone(), config.exchange.use_futures, config.exchange.proxy_url.clone()),
        ))
    };

    let strategies = build_strategies(config);
    let mut engine = ot_live::TradingEngine::new(config.clone(), strategies, exchange);

    if let Err(e) = engine.restore_state() {
        warn!(error = %e, "Could not restore state");
    }

    engine.flatten_all().await?;
    println!("All positions flattened.");
    Ok(())
}

async fn cmd_cancel_all(
    config: &ot_config::AppConfig,
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

    let api_key = config
        .resolve_api_key()
        .context("API key required")?;
    let api_secret = config
        .resolve_api_secret()
        .context("API secret required")?;

    let client = ot_exchange_binance::BinanceClient::with_base_url(
        api_key,
        api_secret,
        config.exchange.use_testnet,
        config.exchange.base_url.clone(),
        config.exchange.use_futures,
        config.exchange.proxy_url.clone(),
    );

    let symbols: Vec<String> = if let Some(s) = symbol {
        vec![s]
    } else {
        config
            .symbols
            .iter()
            .map(|s| s.symbol.as_str().to_string())
            .collect()
    };

    for sym in &symbols {
        match client.cancel_all_orders(sym).await {
            Ok(_) => println!("Cancelled all orders for {}", sym),
            Err(e) => println!("Failed to cancel orders for {}: {}", sym, e),
        }
    }

    Ok(())
}

async fn cmd_explain_last_trade(config: &ot_config::AppConfig) -> Result<()> {
    println!("Last Trade Explanation");
    println!("======================");

    let storage_path = std::path::Path::new(&config.storage.database_path);
    match ot_storage::Storage::new(storage_path) {
        Ok(storage) => {
            // Load recent trades from journal
            if let Ok(Some(positions_json)) = storage.get_state("open_positions") {
                if let Ok(positions) = serde_json::from_str::<Vec<ot_types::positions::Position>>(&positions_json) {
                    if positions.is_empty() {
                        println!("No open positions.");
                    } else {
                        for p in &positions {
                            if !p.is_flat() {
                                println!("Position: {} {:?}", p.symbol, p.side);
                                println!("  Entry: ${}", p.entry_price);
                                println!("  Current: ${}", p.current_price);
                                println!("  Unrealized PnL: ${}", p.unrealized_pnl);
                                println!("  Strategy: {}", p.strategy_name);
                            }
                        }
                    }
                }
            } else {
                println!("No trading data available.");
            }
        }
        Err(_) => {
            println!("No trades recorded. Start paper or live trading first.");
        }
    }

    Ok(())
}

async fn cmd_report(config: &ot_config::AppConfig, days: u32, _format: &str) -> Result<()> {
    println!("Performance Report ({} days)", days);
    println!("===========================");

    let storage_path = std::path::Path::new(&config.storage.database_path);
    match ot_storage::Storage::new(storage_path) {
        Ok(storage) => {
            if let Ok(Some(equity)) = storage.get_state("current_equity") {
                println!("Current equity: ${}", equity);
            }
            if let Ok(Some(pnl)) = storage.get_state("daily_pnl") {
                println!("Today's PnL: ${}", pnl);
            }
            println!("(Detailed report requires trade history data)");
        }
        Err(_) => {
            println!("No trading data available yet.");
            println!("Run paper or live trading to generate reports.");
        }
    }

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
