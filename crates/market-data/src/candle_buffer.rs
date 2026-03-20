use std::collections::VecDeque;

use ot_types::market::Candle;

/// Fixed-size ring buffer for candles, ensuring no lookahead.
/// Oldest candles are dropped when capacity is exceeded.
pub struct CandleBuffer {
    candles: VecDeque<Candle>,
    capacity: usize,
}

impl CandleBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            candles: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new candle. Must be chronologically ordered.
    pub fn push(&mut self, candle: Candle) {
        if let Some(last) = self.candles.back() {
            debug_assert!(
                candle.open_time >= last.open_time,
                "Candles must be in chronological order"
            );
        }
        if self.candles.len() >= self.capacity {
            self.candles.pop_front();
        }
        self.candles.push_back(candle);
    }

    pub fn len(&self) -> usize {
        self.candles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candles.is_empty()
    }

    /// Get the last N candles (most recent last). Returns fewer if not enough data.
    pub fn last_n(&self, n: usize) -> Vec<&Candle> {
        let start = self.candles.len().saturating_sub(n);
        self.candles.range(start..).collect()
    }

    /// Get the most recent candle.
    pub fn latest(&self) -> Option<&Candle> {
        self.candles.back()
    }

    /// Get all candles as a slice-like iterator.
    pub fn iter(&self) -> impl Iterator<Item = &Candle> {
        self.candles.iter()
    }

    /// Extract close prices for the last N candles.
    pub fn closes(&self, n: usize) -> Vec<rust_decimal::Decimal> {
        self.last_n(n).iter().map(|c| c.close).collect()
    }

    /// Extract volumes for the last N candles.
    pub fn volumes(&self, n: usize) -> Vec<rust_decimal::Decimal> {
        self.last_n(n).iter().map(|c| c.volume).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ot_types::market::{MarketType, Symbol, Timeframe};
    use rust_decimal_macros::dec;

    fn make_candle(close: rust_decimal::Decimal, offset_mins: i64) -> Candle {
        let t = Utc::now() + chrono::Duration::minutes(offset_mins);
        Candle {
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            timeframe: Timeframe::M1,
            open_time: t,
            close_time: t + chrono::Duration::seconds(60),
            open: close,
            high: close + dec!(10),
            low: close - dec!(10),
            close,
            volume: dec!(100),
            quote_volume: dec!(5000000),
            trades: 500,
        }
    }

    #[test]
    fn buffer_capacity_enforced() {
        let mut buf = CandleBuffer::new(3);
        for i in 0..5 {
            buf.push(make_candle(dec!(50000) + rust_decimal::Decimal::from(i), i));
        }
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn last_n_returns_most_recent() {
        let mut buf = CandleBuffer::new(10);
        for i in 0..5 {
            buf.push(make_candle(dec!(50000) + rust_decimal::Decimal::from(i), i));
        }
        let last2 = buf.last_n(2);
        assert_eq!(last2.len(), 2);
        assert_eq!(last2[1].close, dec!(50004));
    }

    #[test]
    fn closes_extraction() {
        let mut buf = CandleBuffer::new(10);
        for i in 0..3 {
            buf.push(make_candle(rust_decimal::Decimal::from(100 + i), i));
        }
        let closes = buf.closes(3);
        assert_eq!(closes, vec![dec!(100), dec!(101), dec!(102)]);
    }
}
