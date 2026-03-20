use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::indicators;
use ot_types::market::Candle;

/// A computed feature row for a single point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureRow {
    pub timestamp_ms: i64,
    pub close: Decimal,
    // Returns
    pub return_1: Option<Decimal>,
    pub return_5: Option<Decimal>,
    pub log_return_1: Option<Decimal>,
    // Trend / momentum
    pub sma_20: Option<Decimal>,
    pub sma_50: Option<Decimal>,
    pub ema_12: Option<Decimal>,
    pub ema_26: Option<Decimal>,
    pub macd: Option<Decimal>,
    pub rsi_14: Option<Decimal>,
    // Volatility
    pub atr_14: Option<Decimal>,
    pub bb_upper: Option<Decimal>,
    pub bb_middle: Option<Decimal>,
    pub bb_lower: Option<Decimal>,
    pub realized_vol_20: Option<Decimal>,
    // Derived
    pub bb_width: Option<Decimal>,
    pub price_vs_sma20: Option<Decimal>,
    pub price_vs_sma50: Option<Decimal>,
    pub trend_strength: Option<Decimal>,
    pub volume_sma_20: Option<Decimal>,
    pub volume_ratio: Option<Decimal>,
}

/// Compute a feature row from a buffer of candles.
/// The last candle in the slice is the current bar (no lookahead).
pub fn compute_features(candles: &[Candle]) -> Option<FeatureRow> {
    if candles.len() < 51 {
        return None; // Need at least 50+1 candles for SMA50
    }

    let closes: Vec<Decimal> = candles.iter().map(|c| c.close).collect();
    let highs: Vec<Decimal> = candles.iter().map(|c| c.high).collect();
    let lows: Vec<Decimal> = candles.iter().map(|c| c.low).collect();
    let volumes: Vec<Decimal> = candles.iter().map(|c| c.volume).collect();
    let current = candles.last()?;
    let n = closes.len();

    let return_1 = if n >= 2 {
        indicators::simple_return_pct(closes[n - 1], closes[n - 2])
    } else {
        None
    };

    let return_5 = if n >= 6 {
        indicators::simple_return_pct(closes[n - 1], closes[n - 6])
    } else {
        None
    };

    let log_return_1 = if n >= 2 {
        indicators::log_return(closes[n - 1], closes[n - 2])
    } else {
        None
    };

    let sma_20 = indicators::sma(&closes, 20);
    let sma_50 = indicators::sma(&closes, 50);
    let ema_12 = indicators::ema(&closes, 12);
    let ema_26 = indicators::ema(&closes, 26);
    let macd = indicators::macd(&closes, 12, 26);
    let rsi_14 = indicators::rsi(&closes, 14);
    let atr_14 = indicators::atr(&highs, &lows, &closes, 14);

    let bb = indicators::bollinger_bands(&closes, 20, rust_decimal_macros::dec!(2));
    let (bb_upper, bb_middle, bb_lower) = match bb {
        Some((u, m, l)) => (Some(u), Some(m), Some(l)),
        None => (None, None, None),
    };

    let bb_width = match (bb_upper, bb_lower, bb_middle) {
        (Some(u), Some(l), Some(m)) if m > Decimal::ZERO => Some((u - l) / m),
        _ => None,
    };

    // Compute returns for realized vol
    let returns: Vec<Decimal> = closes
        .windows(2)
        .filter_map(|w| indicators::simple_return_pct(w[1], w[0]))
        .collect();
    let realized_vol_20 = if returns.len() >= 20 {
        let recent_returns = &returns[returns.len() - 20..];
        indicators::realized_volatility(recent_returns, rust_decimal_macros::dec!(252))
    } else {
        None
    };

    let price_vs_sma20 = sma_20.and_then(|sma| {
        if sma > Decimal::ZERO {
            Some((current.close - sma) / sma * rust_decimal_macros::dec!(100))
        } else {
            None
        }
    });

    let price_vs_sma50 = sma_50.and_then(|sma| {
        if sma > Decimal::ZERO {
            Some((current.close - sma) / sma * rust_decimal_macros::dec!(100))
        } else {
            None
        }
    });

    let trend_strength = match (sma_20, sma_50) {
        (Some(s20), Some(s50)) if s50 > Decimal::ZERO => {
            Some((s20 - s50) / s50 * rust_decimal_macros::dec!(100))
        }
        _ => None,
    };

    let volume_sma_20 = indicators::sma(&volumes, 20);
    let volume_ratio = volume_sma_20.and_then(|vsma| {
        if vsma > Decimal::ZERO {
            Some(current.volume / vsma)
        } else {
            None
        }
    });

    Some(FeatureRow {
        timestamp_ms: ot_common::time_utils::datetime_to_ms(&current.close_time),
        close: current.close,
        return_1,
        return_5,
        log_return_1,
        sma_20,
        sma_50,
        ema_12,
        ema_26,
        macd,
        rsi_14,
        atr_14,
        bb_upper,
        bb_middle,
        bb_lower,
        realized_vol_20,
        bb_width,
        price_vs_sma20,
        price_vs_sma50,
        trend_strength,
        volume_sma_20,
        volume_ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ot_types::market::{MarketType, Symbol, Timeframe};
    use rust_decimal_macros::dec;

    fn make_candles(n: usize, base_price: Decimal) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = base_price + Decimal::from(i as u32);
                let t = Utc::now() + chrono::Duration::minutes(i as i64);
                Candle {
                    symbol: Symbol::new("BTCUSDT"),
                    market_type: MarketType::Spot,
                    timeframe: Timeframe::M1,
                    open_time: t,
                    close_time: t + chrono::Duration::seconds(60),
                    open: price,
                    high: price + dec!(5),
                    low: price - dec!(5),
                    close: price,
                    volume: dec!(100),
                    quote_volume: price * dec!(100),
                    trades: 50,
                }
            })
            .collect()
    }

    #[test]
    fn compute_features_needs_enough_data() {
        let candles = make_candles(30, dec!(50000));
        assert!(compute_features(&candles).is_none());
    }

    #[test]
    fn compute_features_with_enough_data() {
        let candles = make_candles(60, dec!(50000));
        let row = compute_features(&candles);
        assert!(row.is_some());
        let row = row.unwrap();
        assert!(row.sma_20.is_some());
        assert!(row.sma_50.is_some());
        assert!(row.rsi_14.is_some());
        assert!(row.macd.is_some());
    }
}
