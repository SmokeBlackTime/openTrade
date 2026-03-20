use chrono::Utc;
use ot_common::{ExchangeError, OtError};
use ot_types::market::*;
use ot_types::orders::*;
use reqwest::Client;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, error, warn};

use crate::auth::sign_query;
use crate::types::*;

/// Binance REST client for spot and futures.
pub struct BinanceClient {
    http: Client,
    api_key: String,
    api_secret: String,
    base_url: String,
    recv_window: u64,
}

impl BinanceClient {
    pub fn new(api_key: String, api_secret: String, use_testnet: bool) -> Self {
        Self::with_base_url(api_key, api_secret, use_testnet, None)
    }

    pub fn with_base_url(
        api_key: String,
        api_secret: String,
        use_testnet: bool,
        custom_base_url: Option<String>,
    ) -> Self {
        let base_url = custom_base_url.unwrap_or_else(|| {
            if use_testnet {
                "https://testnet.binance.vision".to_string()
            } else {
                "https://api.binance.com".to_string()
            }
        });

        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            api_key,
            api_secret,
            base_url,
            recv_window: 5000,
        }
    }

    pub fn futures(api_key: String, api_secret: String, use_testnet: bool) -> Self {
        let base_url = if use_testnet {
            "https://testnet.binancefuture.com".to_string()
        } else {
            "https://fapi.binance.com".to_string()
        };

        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build HTTP client"),
            api_key,
            api_secret,
            base_url,
            recv_window: 5000,
        }
    }

    fn timestamp_ms() -> i64 {
        Utc::now().timestamp_millis()
    }

    /// Fetch historical klines/candles.
    /// Always uses production API since klines are public data and
    /// the testnet has very limited historical data.
    pub async fn get_klines(
        &self,
        symbol: &str,
        interval: &str,
        start_time: Option<i64>,
        end_time: Option<i64>,
        limit: Option<u32>,
    ) -> Result<Vec<Candle>, OtError> {
        let mut params = format!("symbol={}&interval={}", symbol, interval);
        if let Some(st) = start_time {
            params.push_str(&format!("&startTime={}", st));
        }
        if let Some(et) = end_time {
            params.push_str(&format!("&endTime={}", et));
        }
        let lim = limit.unwrap_or(1000).min(1000);
        params.push_str(&format!("&limit={}", lim));

        // Always use production API for klines - it's a public endpoint
        // (no auth required) and the testnet has no historical data.
        let url = format!("https://api.binance.com/api/v3/klines?{}", params);
        debug!(url = %url, "Fetching klines");

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            if status.as_u16() == 429 {
                return Err(OtError::Exchange(ExchangeError::RateLimited {
                    retry_after_ms: 60000,
                }));
            }
            return Err(OtError::Exchange(ExchangeError::Http(format!(
                "{}: {}",
                status, body
            ))));
        }

        let raw: Vec<Vec<serde_json::Value>> = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

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

        let mut candles = Vec::with_capacity(raw.len());
        for kline in &raw {
            if kline.len() < 12 {
                continue;
            }
            let open_time =
                ot_common::time_utils::ms_to_datetime(kline[0].as_i64().unwrap_or(0));
            let close_time =
                ot_common::time_utils::ms_to_datetime(kline[6].as_i64().unwrap_or(0));
            let parse_dec = |v: &serde_json::Value| -> Decimal {
                v.as_str()
                    .and_then(|s| Decimal::from_str(s).ok())
                    .unwrap_or(Decimal::ZERO)
            };

            candles.push(Candle {
                symbol: sym.clone(),
                market_type: MarketType::Spot,
                timeframe: tf,
                open_time,
                close_time,
                open: parse_dec(&kline[1]),
                high: parse_dec(&kline[2]),
                low: parse_dec(&kline[3]),
                close: parse_dec(&kline[4]),
                volume: parse_dec(&kline[5]),
                quote_volume: parse_dec(&kline[7]),
                trades: kline[8].as_u64().unwrap_or(0),
            });
        }

        Ok(candles)
    }

    /// Get current price.
    pub async fn get_price(&self, symbol: &str) -> Result<Decimal, OtError> {
        let url = format!("{}/api/v3/ticker/price?symbol={}", self.base_url, symbol);
        let resp: BinanceTickerPrice = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        Decimal::from_str(&resp.price)
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Get best bid/ask.
    pub async fn get_book_ticker(&self, symbol: &str) -> Result<TopOfBook, OtError> {
        let url = format!(
            "{}/api/v3/ticker/bookTicker?symbol={}",
            self.base_url, symbol
        );
        let resp: BinanceBookTicker = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        let parse_dec = |s: &str| -> Decimal {
            Decimal::from_str(s).unwrap_or(Decimal::ZERO)
        };

        Ok(TopOfBook {
            symbol: Symbol::new(symbol),
            market_type: MarketType::Spot,
            timestamp: Utc::now(),
            best_bid: parse_dec(&resp.bid_price),
            best_bid_qty: parse_dec(&resp.bid_qty),
            best_ask: parse_dec(&resp.ask_price),
            best_ask_qty: parse_dec(&resp.ask_qty),
        })
    }

    /// Submit a new order (signed).
    pub async fn place_order(&self, req: &OrderRequest) -> Result<BinanceOrderResponse, OtError> {
        let side_str = match req.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        };
        let type_str = match req.order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "STOP_LOSS",
            OrderType::StopLossLimit => "STOP_LOSS_LIMIT",
            OrderType::TakeProfit => "TAKE_PROFIT",
            OrderType::TakeProfitLimit => "TAKE_PROFIT_LIMIT",
            OrderType::PostOnly => "LIMIT",
            OrderType::ReduceOnly => "MARKET",
        };

        let mut query = format!(
            "symbol={}&side={}&type={}&quantity={}&newClientOrderId={}&recvWindow={}&timestamp={}",
            req.symbol,
            side_str,
            type_str,
            req.quantity,
            req.client_order_id,
            self.recv_window,
            Self::timestamp_ms()
        );

        if let Some(price) = req.price {
            query.push_str(&format!("&price={}", price));
            if req.time_in_force.is_some() || req.order_type == OrderType::Limit {
                let tif = match req.time_in_force {
                    Some(TimeInForce::Gtc) | None => "GTC",
                    Some(TimeInForce::Ioc) => "IOC",
                    Some(TimeInForce::Fok) => "FOK",
                };
                query.push_str(&format!("&timeInForce={}", tif));
            }
        }

        if let Some(sp) = req.stop_price {
            query.push_str(&format!("&stopPrice={}", sp));
        }

        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/order?{}", self.base_url, query);
        debug!("Placing order: {} {} {} {}", req.symbol, side_str, type_str, req.quantity);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            error!(body = %body, "Order rejected");
            return Err(OtError::Exchange(ExchangeError::OrderRejected(body)));
        }

        resp.json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Cancel an order.
    pub async fn cancel_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<BinanceOrderResponse, OtError> {
        let mut query = format!(
            "symbol={}&origClientOrderId={}&recvWindow={}&timestamp={}",
            symbol, client_order_id, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/order?{}", self.base_url, query);
        debug!("Cancelling order: {} {}", symbol, client_order_id);

        let resp = self
            .http
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        resp.json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Query a specific order by client order ID.
    pub async fn query_order(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<BinanceOrderResponse, OtError> {
        let mut query = format!(
            "symbol={}&origClientOrderId={}&recvWindow={}&timestamp={}",
            symbol, client_order_id, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/order?{}", self.base_url, query);
        debug!("Querying order: {} {}", symbol, client_order_id);

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        resp.json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Get account balance for a specific asset.
    pub async fn get_account_balance(&self, asset: &str) -> Result<Decimal, OtError> {
        let mut query = format!(
            "recvWindow={}&timestamp={}",
            self.recv_window,
            Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/account?{}", self.base_url, query);
        debug!("Fetching account info");

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let account: BinanceAccountInfo = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        for balance in &account.balances {
            if balance.asset == asset {
                let free = Decimal::from_str(&balance.free).unwrap_or(Decimal::ZERO);
                let locked = Decimal::from_str(&balance.locked).unwrap_or(Decimal::ZERO);
                return Ok(free + locked);
            }
        }

        Ok(Decimal::ZERO)
    }

    /// Get all open orders for a symbol.
    pub async fn get_open_orders(&self, symbol: &str) -> Result<Vec<BinanceOrderResponse>, OtError> {
        let mut query = format!(
            "symbol={}&recvWindow={}&timestamp={}",
            symbol, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/openOrders?{}", self.base_url, query);
        debug!("Fetching open orders for {}", symbol);

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        resp.json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Start a user data stream (returns listen key).
    pub async fn start_user_data_stream(&self) -> Result<String, OtError> {
        let url = format!("{}/api/v3/userDataStream", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let data: BinanceListenKey = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        Ok(data.listen_key)
    }

    /// Keep alive user data stream.
    pub async fn keepalive_user_data_stream(&self, listen_key: &str) -> Result<(), OtError> {
        let url = format!(
            "{}/api/v3/userDataStream?listenKey={}",
            self.base_url, listen_key
        );

        let resp = self
            .http
            .put(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        Ok(())
    }

    /// Cancel all open orders for a symbol.
    pub async fn cancel_all_orders(&self, symbol: &str) -> Result<(), OtError> {
        let mut query = format!(
            "symbol={}&recvWindow={}&timestamp={}",
            symbol, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/api/v3/openOrders?{}", self.base_url, query);
        warn!("Cancelling all orders for {}", symbol);

        let resp = self
            .http
            .delete(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "unknown".to_string());
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        Ok(())
    }
}
