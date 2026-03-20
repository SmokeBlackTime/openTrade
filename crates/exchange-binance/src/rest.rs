use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ot_common::OtError;
use ot_market_data::HistoricalDataProvider;
use ot_types::market::{Candle, MarketType, Symbol, Timeframe};

use crate::client::BinanceClient;

/// Historical data provider using Binance REST API.
pub struct BinanceHistoricalProvider {
    client: BinanceClient,
}

impl BinanceHistoricalProvider {
    pub fn new(client: BinanceClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HistoricalDataProvider for BinanceHistoricalProvider {
    async fn fetch_candles(
        &self,
        symbol: &Symbol,
        _market_type: MarketType,
        timeframe: Timeframe,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Candle>, OtError> {
        let start_ms = ot_common::time_utils::datetime_to_ms(&start);
        let end_ms = ot_common::time_utils::datetime_to_ms(&end);

        let mut all_candles = Vec::new();
        let mut current_start = start_ms;

        // Paginate through klines (max 1000 per request)
        loop {
            if current_start >= end_ms {
                break;
            }

            let candles = self
                .client
                .get_klines(
                    symbol.as_str(),
                    timeframe.as_binance_str(),
                    Some(current_start),
                    Some(end_ms),
                    Some(1000),
                )
                .await?;

            if candles.is_empty() {
                break;
            }

            let last_close_time = ot_common::time_utils::datetime_to_ms(
                &candles.last().unwrap().close_time,
            );
            current_start = last_close_time + 1;

            all_candles.extend(candles);

            // Rate limit: short delay between pagination requests
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Ok(all_candles)
    }
}
