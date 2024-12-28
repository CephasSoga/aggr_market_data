#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use crate::request::{make_request, generate_json};
use serde_json::Value;
use std::collections::HashMap;

/// Functions for accessing general market data from the FMP API.
pub struct Market;

impl Market {
    pub async fn most_active() -> Result<Value, reqwest::Error> {
        make_request("stock/actives", HashMap::new()).await
    }

    pub async fn most_gainer() -> Result<Value, reqwest::Error> {
        make_request("stock/gainers", HashMap::new()).await
    }

    pub async fn most_loser() -> Result<Value, reqwest::Error> {
        make_request("stock/losers", HashMap::new()).await
    }

    pub async fn sector_performance() -> Result<Value, reqwest::Error> {
        make_request("stock/sectors-performance", HashMap::new()).await
    }

    pub async fn trading_hours() -> Result<Value, reqwest::Error> {
        make_request("is-the-market-open", HashMap::new()).await
    }
}


pub async fn example() -> Result<(), reqwest::Error> {
    // Get most active stocks
    let active = Market::most_active().await?;
    println!("ACTIVES: {:?}", active);
    
    // Get top gainers
    let gainers = Market::most_gainer().await?;
    
    // Get sector performance
    let sectors = Market::sector_performance().await?;
    
    // Check if market is open
    let is_open = Market::trading_hours().await?;
    
    Ok(())
}

