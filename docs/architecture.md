# Architecture

## Design Principles

1. **Safety first**: Hard risk limits, kill switches, and no panics in trading paths
2. **Decimal arithmetic**: All monetary calculations use `rust_decimal` to avoid float errors
3. **Event-driven**: Candle events flow through features → strategy → risk → execution
4. **Trait-based abstraction**: Exchange adapters, strategies, and models are trait objects
5. **Same path for paper and live**: Only the exchange adapter differs

## Data Flow

```
Exchange WebSocket/REST
       │
       ▼
  Market Data Layer (normalize, buffer, detect staleness)
       │
       ▼
  Feature Pipeline (SMA, EMA, RSI, ATR, MACD, Bollinger, etc.)
       │
       ▼
  Strategy Engine (generate signals with confidence + metadata)
       │
       ▼
  Risk Engine (pre-trade checks, kill switches, limits)
       │
       ▼
  Portfolio Manager (position sizing, drawdown scaling)
       │
       ▼
  Execution Engine (order creation, submission, tracking)
       │
       ▼
  Exchange Adapter (paper: simulated fills / live: real API)
```

## Crate Dependencies

```
types ← common ← config
                ← market-data ← exchange-binance
                ← features
                ← strategy (depends on features)
                ← risk
                ← portfolio (depends on risk)
                ← execution
                ← models (depends on features)
                ← backtest (depends on strategy, risk, portfolio, execution)
                ← paper (depends on execution)
                ← live (depends on all above)
                ← storage
                ← telemetry
cli (depends on everything)
```

## Key Design Decisions

### Why Decimal instead of f64?
Financial calculations with floating point lead to representation errors that accumulate over time. For example, `0.1 + 0.2 != 0.3` in IEEE 754. Using `rust_decimal` eliminates this entire class of bugs.

### Why event-driven?
Event-driven architecture allows the same strategy code to work in backtesting and live mode. It also naturally handles asynchronous exchange events.

### Why trait-based exchange adapters?
This allows paper trading to use the exact same signal and risk path as live trading, with only the exchange interaction differing. It also makes adding new exchanges straightforward.

### Why hardcoded safety maxima?
Config-only risk limits can be accidentally set to dangerous values. The hardcoded absolute maxima in `AbsoluteSafetyLimits` cannot be overridden by any config file, providing a last line of defense.
