use futures::StreamExt;
use ot_common::OtError;
use ot_types::market::*;
use ot_types::orders::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};

use crate::types::{WsKlineEvent, WsUserDataEvent, WsExecutionReport};

const BINANCE_WS_SPOT: &str = "wss://stream.binance.com:9443/ws";
const BINANCE_WS_TESTNET: &str = "wss://stream.testnet.binance.vision/ws";

/// Subscribe to kline/candle stream for a symbol.
pub async fn subscribe_klines(
    symbol: &str,
    interval: &str,
    use_testnet: bool,
    buffer_size: usize,
) -> Result<mpsc::Receiver<Candle>, OtError> {
    let stream_name = format!("{}@kline_{}", symbol.to_lowercase(), interval);
    let base = if use_testnet {
        BINANCE_WS_TESTNET
    } else {
        BINANCE_WS_SPOT
    };
    let url = format!("{}/{}", base, stream_name);

    let (tx, rx) = mpsc::channel(buffer_size);
    let sym = Symbol::new(symbol);
    let tf = match interval {
        "1m" => Timeframe::M1,
        "5m" => Timeframe::M5,
        "15m" => Timeframe::M15,
        "1h" => Timeframe::H1,
        "4h" => Timeframe::H4,
        "1d" => Timeframe::D1,
        _ => Timeframe::H1,
    };

    tokio::spawn(async move {
        let mut reconnect_delay = std::time::Duration::from_secs(1);
        let max_reconnect_delay = std::time::Duration::from_secs(60);

        loop {
            info!(url = %url, "Connecting to Binance WebSocket");

            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    reconnect_delay = std::time::Duration::from_secs(1);
                    let (_, mut read) = ws_stream.split();

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                                match serde_json::from_str::<WsKlineEvent>(&text) {
                                    Ok(event) if event.kline.is_closed => {
                                        let parse =
                                            |s: &str| Decimal::from_str(s).unwrap_or(Decimal::ZERO);
                                        let candle = Candle {
                                            symbol: sym.clone(),
                                            market_type: MarketType::Spot,
                                            timeframe: tf,
                                            open_time: ot_common::time_utils::ms_to_datetime(
                                                event.kline.start_time,
                                            ),
                                            close_time: ot_common::time_utils::ms_to_datetime(
                                                event.kline.close_time,
                                            ),
                                            open: parse(&event.kline.open),
                                            high: parse(&event.kline.high),
                                            low: parse(&event.kline.low),
                                            close: parse(&event.kline.close),
                                            volume: parse(&event.kline.volume),
                                            quote_volume: parse(&event.kline.quote_volume),
                                            trades: event.kline.number_of_trades,
                                        };
                                        if tx.send(candle).await.is_err() {
                                            info!("Candle receiver dropped, stopping WS");
                                            return;
                                        }
                                    }
                                    Ok(_) => {} // Not a closed candle, ignore
                                    Err(e) => {
                                        warn!(error = %e, "Failed to parse WS kline event");
                                    }
                                }
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Ping(_data)) => {
                                // Pong is handled automatically by tungstenite
                            }
                            Err(e) => {
                                error!(error = %e, "WebSocket error");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to connect to WebSocket");
                }
            }

            warn!(delay = ?reconnect_delay, "WebSocket disconnected, reconnecting");
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = (reconnect_delay * 2).min(max_reconnect_delay);
        }
    });

    Ok(rx)
}

/// Parsed user data event for order updates.
#[derive(Debug, Clone)]
pub struct OrderUpdateEvent {
    pub symbol: String,
    pub client_order_id: String,
    pub side: Side,
    pub status: OrderStatus,
    pub exchange_order_id: u64,
    pub filled_quantity: Decimal,
    pub cumulative_quote_qty: Decimal,
    pub last_fill_price: Decimal,
    pub last_fill_qty: Decimal,
    pub commission: Decimal,
    pub commission_asset: String,
    pub transaction_time: i64,
}

fn parse_execution_report(report: &WsExecutionReport) -> OrderUpdateEvent {
    let parse = |s: &str| Decimal::from_str(s).unwrap_or(Decimal::ZERO);
    let side = match report.side.as_str() {
        "BUY" => Side::Buy,
        _ => Side::Sell,
    };
    let status = match report.order_status.as_str() {
        "NEW" => OrderStatus::Submitted,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" | "CANCELLED" => OrderStatus::Cancelled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" => OrderStatus::Expired,
        _ => OrderStatus::Submitted,
    };

    OrderUpdateEvent {
        symbol: report.symbol.clone(),
        client_order_id: report.client_order_id.clone(),
        side,
        status,
        exchange_order_id: report.order_id,
        filled_quantity: parse(&report.cumulative_filled_qty),
        cumulative_quote_qty: parse(&report.cumulative_quote_qty),
        last_fill_price: parse(&report.last_filled_price),
        last_fill_qty: parse(&report.last_filled_qty),
        commission: parse(&report.commission),
        commission_asset: report.commission_asset.clone().unwrap_or_default(),
        transaction_time: report.transaction_time,
    }
}

/// Subscribe to user data stream for order execution updates.
pub async fn subscribe_user_data(
    listen_key: &str,
    use_testnet: bool,
    buffer_size: usize,
) -> Result<mpsc::Receiver<OrderUpdateEvent>, OtError> {
    let base = if use_testnet {
        BINANCE_WS_TESTNET
    } else {
        BINANCE_WS_SPOT
    };
    let url = format!("{}/{}", base, listen_key);

    let (tx, rx) = mpsc::channel(buffer_size);

    tokio::spawn(async move {
        let mut reconnect_delay = std::time::Duration::from_secs(1);
        let max_reconnect_delay = std::time::Duration::from_secs(60);

        loop {
            info!(url = %url, "Connecting to user data stream");

            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    reconnect_delay = std::time::Duration::from_secs(1);
                    let (_, mut read) = ws_stream.split();

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                                // Try parsing as execution report
                                match serde_json::from_str::<WsUserDataEvent>(&text) {
                                    Ok(WsUserDataEvent::ExecutionReport(report)) => {
                                        let update = parse_execution_report(&report);
                                        if tx.send(update).await.is_err() {
                                            info!("User data receiver dropped, stopping");
                                            return;
                                        }
                                    }
                                    Ok(WsUserDataEvent::Other) => {
                                        // Balance update or other event, ignore
                                    }
                                    Err(e) => {
                                        warn!(error = %e, "Failed to parse user data event");
                                    }
                                }
                            }
                            Ok(tokio_tungstenite::tungstenite::Message::Ping(_)) => {}
                            Err(e) => {
                                error!(error = %e, "User data WebSocket error");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Failed to connect to user data stream");
                }
            }

            warn!(delay = ?reconnect_delay, "User data stream disconnected, reconnecting");
            tokio::time::sleep(reconnect_delay).await;
            reconnect_delay = (reconnect_delay * 2).min(max_reconnect_delay);
        }
    });

    Ok(rx)
}
