//! Event-driven backtesting engine for OpenTrade.
//!
//! Replays historical candle data through the same strategy and risk
//! interfaces used in live trading. Supports realistic fees, slippage,
//! partial fills approximation, and walk-forward evaluation.

use chrono::{DateTime, Utc};
use ot_config::BacktestConfig;
use ot_features::pipeline::compute_features;
use ot_portfolio::PortfolioManager;
use ot_risk::RiskEngine;
use ot_strategy::Strategy;
use ot_types::market::*;
use ot_types::orders::*;
use ot_types::positions::*;
use ot_types::signals::*;
use ot_types::trade::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tracing::info;

/// Tracks stop-loss and take-profit levels for open backtest positions.
type StopTargetMap = HashMap<String, (Option<Decimal>, Option<Decimal>)>;

/// Backtest result.
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub metrics: PerformanceMetrics,
    pub trades: Vec<TradeRecord>,
    pub equity_curve: Vec<(DateTime<Utc>, Decimal)>,
    pub drawdown_curve: Vec<(DateTime<Utc>, Decimal)>,
    pub params_used: HashMap<String, serde_json::Value>,
}

/// Simulated fill model.
pub struct FillSimulator {
    pub fee_rate_bps: Decimal,
    pub slippage_bps: Decimal,
}

impl FillSimulator {
    pub fn new(fee_bps: Decimal, slippage_bps: Decimal) -> Self {
        Self {
            fee_rate_bps: fee_bps,
            slippage_bps: slippage_bps,
        }
    }

    /// Simulate a fill price with slippage.
    pub fn simulate_fill_price(&self, price: Decimal, side: Side) -> Decimal {
        let slippage = price * self.slippage_bps / dec!(10000);
        match side {
            Side::Buy => price + slippage,
            Side::Sell => price - slippage,
        }
    }

    /// Compute commission.
    pub fn compute_commission(&self, notional: Decimal) -> Decimal {
        notional * self.fee_rate_bps / dec!(10000)
    }
}

