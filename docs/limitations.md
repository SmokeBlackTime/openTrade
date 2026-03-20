# Limitations and Failure Modes

## Known Limitations

1. **Backtesting is not reality**: Backtest results systematically overestimate live performance due to:
   - Perfect fill assumptions (real markets have partial fills, requotes)
   - Bar-level simulation (intra-bar price paths are unknown)
   - No queue position modeling for limit orders
   - Survivorship bias in historical data

2. **ML models are baselines**: The included models are rule-based heuristics, not trained ML models. Real ML integration requires:
   - ONNX runtime or external model serving
   - Proper train/validation/test splits
   - Walk-forward validation
   - Feature drift monitoring

3. **Single exchange**: Currently supports Binance only. Multi-exchange arbitrage is not implemented.

4. **Network dependency**: All exchange operations require network connectivity. Network failures during order submission can lead to unknown order states.

5. **Clock drift**: Time-critical operations depend on system clock accuracy. NTP synchronization is assumed but not enforced.

## Potential Failure Modes

| Failure | Impact | Mitigation |
|---------|--------|------------|
| Exchange API outage | Cannot submit/cancel orders | Kill switch, graceful degradation |
| Network partition | Stale data, unknown order states | Staleness detection, reconciliation |
| Configuration error | Wrong risk limits | Hardcoded safety maxima |
| Strategy bug | Bad signals | Risk engine pre-trade checks |
| Database corruption | Lost trade history | Regular backups |
| Memory exhaustion | Crash | Bounded buffers, capacity limits |
| Clock drift | Wrong timestamps | Time sync checks |

## What Can Go Wrong in Live Trading

- Slippage exceeding estimates on large orders
- Flash crashes triggering stops at extreme prices
- Exchange maintenance windows with open positions
- API rate limiting during high-volatility periods
- Funding rate changes on futures positions
- Liquidation if leverage and margin are miscalculated
- Regulatory changes affecting exchange operations

## Risk Acknowledgment

This system operates under genuine market uncertainty. It cannot predict future prices. Markets can and do experience conditions that have never occurred in historical data. No amount of backtesting eliminates the risk of loss.

**Run paper trading for at least 3 months before any live deployment.**
