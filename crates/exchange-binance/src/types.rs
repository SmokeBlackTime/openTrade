use serde::Deserialize;

/// Binance kline/candlestick response element.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceKline {
    pub open_time: i64,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub close_time: i64,
    pub quote_asset_volume: String,
    pub number_of_trades: u64,
    pub taker_buy_base: String,
    pub taker_buy_quote: String,
    pub ignore: String,
}

/// Binance order response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceOrderResponse {
    pub symbol: String,
    pub order_id: u64,
    pub client_order_id: String,
    pub transact_time: Option<i64>,
    pub price: String,
    pub orig_qty: String,
    pub executed_qty: String,
    #[serde(alias = "cumQuote", default)]
    pub cummulative_quote_qty: String,
    pub status: String,
    pub r#type: String,
    pub side: String,
    #[serde(default)]
    pub fills: Vec<BinanceFill>,
}

/// A fill from Binance order response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceFill {
    pub price: String,
    pub qty: String,
    pub commission: String,
    pub commission_asset: String,
}

/// Binance account balance.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceBalance {
    pub asset: String,
    pub free: String,
    pub locked: String,
}

/// Binance ticker price.
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceTickerPrice {
    pub symbol: String,
    pub price: String,
}

/// Binance book ticker (best bid/ask).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceBookTicker {
    pub symbol: String,
    pub bid_price: String,
    pub bid_qty: String,
    pub ask_price: String,
    pub ask_qty: String,
}

/// WebSocket kline event.
#[derive(Debug, Clone, Deserialize)]
pub struct WsKlineEvent {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "k")]
    pub kline: WsKlineData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WsKlineData {
    #[serde(rename = "t")]
    pub start_time: i64,
    #[serde(rename = "T")]
    pub close_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "i")]
    pub interval: String,
    #[serde(rename = "o")]
    pub open: String,
    #[serde(rename = "c")]
    pub close: String,
    #[serde(rename = "h")]
    pub high: String,
    #[serde(rename = "l")]
    pub low: String,
    #[serde(rename = "v")]
    pub volume: String,
    #[serde(rename = "n")]
    pub number_of_trades: u64,
    #[serde(rename = "x")]
    pub is_closed: bool,
    #[serde(rename = "q")]
    pub quote_volume: String,
}

/// Binance account info response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceAccountInfo {
    pub balances: Vec<BinanceBalance>,
}

/// Binance futures account info response (/fapi/v2/account).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceFuturesAccountInfo {
    #[serde(default)]
    pub assets: Vec<BinanceFuturesAsset>,
}

/// Binance futures asset balance.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceFuturesAsset {
    pub asset: String,
    pub wallet_balance: String,
    #[serde(default)]
    pub available_balance: String,
}

/// Binance listen key response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceListenKey {
    pub listen_key: String,
}

/// User data stream: execution report (order update).
#[derive(Debug, Clone, Deserialize)]
pub struct WsExecutionReport {
    #[serde(rename = "e")]
    pub event_type: String,
    #[serde(rename = "E")]
    pub event_time: i64,
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: String,
    #[serde(rename = "o")]
    pub order_type: String,
    #[serde(rename = "q")]
    pub orig_quantity: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "X")]
    pub order_status: String,
    #[serde(rename = "i")]
    pub order_id: u64,
    #[serde(rename = "l")]
    pub last_filled_qty: String,
    #[serde(rename = "L")]
    pub last_filled_price: String,
    #[serde(rename = "z")]
    pub cumulative_filled_qty: String,
    #[serde(rename = "Z", default)]
    pub cumulative_quote_qty: String,
    #[serde(rename = "n")]
    pub commission: String,
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    #[serde(rename = "T")]
    pub transaction_time: i64,
}

/// User data stream events (can be execution report or balance update).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "e")]
pub enum WsUserDataEvent {
    #[serde(rename = "executionReport")]
    ExecutionReport(WsExecutionReport),
    /// Futures order update event (ORDER_TRADE_UPDATE).
    #[serde(rename = "ORDER_TRADE_UPDATE")]
    OrderTradeUpdate(WsFuturesOrderUpdate),
    #[serde(other)]
    Other,
}

/// Futures ORDER_TRADE_UPDATE wrapper.
#[derive(Debug, Clone, Deserialize)]
pub struct WsFuturesOrderUpdate {
    /// Event time.
    #[serde(rename = "E")]
    pub event_time: i64,
    /// The order data is nested under "o".
    #[serde(rename = "o")]
    pub order: WsFuturesOrder,
}

/// Futures order detail inside ORDER_TRADE_UPDATE.
#[derive(Debug, Clone, Deserialize)]
pub struct WsFuturesOrder {
    #[serde(rename = "s")]
    pub symbol: String,
    #[serde(rename = "c")]
    pub client_order_id: String,
    #[serde(rename = "S")]
    pub side: String,
    #[serde(rename = "o")]
    pub order_type: String,
    #[serde(rename = "q")]
    pub orig_quantity: String,
    #[serde(rename = "p")]
    pub price: String,
    #[serde(rename = "X")]
    pub order_status: String,
    #[serde(rename = "i")]
    pub order_id: u64,
    #[serde(rename = "l")]
    pub last_filled_qty: String,
    #[serde(rename = "L")]
    pub last_filled_price: String,
    #[serde(rename = "z")]
    pub cumulative_filled_qty: String,
    #[serde(rename = "Z", default)]
    pub cumulative_quote_qty: String,
    #[serde(rename = "n")]
    pub commission: String,
    #[serde(rename = "N")]
    pub commission_asset: Option<String>,
    #[serde(rename = "T")]
    pub transaction_time: i64,
    #[serde(rename = "rp", default)]
    pub realized_profit: String,
}

/// Exchange info for symbol filters.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceExchangeInfo {
    pub symbols: Vec<BinanceSymbolInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceSymbolInfo {
    pub symbol: String,
    pub status: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub filters: Vec<serde_json::Value>,
}
