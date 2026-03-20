//! Binance exchange adapter for OpenTrade.
//!
//! Implements the exchange adapter trait for both Binance spot and USDT-M futures.
//! Supports testnet and production endpoints.

pub mod auth;
pub mod client;
pub mod rest;
pub mod types;
pub mod ws;

pub use client::BinanceClient;
