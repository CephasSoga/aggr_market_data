pub mod cache;
pub mod auth_config;
pub mod commodity;
pub mod crypto;
pub mod etf;
pub mod financial;
pub mod forex;
pub mod index;
pub mod market;
pub mod mutualfund;
pub mod request;
pub mod search;
pub mod stock;
pub mod technical_indicators;
pub mod economic_data;
pub mod websocket;
pub mod utils;
pub mod request_parser;

use tokio;

use crate::stock::StockPolling;
use crate::cache::SharedLockedCache;
use auth_config::BatchConfig;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    //let _ = websocket::run().await;

    let cache = Arc::new(Mutex::new(SharedLockedCache::new(1000)));
    let config = Arc::new(BatchConfig::default());
    let stock_polling = StockPolling::new(cache, config);

    let r = stock_polling.profile("AAPL").await;
    println!("{:?}", r);
}
