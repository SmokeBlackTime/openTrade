//! Binance USDT-M Futures specific operations.
//!
//! Handles margin mode, leverage, funding rates, and liquidation price monitoring.

use chrono::Utc;
use ot_common::{ExchangeError, OtError};
use ot_types::market::{FundingRate, Symbol};
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, info, warn};

use crate::auth::sign_query;
use crate::types::*;

/// Binance USDT-M Futures client.
pub struct BinanceFuturesClient {
    http: reqwest::Client,
    api_key: String,
    api_secret: String,
    base_url: String,
    recv_window: u64,
}

/// Futures position information.
#[derive(Debug, Clone)]
pub struct FuturesPosition {
    pub symbol: String,
    pub position_amt: Decimal,
    pub entry_price: Decimal,
    pub mark_price: Decimal,
    pub unrealized_profit: Decimal,
    pub liquidation_price: Decimal,
    pub leverage: u32,
    pub margin_type: String,
    pub isolated_margin: Decimal,
    pub notional: Decimal,
}

/// Funding rate response.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceFundingRate {
    pub symbol: String,
    pub funding_rate: String,
    pub funding_time: i64,
}

/// Futures account info.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesAccountInfo {
    pub total_wallet_balance: String,
    pub total_unrealized_profit: String,
    pub total_margin_balance: String,
    pub available_balance: String,
    pub positions: Vec<FuturesPositionInfo>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesPositionInfo {
    pub symbol: String,
    pub position_amt: String,
    pub entry_price: String,
    pub mark_price: String,
    pub unrealized_profit: String,
    pub liquidation_price: String,
    pub leverage: String,
    pub margin_type: String,
    pub isolated_margin: String,
    pub notional: String,
}

/// Leverage response.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeverageResponse {
    pub leverage: u32,
    pub max_notional_value: String,
    pub symbol: String,
}

/// Margin type response.
#[allow(dead_code)]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarginTypeResponse {
    pub code: i32,
    pub msg: String,
}

impl BinanceFuturesClient {
    pub fn new(api_key: String, api_secret: String, use_testnet: bool) -> Self {
        let base_url = if use_testnet {
            "https://testnet.binancefuture.com".to_string()
        } else {
            "https://fapi.binance.com".to_string()
        };

        Self {
            http: reqwest::Client::builder()
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

    /// Set leverage for a symbol.
    pub async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<u32, OtError> {
        let mut query = format!(
            "symbol={}&leverage={}&recvWindow={}&timestamp={}",
            symbol, leverage, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/fapi/v1/leverage?{}", self.base_url, query);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let result: LeverageResponse = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        info!(symbol = symbol, leverage = result.leverage, "Leverage set");
        Ok(result.leverage)
    }

    /// Set margin type (ISOLATED or CROSSED).
    pub async fn set_margin_type(&self, symbol: &str, margin_type: &str) -> Result<(), OtError> {
        let mut query = format!(
            "symbol={}&marginType={}&recvWindow={}&timestamp={}",
            symbol, margin_type, self.recv_window, Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/fapi/v1/marginType?{}", self.base_url, query);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            // -4046 means "No need to change margin type" which is fine
            if body.contains("-4046") {
                debug!(symbol = symbol, margin_type = margin_type, "Margin type already set");
                return Ok(());
            }
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        info!(symbol = symbol, margin_type = margin_type, "Margin type set");
        Ok(())
    }

    /// Get current funding rate for a symbol.
    pub async fn get_funding_rate(&self, symbol: &str) -> Result<FundingRate, OtError> {
        let url = format!(
            "{}/fapi/v1/premiumIndex?symbol={}",
            self.base_url, symbol
        );

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        let rate = data["lastFundingRate"]
            .as_str()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO);

        let next_time = data["nextFundingTime"]
            .as_i64()
            .unwrap_or(0);

        Ok(FundingRate {
            symbol: Symbol::new(symbol),
            timestamp: Utc::now(),
            rate,
            next_funding_time: ot_common::time_utils::ms_to_datetime(next_time),
        })
    }

    /// Get all futures positions with non-zero balances.
    pub async fn get_positions(&self) -> Result<Vec<FuturesPosition>, OtError> {
        let mut query = format!(
            "recvWindow={}&timestamp={}",
            self.recv_window,
            Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/fapi/v2/account?{}", self.base_url, query);

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let account: FuturesAccountInfo = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        let parse = |s: &str| Decimal::from_str(s).unwrap_or(Decimal::ZERO);

        let positions: Vec<FuturesPosition> = account
            .positions
            .iter()
            .filter(|p| {
                let amt = parse(&p.position_amt);
                amt != Decimal::ZERO
            })
            .map(|p| FuturesPosition {
                symbol: p.symbol.clone(),
                position_amt: parse(&p.position_amt),
                entry_price: parse(&p.entry_price),
                mark_price: parse(&p.mark_price),
                unrealized_profit: parse(&p.unrealized_profit),
                liquidation_price: parse(&p.liquidation_price),
                leverage: p.leverage.parse().unwrap_or(1),
                margin_type: p.margin_type.clone(),
                isolated_margin: parse(&p.isolated_margin),
                notional: parse(&p.notional),
            })
            .collect();

        Ok(positions)
    }

    /// Get wallet balance.
    pub async fn get_balance(&self) -> Result<Decimal, OtError> {
        let mut query = format!(
            "recvWindow={}&timestamp={}",
            self.recv_window,
            Self::timestamp_ms()
        );
        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/fapi/v2/balance?{}", self.base_url, query);

        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OtError::Exchange(ExchangeError::Http(body)));
        }

        let balances: Vec<serde_json::Value> = resp
            .json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))?;

        for b in &balances {
            if b["asset"].as_str() == Some("USDT") {
                let balance = b["balance"]
                    .as_str()
                    .and_then(|s| Decimal::from_str(s).ok())
                    .unwrap_or(Decimal::ZERO);
                return Ok(balance);
            }
        }

        Ok(Decimal::ZERO)
    }

