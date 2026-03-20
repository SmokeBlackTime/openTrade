# Risk Model

## Philosophy

The risk engine's primary objective is **capital preservation**. When uncertainty is elevated, reduce exposure. When operational integrity is degraded, stop trading safely. Never override hard risk constraints.

## Risk Controls

### Pre-Trade Checks (in order)

1. **Kill switch check**: Any triggered kill switch blocks all orders
2. **Confidence threshold**: Signal must meet minimum confidence
3. **Daily loss limit**: Halt if daily PnL drops below threshold
4. **Max drawdown**: Circuit breaker on peak-to-trough drawdown
5. **Max open positions**: Portfolio-wide cap
6. **Daily trade count**: Prevents overtrading
7. **Order rate limit**: Per-minute cap
8. **Single order size**: Notional cap per order
9. **Per-symbol position size**: Maximum exposure per symbol
10. **Total notional exposure**: Portfolio-wide exposure cap
11. **Leverage check**: Effective leverage vs maximum

### Kill Switches

| Switch | Triggers When |
|--------|--------------|
| Global | Manual trigger or cascading failure |
| Per-Symbol | Symbol-specific anomaly detected |
| Exchange Connectivity | Connection lost or repeated timeouts |
| Stale Market Data | No price update beyond max age |
| Model Confidence | All models report low confidence |
| Extreme Volatility | ATR exceeds N× normal |
| Order Rejection Anomaly | Too many rejections per hour |
| Runaway Trading | Unusual order submission rate |
| Daily Loss Limit | Daily PnL exceeds threshold |
| Max Drawdown | Peak-to-trough exceeds threshold |

### Position Sizing

1. **Risk-per-trade**: Size based on % of equity risked and stop distance
2. **Kelly fraction**: Cap at fractional Kelly (default 0.25)
3. **Volatility targeting**: Scale down when ATR is high
4. **Drawdown de-risking**: Linear reduction starting at 5% drawdown
5. **Concentration limit**: Max allocation per symbol
6. **Leverage limit**: Remaining capacity within portfolio leverage cap

### Absolute Safety Maxima (Hardcoded)

These cannot be overridden by configuration:

| Parameter | Absolute Max |
|-----------|-------------|
| Leverage | 20× |
| Daily loss | 10% |
| Drawdown | 25% |
| Open positions | 50 |
| Single order | $1,000,000 |
| Total exposure | $10,000,000 |
| Trades per day | 5,000 |
