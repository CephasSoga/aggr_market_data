#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::Future;
use metrics::{counter, gauge};
use tracing::{info, error};
use tokio::sync::Semaphore;
use tokio::sync::Mutex;
use serde_json::{json, Value};
use thiserror::Error;

use crate::request::HTTPClient;
use crate::config::{RetryConfig, BatchConfig};
use crate::cache::{Cache, SharedLockedCache};
use crate::options::{TimeFrame, DateTime, FetchType};
use crate::utils::{clone_arc_refs, clone_str_options, retry, now};

const GAINERS_PATH: &str = "stock_market/gainers";
const LOSERS_PATH: &str = "stock_market/losers";
const ACTIVES_PATH: &str = "stock_market/active";
const PERFORMANCE_PATH: &str = "sector-performance";
const HISTORICAL_PERFORMANCE_PATH: &str = "historical-sectors-performance";
const SECTOR_PRICE_EARNING_RATIO_PATH: &str = "sector_price_earning_ratio"; //V4 only
const INDUSTRY_PRICE_EARNING_RATIO_PATH: &str = "industry_price_earning_ratio"; //V4 only

#[derive(Debug, Error)]
pub enum MarketError {
    #[error("Failed to fetch data: {0}")]
    FetchError(String),
    
    #[error("Task encountered an error: {0}")]
    TaskError(String),
    
    #[error("Failed to parse data: {0}")]
    ParseError(String),
    
    #[error("No tickers provided: {0}")]
    VoidTickersError(String),

    #[error("Invalid ticker: {0}")]
    InvalidTicker(String),

    #[error("Too many tickers: {0}")]
    TooManyTickersError(String),
}

pub struct MarketPolling {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}

impl MarketPolling {
    pub fn new(
        http_client: Arc<HTTPClient>, 
        cache: Arc<Mutex<SharedLockedCache>>, 
        batch_config: Arc<BatchConfig>, 
        retry_config: Arc<RetryConfig>) -> Self {
        Self {
            http_client,
            cache,
            batch_config,
            retry_config,
        }
    }

    fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            cache: self.cache.clone(),
            batch_config: self.batch_config.clone(),
            retry_config: self.retry_config.clone(),
        }
    }

    async fn get_from_cache_or_fetch<F, Fut>(
        &self,
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) -> Result<Value, MarketError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, MarketError>>,
    {
        let mut cache = self.cache.lock().await;
        if let Some((value, timestamp)) = cache.get(key).await {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            }
            cache.pop(key);
        }

        match fetch_fn().await {
            Ok(value) => {
                cache.put(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            }
            Err(err) => Err(MarketError::FetchError(err.to_string())),
        }
    }

    async fn fetch_market_data(
        &self, 
        fetch_type: FetchType, 
        date: Option<String>, 
        from: Option<String>, 
        to: Option<String>,
        exchange: Option<String>, 
        limit: Option<u32>
    ) -> Result<Value, MarketError> {
        match fetch_type {
            FetchType::Gainers => {
                let key = "market_gainers";
                let ttl = self.batch_config.cache_ttl;
                let op = self.most_gainer();
                self.get_from_cache_or_fetch(key, || async { op.await }, ttl).await
            },
            FetchType::Losers => {
                let key = "market_losers";
                let ttl = self.batch_config.cache_ttl;
                let op = self.most_loser();
                self.get_from_cache_or_fetch(key, || async { op.await }, ttl).await
            },
            FetchType::Actives => {
                let key = "market_actives";
                let ttl = self.batch_config.cache_ttl;
                let op = self.most_active();
                self.get_from_cache_or_fetch(key, || async { op.await }, ttl).await
            },
            FetchType::Performance => {
                let key = "market_performance";
                let ttl = self.batch_config.cache_ttl;
                let op = self.sector_performance();
                self.get_from_cache_or_fetch(key, || async { op.await }, ttl).await
            },
            FetchType::SectorHistorical => {
                let key = "market_sector_historical";
                let ttl = self.batch_config.cache_ttl;
                let op = self.historical_sector_performance(from, to, limit);
                self.get_from_cache_or_fetch(key, || async { op.await }, ttl).await
            },
            FetchType::SectorPERatio => {
                let key = format!("market_sector_peratio/{}", exchange.clone().unwrap_or("".to_string()));
                let ttl = self.batch_config.cache_ttl;
                let date = &date.unwrap_or(now().split(" ").next().unwrap().to_string());
                let op =self.sector_price_earning_ratio(date, exchange);
                self.get_from_cache_or_fetch(&key, || async { op.await }, ttl).await
            },
            FetchType::IndustryPERatio => {
                let date = &date.unwrap_or(now().split(" ").next().unwrap().to_string());
                let key = format!("market_industry_peratio/{}", exchange.clone().unwrap_or("".to_string()));
                let ttl = self.batch_config.cache_ttl;
                let op = self.industry_price_earning_ratio(date, exchange);
                self.get_from_cache_or_fetch(&key, || async { op.await }, ttl).await
            },
            _ => Err(MarketError::TaskError("Invalid fetch type".to_string())),
        }
    }


    async fn most_active(&self) -> Result<Value, MarketError> {
        self.http_client.get(ACTIVES_PATH, None)
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn most_gainer(&self) -> Result<Value, MarketError> {
        self.http_client.get(GAINERS_PATH, None)
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn most_loser(&self) -> Result<Value, MarketError> {
        self.http_client.get(LOSERS_PATH, None)
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn sector_performance(&self) -> Result<Value, MarketError> {
        self.http_client.get(PERFORMANCE_PATH, None)
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn historical_sector_performance(&self, from: Option<String>, to: Option<String>, limit: Option<u32>) -> Result<Value, MarketError> {
        let query_params = json!({
            "from": from,
            "to": to,
            "limit": limit
        });
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(HISTORICAL_PERFORMANCE_PATH, Some(query_params))
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn sector_price_earning_ratio(&self, date: &str, exchange: Option<String>) -> Result<Value, MarketError> {
        let query_params = json!({
            "date": date,
            "exchange": exchange
        });
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get_v4(SECTOR_PRICE_EARNING_RATIO_PATH, Some(query_params))
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }

    async fn industry_price_earning_ratio(&self, date: &str, exchange: Option<String>) -> Result<Value, MarketError> {
        let query_params = json!({
            "date": date,
            "exchange": exchange
        });
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get_v4(INDUSTRY_PRICE_EARNING_RATIO_PATH, Some(query_params))
            .await
            .map_err(|e| MarketError::FetchError(e.to_string()))
    }



    pub async fn poll(&self, value: &Value) -> Result<Value, MarketError> {
        let fetch_type = value.get("fetch_type")
            .and_then(Value::as_str)
            .map(FetchType::from_str)
            .ok_or(MarketError::ParseError("Fetch type not defined".to_string()))?;

        let date = value.get("date")
            .and_then(Value::as_str)
            .map(String::from);


        let from = value.get("from")
            .and_then(Value::as_str)
            .map(String::from);

        let to = value.get("to")
            .and_then(Value::as_str)
            .map(String::from);

        let exchange = value.get("exchange")
            .and_then(Value::as_str)
            .map(String::from);

        let limit = value.get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as u32);

        self.fetch_market_data(fetch_type, date, from, to, exchange, limit)
           .await
    }
}