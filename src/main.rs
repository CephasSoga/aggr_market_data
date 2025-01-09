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

#[tokio::main]
async fn main() {
    let _ = websocket::main_().await;
}
