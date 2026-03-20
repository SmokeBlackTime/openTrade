//! Telemetry, logging, and metrics for OpenTrade.

use prometheus::{
    register_counter_vec, register_gauge, register_histogram_vec, CounterVec, Gauge,
    HistogramVec, TextEncoder, Encoder,
};
use tracing_subscriber::{fmt, EnvFilter};

/// Initialize structured logging with the specified level.
pub fn init_logging(level: &str, json: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

    if json {
        fmt()
            .with_env_filter(filter)
            .json()
            .with_target(true)
            .with_thread_ids(true)
            .init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    }
}

/// Trading metrics for Prometheus.
pub struct TradingMetrics {
    pub trades_total: CounterVec,
    pub portfolio_equity: Gauge,
    pub open_positions: Gauge,
    pub daily_pnl: Gauge,
    pub signal_confidence: HistogramVec,
    pub order_latency: HistogramVec,
    pub drawdown_pct: Gauge,
}

impl TradingMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        Ok(Self {
            trades_total: register_counter_vec!(
                "opentrade_trades_total",
                "Total number of trades",
                &["strategy", "direction", "result"]
            )?,
            portfolio_equity: register_gauge!(
                "opentrade_portfolio_equity_usd",
                "Current portfolio equity in USD"
            )?,
            open_positions: register_gauge!(
                "opentrade_open_positions",
                "Number of open positions"
            )?,
            daily_pnl: register_gauge!(
                "opentrade_daily_pnl_usd",
                "Daily PnL in USD"
            )?,
            signal_confidence: register_histogram_vec!(
                "opentrade_signal_confidence",
                "Signal confidence distribution",
                &["strategy"],
                vec![0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]
            )?,
            order_latency: register_histogram_vec!(
                "opentrade_order_latency_ms",
                "Order submission latency in milliseconds",
                &["exchange"],
                vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0]
            )?,
            drawdown_pct: register_gauge!(
                "opentrade_drawdown_pct",
                "Current portfolio drawdown percentage"
            )?,
        })
    }

    pub fn record_trade(&self, strategy: &str, direction: &str, result: &str) {
        self.trades_total
            .with_label_values(&[strategy, direction, result])
            .inc();
    }

    pub fn update_equity(&self, equity: f64) {
        self.portfolio_equity.set(equity);
    }

    pub fn update_positions(&self, count: f64) {
        self.open_positions.set(count);
    }

    pub fn update_daily_pnl(&self, pnl: f64) {
        self.daily_pnl.set(pnl);
    }

    pub fn record_signal(&self, strategy: &str, confidence: f64) {
        self.signal_confidence
            .with_label_values(&[strategy])
            .observe(confidence);
    }

    pub fn record_latency(&self, exchange: &str, latency_ms: f64) {
        self.order_latency
            .with_label_values(&[exchange])
            .observe(latency_ms);
    }

    pub fn update_drawdown(&self, dd_pct: f64) {
        self.drawdown_pct.set(dd_pct);
    }
}

impl Default for TradingMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to create metrics")
    }
}

/// Gather all Prometheus metrics as text.
pub fn gather_metrics() -> String {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
