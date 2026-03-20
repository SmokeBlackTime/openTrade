//! Market data ingestion layer for OpenTrade.
//!
//! Provides traits and implementations for:
//! - Historical OHLCV ingestion (REST)
//! - Real-time streaming (WebSocket)
//! - Data normalization and staleness detection
//! - Gap detection and resync

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ot_types::market::{Candle, MarketType, OrderBookSnapshot, Symbol, Timeframe, TopOfBook};
use std::sync::Arc;

pub mod candle_buffer;
pub mod staleness;

/// Trait for historical data providers.
#[async_trait]
pub trait HistoricalDataProvider: Send + Sync {
    async fn fetch_candles(
        &self,
        symbol: &Symbol,
        market_type: MarketType,
        timeframe: Timeframe,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, ot_common::OtError>;
}

/// Trait for real-time data streams.
#[async_trait]
pub trait LiveDataStream: Send + Sync {
    async fn subscribe_candles(
        &self,
        symbol: &Symbol,
        market_type: MarketType,
        timeframe: Timeframe,
    ) -> Result<tokio::sync::mpsc::Receiver<Candle>, ot_common::OtError>;

    async fn subscribe_top_of_book(
        &self,
        symbol: &Symbol,
        market_type: MarketType,
    ) -> Result<tokio::sync::mpsc::Receiver<TopOfBook>, ot_common::OtError>;

    async fn subscribe_order_book(
        &self,
        symbol: &Symbol,
        market_type: MarketType,
    ) -> Result<tokio::sync::mpsc::Receiver<OrderBookSnapshot>, ot_common::OtError>;
}

/// Aggregated data provider combining historical + live.
pub struct MarketDataService {
    pub historical: Arc<dyn HistoricalDataProvider>,
    pub live: Arc<dyn LiveDataStream>,
}

// Re-export async_trait for downstream use
pub use async_trait::async_trait as market_data_async_trait;
