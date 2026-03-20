//! Persistent storage layer for OpenTrade using SQLite.
//!
//! Stores candles, trades, positions, and configuration state.

use chrono::{DateTime, Utc};
use ot_common::OtError;
use ot_types::market::*;
use ot_types::trade::TradeRecord;
use rust_decimal::Decimal;
use rusqlite::{params, Connection};
use std::path::Path;
use std::str::FromStr;

/// SQLite-based storage backend.
pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn new(path: &Path) -> Result<Self, OtError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| OtError::Storage(e.to_string()))?;
        }

        let conn = Connection::open(path)
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    pub fn in_memory() -> Result<Self, OtError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| OtError::Storage(e.to_string()))?;
        let storage = Self { conn };
        storage.init_schema()?;
        Ok(storage)
    }

    fn init_schema(&self) -> Result<(), OtError> {
        self.conn
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS candles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                symbol TEXT NOT NULL,
                market_type TEXT NOT NULL,
                timeframe TEXT NOT NULL,
                open_time TEXT NOT NULL,
                close_time TEXT NOT NULL,
                open TEXT NOT NULL,
                high TEXT NOT NULL,
                low TEXT NOT NULL,
                close TEXT NOT NULL,
                volume TEXT NOT NULL,
                quote_volume TEXT NOT NULL,
                trades INTEGER NOT NULL,
                UNIQUE(symbol, timeframe, open_time)
            );

            CREATE TABLE IF NOT EXISTS trade_journal (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                trade_id TEXT NOT NULL UNIQUE,
                client_order_id TEXT NOT NULL,
                symbol TEXT NOT NULL,
                market_type TEXT NOT NULL,
                side TEXT NOT NULL,
                quantity TEXT NOT NULL,
                price TEXT NOT NULL,
                commission TEXT NOT NULL,
                commission_asset TEXT NOT NULL,
                realized_pnl TEXT,
                strategy_name TEXT NOT NULL,
                timestamp TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS system_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_candles_symbol_time
                ON candles(symbol, timeframe, open_time);
            CREATE INDEX IF NOT EXISTS idx_trades_symbol
                ON trade_journal(symbol, timestamp);
            CREATE INDEX IF NOT EXISTS idx_trades_strategy
                ON trade_journal(strategy_name, timestamp);
            ",
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Store a batch of candles (upsert).
    pub fn store_candles(&self, candles: &[Candle]) -> Result<usize, OtError> {
        let mut count = 0;
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| OtError::Storage(e.to_string()))?;

        for candle in candles {
            let result = tx.execute(
                "INSERT OR REPLACE INTO candles
                 (symbol, market_type, timeframe, open_time, close_time,
                  open, high, low, close, volume, quote_volume, trades)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    candle.symbol.as_str(),
                    candle.market_type.to_string(),
                    candle.timeframe.to_string(),
                    candle.open_time.to_rfc3339(),
                    candle.close_time.to_rfc3339(),
                    candle.open.to_string(),
                    candle.high.to_string(),
                    candle.low.to_string(),
                    candle.close.to_string(),
                    candle.volume.to_string(),
                    candle.quote_volume.to_string(),
                    candle.trades as i64,
                ],
            );
            if result.is_ok() {
                count += 1;
            }
        }

        tx.commit().map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(count)
    }

    /// Load candles for a symbol/timeframe in a date range.
    pub fn load_candles(
        &self,
        symbol: &str,
        timeframe: &str,
        start: &DateTime<Utc>,
        end: &DateTime<Utc>,
    ) -> Result<Vec<Candle>, OtError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT symbol, market_type, timeframe, open_time, close_time,
                        open, high, low, close, volume, quote_volume, trades
                 FROM candles
                 WHERE symbol = ?1 AND timeframe = ?2
                   AND open_time >= ?3 AND open_time <= ?4
                 ORDER BY open_time ASC",
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let candles = stmt
            .query_map(
                params![
                    symbol,
                    timeframe,
                    start.to_rfc3339(),
                    end.to_rfc3339()
                ],
                |row| {
                    let symbol_str: String = row.get(0)?;
                    let market_type_str: String = row.get(1)?;
                    let timeframe_str: String = row.get(2)?;
                    let open_time_str: String = row.get(3)?;
                    let close_time_str: String = row.get(4)?;
                    let open_str: String = row.get(5)?;
                    let high_str: String = row.get(6)?;
                    let low_str: String = row.get(7)?;
                    let close_str: String = row.get(8)?;
                    let volume_str: String = row.get(9)?;
                    let quote_volume_str: String = row.get(10)?;
                    let trades_val: i64 = row.get(11)?;

                    Ok(Candle {
                        symbol: Symbol::new(&symbol_str),
                        market_type: match market_type_str.as_str() {
                            "usdt_futures" => MarketType::UsdtFutures,
                            "coin_futures" => MarketType::CoinFutures,
                            _ => MarketType::Spot,
                        },
                        timeframe: match timeframe_str.as_str() {
                            "1m" => Timeframe::M1,
                            "5m" => Timeframe::M5,
                            "15m" => Timeframe::M15,
                            "4h" => Timeframe::H4,
                            "1d" => Timeframe::D1,
                            _ => Timeframe::H1,
                        },
                        open_time: DateTime::parse_from_rfc3339(&open_time_str)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        close_time: DateTime::parse_from_rfc3339(&close_time_str)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        open: Decimal::from_str(&open_str).unwrap_or(Decimal::ZERO),
                        high: Decimal::from_str(&high_str).unwrap_or(Decimal::ZERO),
                        low: Decimal::from_str(&low_str).unwrap_or(Decimal::ZERO),
                        close: Decimal::from_str(&close_str).unwrap_or(Decimal::ZERO),
                        volume: Decimal::from_str(&volume_str).unwrap_or(Decimal::ZERO),
                        quote_volume: Decimal::from_str(&quote_volume_str).unwrap_or(Decimal::ZERO),
                        trades: trades_val as u64,
                    })
                },
            )
            .map_err(|e| OtError::Storage(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| OtError::Storage(e.to_string()))?;

        Ok(candles)
    }

    /// Store a trade in the journal.
    pub fn store_trade(&self, trade: &TradeRecord) -> Result<(), OtError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO trade_journal
                 (trade_id, client_order_id, symbol, market_type, side,
                  quantity, price, commission, commission_asset,
                  realized_pnl, strategy_name, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    trade.trade_id,
                    trade.client_order_id.0,
                    trade.symbol.as_str(),
                    trade.market_type.to_string(),
                    trade.side.to_string(),
                    trade.quantity.to_string(),
                    trade.price.to_string(),
                    trade.commission.to_string(),
                    trade.commission_asset,
                    trade.realized_pnl.map(|p| p.to_string()),
                    trade.strategy_name,
                    trade.timestamp.to_rfc3339(),
                ],
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Get system state value.
    pub fn get_state(&self, key: &str) -> Result<Option<String>, OtError> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM system_state WHERE key = ?1")
            .map_err(|e| OtError::Storage(e.to_string()))?;

        let result = stmt
            .query_row(params![key], |row| row.get(0))
            .ok();
        Ok(result)
    }

    /// Set system state value.
    pub fn set_state(&self, key: &str, value: &str) -> Result<(), OtError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO system_state (key, value, updated_at)
                 VALUES (?1, ?2, ?3)",
                params![key, value, Utc::now().to_rfc3339()],
            )
            .map_err(|e| OtError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_candle(symbol: &str, i: u64) -> Candle {
        let t = Utc::now() + chrono::Duration::minutes(i as i64);
        Candle {
            symbol: Symbol::new(symbol),
            market_type: MarketType::Spot,
            timeframe: Timeframe::H1,
            open_time: t,
            close_time: t + chrono::Duration::seconds(3600),
            open: dec!(50000) + Decimal::from(i),
            high: dec!(50100) + Decimal::from(i),
            low: dec!(49900) + Decimal::from(i),
            close: dec!(50050) + Decimal::from(i),
            volume: dec!(100),
            quote_volume: dec!(5000000),
            trades: 500,
        }
    }

    #[test]
    fn store_and_load_candles() {
        let storage = Storage::in_memory().unwrap();
        let candles: Vec<Candle> = (0..10).map(|i| make_candle("BTCUSDT", i)).collect();
        let stored = storage.store_candles(&candles).unwrap();
        assert_eq!(stored, 10);

        let start = candles[0].open_time - chrono::Duration::seconds(1);
        let end = candles[9].open_time + chrono::Duration::seconds(1);
        let loaded = storage.load_candles("BTCUSDT", "1h", &start, &end).unwrap();
        assert_eq!(loaded.len(), 10);
    }

    #[test]
    fn system_state_roundtrip() {
        let storage = Storage::in_memory().unwrap();
        storage.set_state("last_candle_time", "2024-01-01").unwrap();
        let val = storage.get_state("last_candle_time").unwrap();
        assert_eq!(val, Some("2024-01-01".to_string()));
    }
}