    /// Place a futures order.
    pub async fn place_order(
        &self,
        symbol: &str,
        side: &str,
        order_type: &str,
        quantity: Decimal,
        price: Option<Decimal>,
        stop_price: Option<Decimal>,
        reduce_only: bool,
    ) -> Result<BinanceOrderResponse, OtError> {
        let mut query = format!(
            "symbol={}&side={}&type={}&quantity={}&recvWindow={}&timestamp={}",
            symbol, side, order_type, quantity, self.recv_window, Self::timestamp_ms()
        );

        if let Some(p) = price {
            query.push_str(&format!("&price={}&timeInForce=GTC", p));
        }

        if let Some(sp) = stop_price {
            query.push_str(&format!("&stopPrice={}", sp));
        }

        if reduce_only {
            query.push_str("&reduceOnly=true");
        }

        let signature = sign_query(&query, &self.api_secret);
        query.push_str(&format!("&signature={}", signature));

        let url = format!("{}/fapi/v1/order?{}", self.base_url, query);

        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .send()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Http(e.to_string())))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OtError::Exchange(ExchangeError::OrderRejected(body)));
        }

        resp.json()
            .await
            .map_err(|e| OtError::Exchange(ExchangeError::Parse(e.to_string())))
    }

    /// Monitor liquidation price proximity.
    /// Returns (symbol, distance_pct) for positions near liquidation.
    pub async fn check_liquidation_risk(
        &self,
        warn_threshold_pct: Decimal,
    ) -> Result<Vec<(String, Decimal)>, OtError> {
        let positions = self.get_positions().await?;
        let mut warnings = Vec::new();

        for pos in &positions {
            if pos.liquidation_price <= Decimal::ZERO || pos.mark_price <= Decimal::ZERO {
                continue;
            }

            let distance_pct = if pos.position_amt > Decimal::ZERO {
                // Long: distance = (mark - liq) / mark * 100
                (pos.mark_price - pos.liquidation_price) / pos.mark_price
                    * Decimal::from(100)
            } else {
                // Short: distance = (liq - mark) / mark * 100
                (pos.liquidation_price - pos.mark_price) / pos.mark_price
                    * Decimal::from(100)
            };

            if distance_pct < warn_threshold_pct {
                warn!(
                    symbol = %pos.symbol,
                    mark = %pos.mark_price,
                    liq = %pos.liquidation_price,
                    distance_pct = %distance_pct,
                    "Position near liquidation!"
                );
                warnings.push((pos.symbol.clone(), distance_pct));
            }
        }

        Ok(warnings)
    }
}

/// Futures WebSocket stream for klines.
pub const BINANCE_FUTURES_WS: &str = "wss://fstream.binance.com/ws";
pub const BINANCE_FUTURES_WS_TESTNET: &str = "wss://stream.binancefuture.com/ws";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn futures_client_creation() {
        let client = BinanceFuturesClient::new(
            "test_key".into(),
            "test_secret".into(),
            true,
        );
        assert_eq!(client.base_url, "https://testnet.binancefuture.com");
    }

    #[test]
    fn futures_client_production_url() {
        let client = BinanceFuturesClient::new(
            "test_key".into(),
            "test_secret".into(),
            false,
        );
        assert_eq!(client.base_url, "https://fapi.binance.com");
    }
}
