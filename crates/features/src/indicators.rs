use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// Simple Moving Average.
pub fn sma(values: &[Decimal], period: usize) -> Option<Decimal> {
    if values.len() < period || period == 0 {
        return None;
    }
    let slice = &values[values.len() - period..];
    let sum: Decimal = slice.iter().copied().sum();
    Some(sum / Decimal::from(period as u32))
}

/// Exponential Moving Average (approximation using Decimal).
pub fn ema(values: &[Decimal], period: usize) -> Option<Decimal> {
    if values.len() < period || period == 0 {
        return None;
    }
    let multiplier = dec!(2) / (Decimal::from(period as u32) + dec!(1));
    let mut ema_val = sma(&values[..period], period)?;
    for &val in &values[period..] {
        ema_val = (val - ema_val) * multiplier + ema_val;
    }
    Some(ema_val)
}

/// Relative Strength Index.
pub fn rsi(closes: &[Decimal], period: usize) -> Option<Decimal> {
    if closes.len() < period + 1 || period == 0 {
        return None;
    }

    let mut gains = Vec::new();
    let mut losses = Vec::new();

    for i in 1..closes.len() {
        let change = closes[i] - closes[i - 1];
        if change > dec!(0) {
            gains.push(change);
            losses.push(dec!(0));
        } else {
            gains.push(dec!(0));
            losses.push(change.abs());
        }
    }

    // Initial average gain/loss
    let first_gains: Decimal = gains[..period].iter().sum();
    let first_losses: Decimal = losses[..period].iter().sum();
    let period_dec = Decimal::from(period as u32);

    let mut avg_gain = first_gains / period_dec;
    let mut avg_loss = first_losses / period_dec;

    // Smoothed RMA
    for i in period..gains.len() {
        avg_gain = (avg_gain * (period_dec - dec!(1)) + gains[i]) / period_dec;
        avg_loss = (avg_loss * (period_dec - dec!(1)) + losses[i]) / period_dec;
    }

    if avg_loss == dec!(0) {
        return Some(dec!(100));
    }

    let rs = avg_gain / avg_loss;
    Some(dec!(100) - dec!(100) / (dec!(1) + rs))
}

/// Average True Range.
pub fn atr(
    highs: &[Decimal],
    lows: &[Decimal],
    closes: &[Decimal],
    period: usize,
) -> Option<Decimal> {
    if highs.len() < period + 1
        || highs.len() != lows.len()
        || highs.len() != closes.len()
    {
        return None;
    }

    let mut true_ranges = Vec::with_capacity(highs.len() - 1);
    for i in 1..highs.len() {
        let hl = highs[i] - lows[i];
        let hc = (highs[i] - closes[i - 1]).abs();
        let lc = (lows[i] - closes[i - 1]).abs();
        true_ranges.push(hl.max(hc).max(lc));
    }

    sma(&true_ranges, period)
}

/// MACD line (fast EMA - slow EMA).
pub fn macd(
    closes: &[Decimal],
    fast: usize,
    slow: usize,
) -> Option<Decimal> {
    let fast_ema = ema(closes, fast)?;
    let slow_ema = ema(closes, slow)?;
    Some(fast_ema - slow_ema)
}

/// Full MACD: returns (macd_line, signal_line, histogram).
/// signal_period is typically 9.
/// Requires closes.len() >= slow + signal_period.
/// O(n * slow) — acceptable for single candle-per-tick calls on buffers up to 500.
pub fn macd_full(
    closes: &[Decimal],
    fast: usize,
    slow: usize,
    signal_period: usize,
) -> Option<(Decimal, Decimal, Decimal)> {
    if fast >= slow {
        return None;
    }
    if closes.len() < slow + signal_period {
        return None;
    }
    // Build a series of MACD values for the last `signal_period + 1` points
    // so we can compute EMA(signal_period) over them.
    let macd_series: Vec<Decimal> = (slow..=closes.len())
        .filter_map(|i| macd(&closes[..i], fast, slow))
        .collect();
    if macd_series.len() < signal_period {
        return None;
    }
    let macd_line = *macd_series.last()?;
    let signal_line = ema(&macd_series, signal_period)?;
    Some((macd_line, signal_line, macd_line - signal_line))
}

