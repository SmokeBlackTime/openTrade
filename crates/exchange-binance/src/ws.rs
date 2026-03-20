use futures::StreamExt;
use ot_common::OtError;
use ot_types::market::*;
use rust_decimal::Decimal;
use std::str::FromStr;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tracing::{error, info, warn};

use crate::types::WsKlineEvent;

const BINANCE_WS_SPOT: &str = "wss://stream.binance.com:9443/ws";
const BINANCE_WS_TESTNET: &str = "wss://testnet.binance.vision/ws";

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
