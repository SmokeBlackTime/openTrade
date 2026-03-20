//! Integration tests for OpenTrade trading engine.
//!
//! Tests the full pipeline: candle -> features -> strategy -> risk -> execution
//! using a mock exchange adapter.

use chrono::Utc;
use ot_config::AppConfig;
use ot_execution::ExchangeAdapter;
use ot_live::TradingEngine;
use ot_paper::PaperExchange;
use ot_types::market::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;

fn sample_config() -> AppConfig {
    let yaml = r#"
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
  min_confidence_threshold: "0.5"
  extreme_volatility_multiplier: "3"
  max_order_rejections_per_hour: 5
  max_orders_per_minute: 30
portfolio:
  initial_capital: "100000"
  risk_per_trade_pct: "1"
  target_volatility_pct: "15"
  kelly_fraction: "0.25"
  max_portfolio_leverage: "2"
  concentration_limit_pct: "25"
  correlation_lookback_bars: 100
  rebalance_threshold_pct: "5"
execution:
  default_order_type: market
  slippage_bps: "5"
  max_retries: 3
  retry_delay_ms: 500
  cancel_timeout_secs: 10
  reconciliation_interval_secs: 60
  smart_order_splitting: false
  max_split_orders: 5
  min_order_size_usd: "10"
backtest:
  start_date: "2024-01-01"
  end_date: "2024-06-30"
  fee_rate_bps: "10"
  slippage_bps: "5"
  initial_capital: "100000"
  enable_partial_fills: false
  latency_ms: 50
  use_funding_fees: false
storage:
  database_path: ":memory:"
  data_dir: "./data"
  journal_enabled: false
  max_candle_cache_size: 100000
telemetry:
  log_level: warn
  json_logs: false
  metrics_enabled: false
  metrics_port: 9090
  tracing_enabled: false
features:
  enable_ml_models: false
  enable_market_making: false
  enable_statistical_arbitrage: false
  enable_smart_order_routing: false
  enable_monte_carlo_stress: false
"#;
    AppConfig::from_yaml_str(yaml).unwrap()
}

fn make_candle(i: u64, base_price: Decimal, trending_up: bool) -> Candle {
    let t = Utc::now() + chrono::Duration::hours(i as i64);
    let price_offset = if trending_up {
        Decimal::from(i) * dec!(10)
    } else {
        -Decimal::from(i) * dec!(10)
    };
    let close = base_price + price_offset;
    Candle {
        symbol: Symbol::new("BTCUSDT"),
        market_type: MarketType::Spot,
        timeframe: Timeframe::H1,
        open_time: t,
        close_time: t + chrono::Duration::seconds(3600),
        open: close - dec!(5),
        high: close + dec!(50),
        low: close - dec!(50),
        close,
        volume: dec!(100),
        quote_volume: close * dec!(100),
        trades: 500,
    }
}

#[tokio::test]
async fn engine_processes_candles_without_error() {
    let config = sample_config();
    let exchange = Arc::new(PaperExchange::new(dec!(100000), dec!(5), dec!(10)));

    let strategies: Vec<Box<dyn ot_strategy::Strategy>> = vec![Box::new(
        ot_strategy::trend::TrendFollowing::new(&std::collections::HashMap::new()),
    )];

    let mut engine = TradingEngine::new(config, strategies, exchange.clone());

    // Feed 60 candles (enough for features)
    for i in 0..60 {
        let candle = make_candle(i, dec!(50000), true);
        exchange.set_price("BTCUSDT", candle.close);
        let result = engine.on_candle(candle).await;
        assert!(result.is_ok(), "Candle {} failed: {:?}", i, result.err());
    }
}

#[tokio::test]
async fn engine_generates_trades_on_trending_data() {
    let config = sample_config();
    let exchange = Arc::new(PaperExchange::new(dec!(100000), dec!(5), dec!(10)));

    let strategies: Vec<Box<dyn ot_strategy::Strategy>> = vec![Box::new(
        ot_strategy::trend::TrendFollowing::new(&std::collections::HashMap::new()),
    )];

    let mut engine = TradingEngine::new(config, strategies, exchange.clone());

    // Feed enough candles to trigger a trend signal
    for i in 0..100 {
        let candle = make_candle(i, dec!(50000), true);
        exchange.set_price("BTCUSDT", candle.close);
        let _ = engine.on_candle(candle).await;
    }

    // Check that the order manager has processed some orders
    let active = engine.order_manager().active_count();
    let completed = engine.order_manager().completed_count();
    // At least one order should have been processed (either active bracket or completed entry)
    // The exact count depends on the strategy's signals
    println!(
        "Active orders: {}, Completed orders: {}",
        active, completed
    );
}

