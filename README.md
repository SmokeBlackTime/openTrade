# OpenTrade

**A production-grade autonomous cryptocurrency trading system built in Rust.**

> **DISCLAIMER**: Trading cryptocurrency involves substantial risk of loss and is not suitable for every investor. This software is provided for educational and experimental purposes only. No profitability is guaranteed. Past performance, including backtest results, does not guarantee future results. You are solely responsible for compliance with local regulations and exchange terms of service.

## What This Is

OpenTrade is an event-driven, modular trading platform designed for Binance spot and USDT-M futures. It provides:

- Real-time and historical market data ingestion
- Technical indicator and feature computation (all using `Decimal` — no float footguns)
- Multiple pluggable strategies (trend following, mean reversion, breakout, momentum)
- Regime detection and meta-strategy allocation
- Comprehensive risk management with hard limits and kill switches
- Kelly-inspired position sizing with volatility targeting
- Event-driven backtesting with realistic fees and slippage
- Paper trading using the same signal/risk path as live
- Structured logging, Prometheus metrics, and tracing
- Professional CLI with safety confirmations

## What This Is NOT

- A guaranteed money-making machine
- A system that can predict the future
- Something you should run with real money without extensive testing
- Financial advice

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI (opentrade)                       │
├──────────┬───────────┬──────────┬───────────┬───────────────┤
│ Backtest │   Paper   │   Live   │  Ingest   │    Report     │
├──────────┴───────────┴──────────┴───────────┴───────────────┤
│                    Trading Engine (live)                      │
├──────────┬───────────┬──────────┬───────────┬───────────────┤
│ Strategy │    Risk   │Portfolio │ Execution │    Models     │
│  Engine  │  Engine   │ Manager  │  Engine   │   (ML/AI)     │
├──────────┴───────────┴──────────┴───────────┴───────────────┤
│                    Feature Pipeline                           │
├─────────────────────────────────────────────────────────────┤
│                    Market Data Layer                          │
├──────────┬──────────────────────────────────────────────────┤
│ Exchange │   Storage (SQLite)   │      Telemetry            │
│ Adapters │                      │   (Prometheus/Tracing)    │
└──────────┴──────────────────────┴───────────────────────────┘
```

## Workspace Crates

| Crate | Purpose |
|-------|---------|
| `ot-types` | Core domain types: Symbol, Candle, Order, Position, Signal |
| `ot-common` | Error taxonomy, time utilities |
| `ot-config` | YAML config with hardcoded safety limits |
| `ot-market-data` | Data provider traits, candle buffering, staleness detection |
| `ot-exchange-binance` | Binance REST/WS client, HMAC auth |
| `ot-features` | Technical indicators (SMA, EMA, RSI, ATR, MACD, Bollinger) |
| `ot-strategy` | Strategy trait + implementations |
| `ot-models` | ML model trait, regime detection, ensemble scoring |
| `ot-risk` | Risk engine with kill switches and hard limits |
| `ot-portfolio` | Position sizing, drawdown-aware allocation |
| `ot-execution` | Order management, exchange adapter trait |
| `ot-backtest` | Event-driven backtester with realistic fills |
| `ot-paper` | Paper trading exchange adapter |
| `ot-live` | Live trading orchestrator |
| `ot-storage` | SQLite persistence for candles and trades |
| `ot-telemetry` | Structured logging and Prometheus metrics |
| `ot-cli` | CLI binary with all subcommands |

## Strategies

1. **Trend Following** — Dual SMA crossover with RSI filter and ATR-based stops
2. **Mean Reversion** — Bollinger Band + RSI reversal entries with mean target
3. **Breakout** — N-bar high/low breakout with volume confirmation
4. **Momentum** — 5-bar return momentum with RSI confirmation
5. **Meta Strategy** — Regime-aware strategy allocation

## Risk Management

- Hard maximum leverage (absolute cap: 20x, configurable lower)
- Daily loss limit with automatic halt
- Maximum drawdown circuit breaker
- Per-symbol and portfolio-wide position limits
- Confidence threshold gating
- Order rate limiting and rejection monitoring
- 10 independent kill switch types
- All risk limits bounded by hardcoded absolute maxima that config cannot override

## Quick Start

### Prerequisites

- Rust 1.75+ (stable)
- Binance API key (testnet recommended for testing)

### Build

```bash
cargo build --release
```

### Run Health Check

```bash
./target/release/opentrade --config config/default.yaml doctor
```

### Ingest Historical Data

```bash
export BINANCE_API_KEY=your_key
export BINANCE_API_SECRET=your_secret
./target/release/opentrade ingest --symbol BTCUSDT --timeframe 1h \
  --start 2024-01-01 --end 2024-12-31
```

### Run Backtest

```bash
./target/release/opentrade backtest --strategy trend_btc --symbol BTCUSDT \
  --start 2024-01-01 --end 2024-12-31
```

### Paper Trade

```bash
./target/release/opentrade paper
```

### Live Trade (requires --confirm flag)

```bash
./target/release/opentrade live --confirm
```

### Emergency Shutdown

```bash
./target/release/opentrade flatten --confirm
./target/release/opentrade cancel-all --confirm
```

## Docker

```bash
# Paper trading with monitoring stack
docker-compose up -d

# Access Grafana at http://localhost:3000 (admin/opentrade)
```

## Testing

```bash
cargo test --workspace
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ingest` | Fetch and store historical candle data |
| `backtest` | Run strategy backtest |
| `paper` | Paper trading mode |
| `live` | Live trading (requires --confirm) |
| `doctor` | System health check |
| `status` | Current trading status |
| `flatten` | Emergency: close all positions |
| `cancel-all` | Cancel all open orders |
| `explain-last-trade` | Show last trade decision details |
| `report` | Performance report |
| `stress-test` | Monte Carlo stress testing |
| `hyperopt` | Hyperparameter optimization |
| `replay` | Replay a historical session |

## Configuration

All configuration is in YAML. See `config/default.yaml` for the full schema.

**Security**: API keys are loaded from environment variables, never stored in config files. Secrets are never logged.

**Safety limits**: Risk parameters in config are bounded by hardcoded absolute maxima in the code that cannot be overridden. For example, max leverage cannot exceed 20x regardless of config.

## Known Limitations

- WebSocket streaming runs but requires manual integration with the trading loop for live execution
- ML models are rule-based baselines; real ML requires ONNX integration or external model serving
- Backtester uses bar-level simulation (not tick-level)
- No multi-exchange arbitrage (single exchange per deployment)
- Funding rate and liquidation logic for futures is approximate
- Walk-forward optimization framework is structured but needs the optimization loop
- Order book features require L2 data subscription

## Honest Assessment

The edge in any trading system, if it exists at all, comes from:
1. **Risk management** — not the signals
2. **Execution quality** — minimizing slippage and fees
3. **Regime awareness** — knowing when NOT to trade
4. **Capital preservation** — surviving drawdowns

Run this in paper trading mode for at least 3 months before considering real capital. Backtest results always look better than live performance.

## License

MIT
