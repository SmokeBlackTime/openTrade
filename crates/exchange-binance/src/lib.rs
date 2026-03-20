//! Binance exchange adapter for OpenTrade.
//!
//! Implements the exchange adapter trait for both Binance spot and USDT-M futures.
//! Supports testnet and production endpoints.

pub mod adapter;
pub mod auth;
pub mod client;
pub mod futures;
pub mod rest;
pub mod types;
pub mod ws;

pub use adapter::BinanceExchangeAdapter;
pub use client::BinanceClient;
pub use futures::BinanceFuturesClient;