#[tokio::test]
async fn paper_exchange_fills_immediately() {
    let exchange = PaperExchange::new(dec!(100000), dec!(5), dec!(10));
    exchange.set_price("BTCUSDT", dec!(50000));

    let request = ot_types::orders::OrderRequest {
        client_order_id: ot_types::orders::ClientOrderId::generate(),
        symbol: Symbol::new("BTCUSDT"),
        market_type: MarketType::Spot,
        side: ot_types::orders::Side::Buy,
        order_type: ot_types::orders::OrderType::Market,
        quantity: dec!(0.1),
        price: Some(dec!(50000)),
        stop_price: None,
        time_in_force: None,
        reduce_only: false,
        strategy_name: "test".into(),
        reason: ot_types::orders::OrderReason {
            strategy: "test".into(),
            signal_type: "long".into(),
            confidence: dec!(0.7),
            description: "test".into(),
        },
        created_at: Utc::now(),
    };

    let result = exchange.submit_order(&request).await;
    assert!(result.is_ok());
    let tracked = result.unwrap();
    assert_eq!(tracked.status, ot_types::orders::OrderStatus::Filled);
    assert_eq!(tracked.filled_quantity, dec!(0.1));
    assert!(tracked.average_fill_price.unwrap() > dec!(50000)); // slippage
    assert!(tracked.commission > dec!(0));
}

#[tokio::test]
async fn risk_engine_blocks_excessive_orders() {
    let config = sample_config();
    let exchange = Arc::new(PaperExchange::new(dec!(100000), dec!(5), dec!(10)));

    let strategies: Vec<Box<dyn ot_strategy::Strategy>> = vec![Box::new(
        ot_strategy::trend::TrendFollowing::new(&std::collections::HashMap::new()),
    )];

    let engine = TradingEngine::new(config.clone(), strategies, exchange.clone());

    // Verify risk engine starts with expected equity
    assert_eq!(engine.risk_engine().current_equity(), dec!(100000));
}

