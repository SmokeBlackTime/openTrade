use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A normalized trading symbol (e.g., "BTCUSDT").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(pub String);

impl Symbol {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into().to_uppercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The market venue type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketType {
    Spot,
    UsdtFutures,
    CoinFutures,
}

impl fmt::Display for MarketType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spot => write!(f, "spot"),
            Self::UsdtFutures => write!(f, "usdt_futures"),
            Self::CoinFutures => write!(f, "coin_futures"),
        }
    }
}

/// Candle timeframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Timeframe {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

impl Timeframe {
    pub fn as_secs(&self) -> u64 {
        match self {
            Self::M1 => 60,
            Self::M5 => 300,
            Self::M15 => 900,
            Self::H1 => 3600,
            Self::H4 => 14400,
            Self::D1 => 86400,
        }
    }

    pub fn as_binance_str(&self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H4 => "4h",
            Self::D1 => "1d",
        }
    }
}

impl fmt::Display for Timeframe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_binance_str())
    }
}

/// A single OHLCV candle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timeframe: Timeframe,
    pub open_time: DateTime<Utc>,
    pub close_time: DateTime<Utc>,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub quote_volume: Decimal,
    pub trades: u64,
}

impl Candle {
    /// True range: max(high-low, |high-prev_close|, |low-prev_close|).
    /// Without previous close, returns high - low.
    pub fn range(&self) -> Decimal {
        self.high - self.low
    }

    pub fn body(&self) -> Decimal {
        (self.close - self.open).abs()
    }

    pub fn is_bullish(&self) -> bool {
        self.close > self.open
    }

    pub fn mid_price(&self) -> Decimal {
        (self.high + self.low) / Decimal::from(2)
    }

    pub fn vwap_approx(&self) -> Option<Decimal> {
        if self.volume > Decimal::ZERO {
            Some(self.quote_volume / self.volume)
        } else {
            None
        }
    }
}

/// A single trade from the exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketTrade {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timestamp: DateTime<Utc>,
    pub price: Decimal,
    pub quantity: Decimal,
    pub is_buyer_maker: bool,
    pub trade_id: u64,
}

/// Top-of-book snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopOfBook {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timestamp: DateTime<Utc>,
    pub best_bid: Decimal,
    pub best_bid_qty: Decimal,
    pub best_ask: Decimal,
    pub best_ask_qty: Decimal,
}

impl TopOfBook {
    pub fn spread(&self) -> Decimal {
        self.best_ask - self.best_bid
    }

    pub fn mid_price(&self) -> Decimal {
        (self.best_bid + self.best_ask) / Decimal::from(2)
    }

    pub fn spread_bps(&self) -> Option<Decimal> {
        let mid = self.mid_price();
        if mid > Decimal::ZERO {
            Some(self.spread() / mid * Decimal::from(10000))
        } else {
            None
        }
    }
}

/// Order book depth snapshot (top N levels).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookSnapshot {
    pub symbol: Symbol,
    pub market_type: MarketType,
    pub timestamp: DateTime<Utc>,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub last_update_id: u64,
}

/// A single price level in the order book.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: Decimal,
    pub quantity: Decimal,
}

/// Funding rate information for perpetual futures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingRate {
    pub symbol: Symbol,
    pub timestamp: DateTime<Utc>,
    pub rate: Decimal,
    pub next_funding_time: DateTime<Utc>,
}

/// Open interest data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenInterest {
    pub symbol: Symbol,
    pub timestamp: DateTime<Utc>,
    pub open_interest: Decimal,
    pub open_interest_value: Decimal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn symbol_normalizes_to_uppercase() {
        let s = Symbol::new("btcusdt");
        assert_eq!(s.as_str(), "BTCUSDT");
    }

    #[test]
    fn top_of_book_spread() {
        let tob = TopOfBook {
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            timestamp: Utc::now(),
            best_bid: dec!(50000),
            best_bid_qty: dec!(1),
            best_ask: dec!(50010),
            best_ask_qty: dec!(1),
        };
        assert_eq!(tob.spread(), dec!(10));
        assert_eq!(tob.mid_price(), dec!(50005));
    }

    #[test]
    fn timeframe_seconds() {
        assert_eq!(Timeframe::M1.as_secs(), 60);
        assert_eq!(Timeframe::H1.as_secs(), 3600);
        assert_eq!(Timeframe::D1.as_secs(), 86400);
    }
}