/// Bollinger Bands: (upper, middle, lower).
pub fn bollinger_bands(
    closes: &[Decimal],
    period: usize,
    num_std: Decimal,
) -> Option<(Decimal, Decimal, Decimal)> {
    let middle = sma(closes, period)?;
    let std_dev = std_deviation(closes, period)?;
    let upper = middle + num_std * std_dev;
    let lower = middle - num_std * std_dev;
    Some((upper, middle, lower))
}

/// Standard deviation of last `period` values.
pub fn std_deviation(values: &[Decimal], period: usize) -> Option<Decimal> {
    if values.len() < period || period < 2 {
        return None;
    }
    let slice = &values[values.len() - period..];
    let mean = sma(values, period)?;
    let variance: Decimal = slice
        .iter()
        .map(|v| {
            let diff = *v - mean;
            diff * diff
        })
        .sum::<Decimal>()
        / Decimal::from((period - 1) as u32);

    // Decimal sqrt approximation via Newton's method
    decimal_sqrt(variance)
}

/// Newton's method square root for Decimal.
pub fn decimal_sqrt(value: Decimal) -> Option<Decimal> {
    if value < dec!(0) {
        return None;
    }
    if value == dec!(0) {
        return Some(dec!(0));
    }

    let mut guess = value / dec!(2);
    for _ in 0..50 {
        let new_guess = (guess + value / guess) / dec!(2);
        if (new_guess - guess).abs() < dec!(0.00000001) {
            return Some(new_guess);
        }
        guess = new_guess;
    }
    Some(guess)
}

/// Log return: ln(close / prev_close) approximated.
/// Uses ln(1+x) ≈ x - x²/2 + x³/3 for |x| < 1.
pub fn log_return(current: Decimal, previous: Decimal) -> Option<Decimal> {
    if previous <= dec!(0) || current <= dec!(0) {
        return None;
    }
    let ratio = current / previous;
    // Simple approximation: use (ratio - 1) for small moves
    // For larger moves, use the series expansion
    let x = ratio - dec!(1);
    if x.abs() > dec!(0.5) {
        // Fall back to simple return for large moves
        Some(x)
    } else {
        // ln(1+x) ≈ x - x²/2 + x³/3 - x⁴/4
        Some(x - x * x / dec!(2) + x * x * x / dec!(3) - x * x * x * x / dec!(4))
    }
}

/// Returns as percentage: (current - previous) / previous * 100.
pub fn simple_return_pct(current: Decimal, previous: Decimal) -> Option<Decimal> {
    if previous == dec!(0) {
        return None;
    }
    Some((current - previous) / previous * dec!(100))
}

/// Order book imbalance: (bid_qty - ask_qty) / (bid_qty + ask_qty).
pub fn order_book_imbalance(bid_qty: Decimal, ask_qty: Decimal) -> Option<Decimal> {
    let total = bid_qty + ask_qty;
    if total == dec!(0) {
        return None;
    }
    Some((bid_qty - ask_qty) / total)
}