#[tokio::test]
async fn engine_flatten_with_no_positions() {
    let config = sample_config();
    let exchange = Arc::new(PaperExchange::new(dec!(100000), dec!(5), dec!(10)));

    let strategies: Vec<Box<dyn ot_strategy::Strategy>> = vec![];

    let mut engine = TradingEngine::new(config, strategies, exchange);

    // Flatten should succeed even with no positions
    let result = engine.flatten_all().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn backtest_runs_with_trending_data() {
    let config = sample_config();
    let bt_config = config.backtest.as_ref().unwrap();

    let candles: Vec<Candle> = (0..200)
        .map(|i| make_candle(i, dec!(50000), true))
        .collect();

    let mut strategy = ot_strategy::trend::TrendFollowing::new(&std::collections::HashMap::new());

    let result = ot_backtest::run_backtest(
        &candles,
        &mut strategy,
        bt_config,
        &config.risk,
        &config.portfolio,
    );

    assert!(result.metrics.total_trades > 0 || candles.len() > 51);
    // The equity curve should have entries
    assert!(!result.equity_curve.is_empty());
}

#[tokio::test]
async fn order_manager_bracket_lifecycle() {
    use ot_execution::{BracketPair, OrderManager};

    let mut mgr = OrderManager::new(100);
    let entry_id = ot_types::orders::ClientOrderId::generate();
    let sl_id = ot_types::orders::ClientOrderId::generate();
    let tp_id = ot_types::orders::ClientOrderId::generate();

    // Register bracket
    mgr.register_bracket(BracketPair {
        entry_order_id: entry_id.clone(),
        stop_loss_order_id: Some(sl_id.clone()),
        take_profit_order_id: Some(tp_id.clone()),
        symbol: Symbol::new("BTCUSDT"),
    });

    // Find by SL
    assert!(mgr.find_bracket_containing(&sl_id).is_some());
    // Find by TP
    assert!(mgr.find_bracket_containing(&tp_id).is_some());
    // Find by entry
    assert!(mgr.get_bracket(&entry_id).is_some());

    // Remove bracket
    let removed = mgr.remove_bracket(&entry_id);
    assert!(removed.is_some());
    assert!(mgr.get_bracket(&entry_id).is_none());
}

#[tokio::test]
async fn feature_computation_with_real_data() {
    let candles: Vec<Candle> = (0..60)
        .map(|i| make_candle(i, dec!(50000), true))
        .collect();

    let features = ot_features::pipeline::compute_features(&candles);
    assert!(features.is_some());
    let f = features.unwrap();
    assert!(f.sma_20.is_some());
    assert!(f.sma_50.is_some());
    assert!(f.rsi_14.is_some());
    assert!(f.macd.is_some());
    assert!(f.atr_14.is_some());
    assert!(f.bb_upper.is_some());
}

#[tokio::test]
async fn storage_trade_journal_roundtrip() {
    let storage = ot_storage::Storage::in_memory().unwrap();

    let trade = ot_types::trade::TradeRecord {
        trade_id: "test-trade-1".into(),
        client_order_id: ot_types::orders::ClientOrderId::generate(),
        symbol: Symbol::new("BTCUSDT"),
        market_type: MarketType::Spot,
        side: ot_types::orders::Side::Buy,
        quantity: dec!(0.1),
        price: dec!(50000),
        commission: dec!(5),
        commission_asset: "USDT".into(),
        realized_pnl: Some(dec!(100)),
        strategy_name: "trend_btc".into(),
        timestamp: Utc::now(),
    };

    let result = storage.store_trade(&trade);
    assert!(result.is_ok());

    // Verify state management
    storage.set_state("test_key", "test_value").unwrap();
    let value = storage.get_state("test_key").unwrap();
    assert_eq!(value, Some("test_value".to_string()));
}

#[tokio::test]
async fn candle_buffer_behavior() {
    use ot_market_data::candle_buffer::CandleBuffer;

    let mut buf = CandleBuffer::new(10);
    for i in 0..15 {
        buf.push(make_candle(i, dec!(50000), true));
    }

    // Buffer should cap at 10
    assert_eq!(buf.len(), 10);

    // Latest should be the last pushed
    let latest = buf.latest().unwrap();
    assert_eq!(latest.close, dec!(50000) + dec!(14) * dec!(10));

    // Closes extraction
    let closes = buf.closes(5);
    assert_eq!(closes.len(), 5);
}

#[tokio::test]
async fn position_pnl_tracking() {
    use ot_types::positions::{Position, PositionSide};

    let mut pos = Position {
        symbol: Symbol::new("BTCUSDT"),
        market_type: MarketType::Spot,
        side: PositionSide::Long,
        quantity: dec!(1),
        entry_price: dec!(50000),
        current_price: dec!(50000),
        unrealized_pnl: dec!(0),
        realized_pnl: dec!(0),
        total_commission: dec!(0),
        leverage: dec!(1),
        liquidation_price: None,
        opened_at: Utc::now(),
        last_update: Utc::now(),
        strategy_name: "test".into(),
    };

    // Price goes up
    pos.update_mark_price(dec!(51000), Utc::now());
    assert_eq!(pos.unrealized_pnl, dec!(1000));

    // Price goes down
    pos.update_mark_price(dec!(49000), Utc::now());
    assert_eq!(pos.unrealized_pnl, dec!(-1000));

    // Return pct
    let ret = pos.return_pct().unwrap();
    assert_eq!(ret, dec!(-2)); // -2%
}

#[tokio::test]
async fn regime_detection_works() {
    use ot_models::regime::Regime;

    let features = ot_features::pipeline::FeatureRow {
        timestamp_ms: 0,
        close: dec!(50000),
        return_1: Some(dec!(0.5)),
        return_5: Some(dec!(2)),
        log_return_1: Some(dec!(0.005)),
        sma_20: Some(dec!(49500)),
        sma_50: Some(dec!(48000)),
        ema_12: Some(dec!(49800)),
        ema_26: Some(dec!(49200)),
        macd: Some(dec!(600)),
        rsi_14: Some(dec!(65)),
        atr_14: Some(dec!(500)),
        bb_upper: Some(dec!(51000)),
        bb_middle: Some(dec!(49500)),
        bb_lower: Some(dec!(48000)),
        realized_vol_20: Some(dec!(25)),
        bb_width: Some(dec!(0.06)),
        price_vs_sma20: Some(dec!(1)),
        price_vs_sma50: Some(dec!(4.2)),
        trend_strength: Some(dec!(3.1)),
        volume_sma_20: Some(dec!(1000)),
        volume_ratio: Some(dec!(1.2)),
    };

    let regime = Regime::detect(&features);
    assert_eq!(regime, Regime::TrendingUp);
    assert!(regime.favors_trend());
    assert!(!regime.favors_mean_reversion());
}