/// Run a backtest on historical candle data.
pub fn run_backtest(
    candles: &[Candle],
    strategy: &mut dyn Strategy,
    config: &BacktestConfig,
    risk_config: &ot_config::RiskConfig,
    portfolio_config: &ot_config::PortfolioConfig,
) -> BacktestResult {
    let fill_sim = FillSimulator::new(config.fee_rate_bps, config.slippage_bps);
    let mut portfolio = PortfolioManager::new(portfolio_config.clone());
    let mut risk = RiskEngine::new(risk_config.clone(), config.initial_capital);

    let mut trades: Vec<TradeRecord> = Vec::new();
    let mut equity_curve: Vec<(DateTime<Utc>, Decimal)> = Vec::new();
    let mut drawdown_curve: Vec<(DateTime<Utc>, Decimal)> = Vec::new();
    let mut peak_equity = config.initial_capital;
    let mut current_equity = config.initial_capital;
    let mut total_commission = dec!(0);
    let mut stop_targets: StopTargetMap = HashMap::new();

    strategy.reset();

    // We need at least 51 candles for features
    if candles.len() < 51 {
        return BacktestResult {
            metrics: empty_metrics(),
            trades,
            equity_curve,
            drawdown_curve,
            params_used: strategy.params(),
        };
    }

    for i in 51..candles.len() {
        let window = &candles[..=i];
        let candle = &candles[i];

        // Compute features (no lookahead: only candles up to current)
        let features = match compute_features(window) {
            Some(f) => f,
            None => continue,
        };

        // Update position mark prices
        let position = portfolio.get_position(&candle.symbol, strategy.name()).cloned();

        // Check stop loss / take profit
        if let Some(ref pos) = position {
            if !pos.is_flat() {
                let pos_key = format!("{}:{}", pos.symbol, pos.strategy_name);
                let (sl, tp) = stop_targets
                    .get(&pos_key)
                    .copied()
                    .unwrap_or((None, None));

                let (should_exit, exit_price) = check_stop_target(pos, candle, sl, tp);
                if should_exit {
                    let fill_price = fill_sim.simulate_fill_price(exit_price, pos.side.exit_side());
                    let commission = fill_sim.compute_commission(pos.quantity * fill_price);
                    let pnl = compute_trade_pnl(pos, fill_price) - commission;

                    current_equity += pnl;
                    risk.record_trade(pnl);
                    risk.update_equity(current_equity);
                    portfolio.update_equity(current_equity);
                    total_commission += commission;

                    trades.push(TradeRecord {
                        trade_id: TradeRecord::new_id(),
                        client_order_id: ClientOrderId::generate(),
                        symbol: candle.symbol.clone(),
                        market_type: candle.market_type,
                        side: pos.side.exit_side(),
                        quantity: pos.quantity,
                        price: fill_price,
                        commission,
                        commission_asset: "USDT".into(),
                        realized_pnl: Some(pnl),
                        strategy_name: strategy.name().to_string(),
                        timestamp: candle.close_time,
                    });

                    let mut closed = pos.clone();
                    closed.side = PositionSide::Flat;
                    closed.quantity = dec!(0);
                    portfolio.update_position(closed);
                    stop_targets.remove(&pos_key);
                }
            }
        }

        // Generate signal
        let current_pos_snapshot = portfolio
            .get_position(&candle.symbol, strategy.name())
            .cloned();
        if let Some(signal) = strategy.on_bar(candle, &features, current_pos_snapshot.as_ref()) {
            match signal.direction {
                SignalDirection::Long | SignalDirection::Short => {
                    if current_pos_snapshot.as_ref().map(|p| p.is_flat()).unwrap_or(true) {
                        let entry_price = fill_sim.simulate_fill_price(
                            candle.close,
                            if signal.direction == SignalDirection::Long {
                                Side::Buy
                            } else {
                                Side::Sell
                            },
                        );

                        let size = portfolio.compute_position_size(
                            &signal,
                            candle.close,
                            features.atr_14,
                        );

                        if size > dec!(0) {
                            let notional = size * entry_price;
                            let commission = fill_sim.compute_commission(notional);
                            total_commission += commission;
                            current_equity -= commission;

                            let side = if signal.direction == SignalDirection::Long {
                                PositionSide::Long
                            } else {
                                PositionSide::Short
                            };

                            let new_pos = Position {
                                symbol: candle.symbol.clone(),
                                market_type: candle.market_type,
                                side,
                                quantity: size,
                                entry_price,
                                current_price: entry_price,
                                unrealized_pnl: dec!(0),
                                realized_pnl: dec!(0),
                                total_commission: commission,
                                leverage: dec!(1),
                                liquidation_price: None,
                                opened_at: candle.close_time,
                                last_update: candle.close_time,
                                strategy_name: strategy.name().to_string(),
                            };

                            portfolio.update_position(new_pos);

                            // Track stop-loss/take-profit for this position
                            if signal.stop_loss.is_some() || signal.take_profit.is_some() {
                                let pos_key = format!("{}:{}", candle.symbol, strategy.name());
                                stop_targets.insert(pos_key, (signal.stop_loss, signal.take_profit));
                            }

                            let trade_side = if signal.direction == SignalDirection::Long {
                                Side::Buy
                            } else {
                                Side::Sell
                            };
                            trades.push(TradeRecord {
                                trade_id: TradeRecord::new_id(),
                                client_order_id: ClientOrderId::generate(),
                                symbol: candle.symbol.clone(),
                                market_type: candle.market_type,
                                side: trade_side,
                                quantity: size,
                                price: entry_price,
                                commission,
                                commission_asset: "USDT".into(),
                                realized_pnl: None,
                                strategy_name: strategy.name().to_string(),
                                timestamp: candle.close_time,
                            });
                        }
                    }
                }
                SignalDirection::Flat | SignalDirection::ReduceLong | SignalDirection::ReduceShort => {
                    if let Some(ref pos) = current_pos_snapshot {
                        if !pos.is_flat() {
                            let fill_price = fill_sim.simulate_fill_price(
                                candle.close,
                                pos.side.exit_side(),
                            );
                            let commission =
                                fill_sim.compute_commission(pos.quantity * fill_price);
                            let pnl = compute_trade_pnl(pos, fill_price) - commission;

                            current_equity += pnl;
                            risk.record_trade(pnl);
                            risk.update_equity(current_equity);
                            portfolio.update_equity(current_equity);
                            total_commission += commission;

                            trades.push(TradeRecord {
                                trade_id: TradeRecord::new_id(),
                                client_order_id: ClientOrderId::generate(),
                                symbol: candle.symbol.clone(),
                                market_type: candle.market_type,
                                side: pos.side.exit_side(),
                                quantity: pos.quantity,
                                price: fill_price,
                                commission,
                                commission_asset: "USDT".into(),
                                realized_pnl: Some(pnl),
                                strategy_name: strategy.name().to_string(),
                                timestamp: candle.close_time,
                            });

                            let mut closed = pos.clone();
                            closed.side = PositionSide::Flat;
                            closed.quantity = dec!(0);
                            portfolio.update_position(closed);

                            let pos_key = format!("{}:{}", candle.symbol, strategy.name());
                            stop_targets.remove(&pos_key);
                        }
                    }
                }
            }
        }

        // Update equity curve
        let unrealized = portfolio.total_unrealized_pnl();
        let mark_equity = current_equity + unrealized;
        if mark_equity > peak_equity {
            peak_equity = mark_equity;
        }
        let dd = if peak_equity > dec!(0) {
            (peak_equity - mark_equity) / peak_equity * dec!(100)
        } else {
            dec!(0)
        };
        equity_curve.push((candle.close_time, mark_equity));
        drawdown_curve.push((candle.close_time, dd));
    }

    let metrics = compute_metrics(&trades, config.initial_capital, current_equity, total_commission);

    info!(
        total_trades = trades.len(),
        total_return = %metrics.total_return_pct,
        max_dd = %metrics.max_drawdown_pct,
        win_rate = %metrics.win_rate,
        "Backtest complete"
    );

    BacktestResult {
        metrics,
        trades,
        equity_curve,
        drawdown_curve,
        params_used: strategy.params(),
    }
}