/// Realized volatility from returns.
pub fn realized_volatility(returns: &[Decimal], annualization_factor: Decimal) -> Option<Decimal> {
    if returns.len() < 2 {
        return None;
    }
    let mean: Decimal = returns.iter().sum::<Decimal>() / Decimal::from(returns.len() as u32);
    let variance: Decimal = returns
        .iter()
        .map(|r| {
            let d = *r - mean;
            d * d
        })
        .sum::<Decimal>()
        / Decimal::from((returns.len() - 1) as u32);

    let std = decimal_sqrt(variance)?;
    let ann_factor_sqrt = decimal_sqrt(annualization_factor)?;
    Some(std * ann_factor_sqrt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sma_basic() {
        let values: Vec<Decimal> = (1..=5).map(|i| Decimal::from(i)).collect();
        assert_eq!(sma(&values, 5), Some(dec!(3)));
        assert_eq!(sma(&values, 3), Some(dec!(4)));
    }

    #[test]
    fn sma_insufficient_data() {
        let values = vec![dec!(1), dec!(2)];
        assert_eq!(sma(&values, 5), None);
    }

    #[test]
    fn ema_converges() {
        let values: Vec<Decimal> = (1..=20).map(|i| Decimal::from(i)).collect();
        let result = ema(&values, 10);
        assert!(result.is_some());
        let ema_val = result.unwrap();
        // EMA should be between SMA and latest value
        assert!(ema_val > dec!(10));
        assert!(ema_val < dec!(20));
    }

    #[test]
    fn rsi_range() {
        let closes: Vec<Decimal> = vec![
            dec!(44), dec!(44.34), dec!(44.09), dec!(43.61), dec!(44.33),
            dec!(44.83), dec!(45.10), dec!(45.42), dec!(45.84), dec!(46.08),
            dec!(45.89), dec!(46.03), dec!(45.61), dec!(46.28), dec!(46.28),
        ];
        let result = rsi(&closes, 14);
        assert!(result.is_some());
        let rsi_val = result.unwrap();
        assert!(rsi_val >= dec!(0) && rsi_val <= dec!(100));
    }

    #[test]
    fn atr_basic() {
        let highs = vec![dec!(12), dec!(13), dec!(14), dec!(13), dec!(15)];
        let lows = vec![dec!(10), dec!(11), dec!(12), dec!(11), dec!(13)];
        let closes = vec![dec!(11), dec!(12), dec!(13), dec!(12), dec!(14)];
        let result = atr(&highs, &lows, &closes, 3);
        assert!(result.is_some());
    }

    #[test]
    fn macd_basic() {
        let closes: Vec<Decimal> = (1..=30).map(|i| Decimal::from(i * 10)).collect();
        let result = macd(&closes, 12, 26);
        assert!(result.is_some());
    }

    #[test]
    fn bollinger_bands_symmetric() {
        let closes: Vec<Decimal> = vec![dec!(100); 20];
        let (upper, middle, lower) = bollinger_bands(&closes, 20, dec!(2)).unwrap();
        assert_eq!(middle, dec!(100));
        // With constant prices, std dev is 0, bands collapse
        assert_eq!(upper, dec!(100));
        assert_eq!(lower, dec!(100));
    }

    #[test]
    fn order_book_imbalance_balanced() {
        let result = order_book_imbalance(dec!(100), dec!(100));
        assert_eq!(result, Some(dec!(0)));
    }

    #[test]
    fn order_book_imbalance_bid_heavy() {
        let result = order_book_imbalance(dec!(80), dec!(20));
        assert_eq!(result, Some(dec!(0.6)));
    }

    #[test]
    fn log_return_positive() {
        let result = log_return(dec!(105), dec!(100)).unwrap();
        // Should be approximately 0.0488
        assert!(result > dec!(0.04) && result < dec!(0.06));
    }

    #[test]
    fn simple_return_pct_basic() {
        assert_eq!(
            simple_return_pct(dec!(110), dec!(100)),
            Some(dec!(10))
        );
    }

    #[test]
    fn decimal_sqrt_known_values() {
        let four = decimal_sqrt(dec!(4)).unwrap();
        assert!((four - dec!(2)).abs() < dec!(0.0001));

        let nine = decimal_sqrt(dec!(9)).unwrap();
        assert!((nine - dec!(3)).abs() < dec!(0.0001));
    }

    #[test]
    fn macd_full_returns_three_components() {
        let closes: Vec<Decimal> = (1..=50).map(|i| Decimal::from(i * 100)).collect();
        let result = macd_full(&closes, 12, 26, 9);
        assert!(result.is_some(), "macd_full should return Some with 50 candles");
        let (line, signal, hist) = result.unwrap();
        // Verify histogram is computed correctly
        assert_eq!(hist, line - signal);
    }

    #[test]
    fn macd_full_boundary_min_data() {
        // Test with exactly slow + signal_period candles (26 + 9 = 35)
        let closes: Vec<Decimal> = (1..=35).map(|i| Decimal::from(i * 100)).collect();
        let result = macd_full(&closes, 12, 26, 9);
        assert!(result.is_some(), "macd_full should return Some with exactly 35 candles");
        let (line, signal, hist) = result.unwrap();
        // Verify histogram computation
        assert_eq!(hist, line - signal);
    }

    #[test]
    fn macd_full_needs_enough_data() {
        let closes: Vec<Decimal> = (1..=30).map(|i| Decimal::from(i)).collect();
        // 26 + 9 = 35 required, only 30 given
        assert!(macd_full(&closes, 12, 26, 9).is_none());
    }

    #[test]
    fn macd_full_fast_ge_slow_invalid() {
        let closes: Vec<Decimal> = (1..=50).map(|i| Decimal::from(i)).collect();
        // fast >= slow should return None
        assert!(macd_full(&closes, 26, 26, 9).is_none());
        assert!(macd_full(&closes, 30, 26, 9).is_none());
    }
}
