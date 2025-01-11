pub mod cache;
pub mod config;
//pub mod commodity;
//pub mod crypto;
//pub mod etf;
pub mod financial;
//pub mod forex;
//pub mod index;
//pub mod market;
//pub mod mutualfund;
pub mod request;
//pub mod search;
pub mod stock;
//pub mod technical_indicators;
//pub mod economic_data;
pub mod websocket;
pub mod utils;
pub mod request_parser;

use tokio;

use crate::stock::StockPolling;
use crate::cache::SharedLockedCache;
use crate::request::HTTPClient;
use config::BatchConfig;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    //let _ = websocket::run().await;

    let cache = Arc::new(Mutex::new(SharedLockedCache::new(1000)));
    let config = Arc::new(BatchConfig::default());
    let http_client = Arc::new(HTTPClient::new().expect("Failed to create HTTP client"));
    let stock_polling = StockPolling::new(cache, config, http_client);

    let r = stock_polling.split_history("AAPL").await;
    println!("{:?}", r);
}