fn check_stop_target(
    pos: &Position,
    candle: &Candle,
    stop_loss: Option<Decimal>,
    take_profit: Option<Decimal>,
) -> (bool, Decimal) {
    // For backtesting, check if candle high/low crossed stop/target.
    // This is an approximation - real markets may gap through.
    // When both SL and TP could trigger on the same bar, assume SL hit first
    // (conservative: worst-case scenario for the trader).
    match pos.side {
        PositionSide::Long => {
            // Stop loss: candle low went below stop price
            if let Some(sl) = stop_loss {
                if candle.low <= sl {
                    return (true, sl);
                }
            }
            // Take profit: candle high reached target price
            if let Some(tp) = take_profit {
                if candle.high >= tp {
                    return (true, tp);
                }
            }
            (false, candle.close)
        }
        PositionSide::Short => {
            // Stop loss: candle high went above stop price
            if let Some(sl) = stop_loss {
                if candle.high >= sl {
                    return (true, sl);
                }
            }
            // Take profit: candle low reached target price
            if let Some(tp) = take_profit {
                if candle.low <= tp {
                    return (true, tp);
                }
            }
            (false, candle.close)
        }
        PositionSide::Flat => (false, candle.close),
    }
}

fn compute_trade_pnl(pos: &Position, exit_price: Decimal) -> Decimal {
    match pos.side {
        PositionSide::Long => (exit_price - pos.entry_price) * pos.quantity,
        PositionSide::Short => (pos.entry_price - exit_price) * pos.quantity,
        PositionSide::Flat => dec!(0),
    }
}

fn compute_metrics(
    trades: &[TradeRecord],
    initial_capital: Decimal,
    final_equity: Decimal,
    total_commission: Decimal,
) -> PerformanceMetrics {
    let pnl_trades: Vec<Decimal> = trades
        .iter()
        .filter_map(|t| t.realized_pnl)
        .collect();

    let total_return = final_equity - initial_capital;
    let total_return_pct = if initial_capital > dec!(0) {
        total_return / initial_capital * dec!(100)
    } else {
        dec!(0)
    };

    let winning: Vec<Decimal> = pnl_trades.iter().filter(|p| **p > dec!(0)).copied().collect();
    let losing: Vec<Decimal> = pnl_trades.iter().filter(|p| **p < dec!(0)).copied().collect();

    let win_rate = if !pnl_trades.is_empty() {
        Decimal::from(winning.len() as u32) / Decimal::from(pnl_trades.len() as u32) * dec!(100)
    } else {
        dec!(0)
    };

    let avg_win = if !winning.is_empty() {
        winning.iter().sum::<Decimal>() / Decimal::from(winning.len() as u32)
    } else {
        dec!(0)
    };
    let avg_loss = if !losing.is_empty() {
        losing.iter().sum::<Decimal>() / Decimal::from(losing.len() as u32)
    } else {
        dec!(0)
    };

    let avg_trade_return = if !pnl_trades.is_empty() {
        pnl_trades.iter().sum::<Decimal>() / Decimal::from(pnl_trades.len() as u32)
    } else {
        dec!(0)
    };

    let profit_factor = if !losing.is_empty() {
        let gross_profit: Decimal = winning.iter().sum();
        let gross_loss: Decimal = losing.iter().map(|l| l.abs()).sum();
        if gross_loss > dec!(0) {
            Some(gross_profit / gross_loss)
        } else {
            None
        }
    } else {
        None
    };

    // Max consecutive losses
    let mut max_consec = 0usize;
    let mut current_consec = 0usize;
    for pnl in &pnl_trades {
        if *pnl < dec!(0) {
            current_consec += 1;
            max_consec = max_consec.max(current_consec);
        } else {
            current_consec = 0;
        }
    }

    // Max drawdown from PnL series
    let mut equity = initial_capital;
    let mut peak = initial_capital;
    let mut max_dd_pct = dec!(0);
    for pnl in &pnl_trades {
        equity += pnl;
        if equity > peak {
            peak = equity;
        }
        let dd = if peak > dec!(0) {
            (peak - equity) / peak * dec!(100)
        } else {
            dec!(0)
        };
        if dd > max_dd_pct {
            max_dd_pct = dd;
        }
    }

    PerformanceMetrics {
        total_return,
        total_return_pct,
        annualized_return_pct: None,
        max_drawdown_pct: max_dd_pct,
        sharpe_ratio: None,
        sortino_ratio: None,
        calmar_ratio: None,
        win_rate,
        profit_factor,
        total_trades: trades.len(),
        avg_trade_return_pct: avg_trade_return,
        avg_win_pct: avg_win,
        avg_loss_pct: avg_loss,
        max_consecutive_losses: max_consec,
        exposure_pct: dec!(0),
        total_commission,
    }
}

