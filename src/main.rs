#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

pub mod cache;
pub mod config;
pub mod commodity;
pub mod crypto;
//pub mod etf;
pub mod financial;
pub mod forex;
pub mod index;
pub mod market;
//pub mod mutualfund;
pub mod request;
//pub mod search;
pub mod stock;
pub mod technical_indicators;
pub mod economic_data;
pub mod websocket;
pub mod utils;
pub mod request_parser;
pub mod options;
//pub mod test;
pub mod logging;

use serde_json::json;
use tokio;

use crate::stock::StockPolling;
use crate::commodity::CommodityPolling;
use crate::crypto::CryptoPolling;
use crate::forex::ForexPolling;
use crate::market::MarketPolling;
use crate::technical_indicators::TechnicalIndicatorPolling;
use crate::economic_data::EconomicDataPolling;
use crate::index::IndexPolling;
use crate::cache::SharedLockedCache;
use crate::request::HTTPClient;
use config::{BatchConfig, RetryConfig};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    //let _ = websocket::run().await;

    let cache = Arc::new(Mutex::new(SharedLockedCache::new(1000)));
    let batch_config = Arc::new(BatchConfig::default());
    let retry_config = Arc::new(RetryConfig::default());
    let http_client = Arc::new(HTTPClient::new().expect("Failed to create HTTP client"));
    let polling = StockPolling::new( http_client, cache, batch_config, retry_config);

    let r = polling.poll(&json!({
        "tickers": ["AAPL"],
        "fetch_type": "profile",
    })).await;
    println!("\n***\nResult: {:?}\n***", r);
}