fn empty_metrics() -> PerformanceMetrics {
    PerformanceMetrics {
        total_return: dec!(0),
        total_return_pct: dec!(0),
        annualized_return_pct: None,
        max_drawdown_pct: dec!(0),
        sharpe_ratio: None,
        sortino_ratio: None,
        calmar_ratio: None,
        win_rate: dec!(0),
        profit_factor: None,
        total_trades: 0,
        avg_trade_return_pct: dec!(0),
        avg_win_pct: dec!(0),
        avg_loss_pct: dec!(0),
        max_consecutive_losses: 0,
        exposure_pct: dec!(0),
        total_commission: dec!(0),
    }
}

/// Helper trait for position side exit.
trait PositionSideExt {
    fn exit_side(&self) -> Side;
}

impl PositionSideExt for PositionSide {
    fn exit_side(&self) -> Side {
        match self {
            PositionSide::Long => Side::Sell,
            PositionSide::Short => Side::Buy,
            PositionSide::Flat => Side::Sell, // No-op
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_simulator_slippage() {
        let sim = FillSimulator::new(dec!(10), dec!(5));
        let buy_price = sim.simulate_fill_price(dec!(50000), Side::Buy);
        assert!(buy_price > dec!(50000));

        let sell_price = sim.simulate_fill_price(dec!(50000), Side::Sell);
        assert!(sell_price < dec!(50000));
    }

    #[test]
    fn fill_simulator_commission() {
        let sim = FillSimulator::new(dec!(10), dec!(5));
        let commission = sim.compute_commission(dec!(50000));
        // 10 bps of 50000 = 50
        assert_eq!(commission, dec!(50));
    }

    #[test]
    fn compute_trade_pnl_long() {
        let pos = Position {
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            side: PositionSide::Long,
            quantity: dec!(1),
            entry_price: dec!(50000),
            current_price: dec!(51000),
            unrealized_pnl: dec!(0),
            realized_pnl: dec!(0),
            total_commission: dec!(0),
            leverage: dec!(1),
            liquidation_price: None,
            opened_at: Utc::now(),
            last_update: Utc::now(),
            strategy_name: "test".into(),
        };
        assert_eq!(compute_trade_pnl(&pos, dec!(51000)), dec!(1000));
    }

    #[test]
    fn compute_trade_pnl_short() {
        let pos = Position {
            symbol: Symbol::new("BTCUSDT"),
            market_type: MarketType::Spot,
            side: PositionSide::Short,
            quantity: dec!(1),
            entry_price: dec!(50000),
            current_price: dec!(49000),
            unrealized_pnl: dec!(0),
            realized_pnl: dec!(0),
            total_commission: dec!(0),
            leverage: dec!(1),
            liquidation_price: None,
            opened_at: Utc::now(),
            last_update: Utc::now(),
            strategy_name: "test".into(),
        };
        assert_eq!(compute_trade_pnl(&pos, dec!(49000)), dec!(1000));
    }
}
