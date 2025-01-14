#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::clone;
use std::collections::HashMap;

use crate::request::HTTPClient;
use tokio::time::sleep;
use serde_json::{json, to_value, Value};
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use metrics::{counter, gauge};
use tracing::{info, error};
use std::sync::Arc;
use serde::{Deserialize, Serialize};

use futures_util::Future;
use tokio::sync::Mutex;
use thiserror::Error;

use crate::utils::{retry, clone_str_options, clone_arc_refs};
use crate::config::{RetryConfig, BatchConfig};
use crate::cache::{Cache, SharedLockedCache};
use crate::options::{DateTime, TimeFrame, FetchType};

const LIST_PATH: &str = "symbol/available-forex-currency-pairs";
const INTRADAY_PATH: &str = "history-chart";
const DAILY_PATH: &str = "historical-price-full";



#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Forex{
    symbol: String,
    name: String,
    currency: String,
    stock_exchange: String,
    exchange_short_name: String,
}

#[derive(Debug, Error)]
pub enum ForexError {
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

/// Functions for accessing foreign exchange rate data from the FMP API.
pub struct ForexPolling {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    retry_config: Arc<RetryConfig>, //RetryConfig,
    batch_config: Arc<BatchConfig>,
}

impl ForexPolling {
    pub fn new(http_client: Arc<HTTPClient>, cache: Arc<Mutex<SharedLockedCache>>, batch_config: Arc<BatchConfig>, retry_config: Arc<RetryConfig>) -> Self {

        Self { 
            http_client,
            cache,
            batch_config,
            retry_config
        }
    }

    pub fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            cache: self.cache.clone(),
            batch_config: self.batch_config.clone(),
            retry_config: self.retry_config.clone(),
        }
    }

    pub async fn list(&self) -> Result<Value, reqwest::Error> {
        self.http_client.get(LIST_PATH, None).await
    }

    fn normalize_symbol(symbol: &str) -> String {
        let symbol = symbol.to_uppercase();
        if !symbol.to_lowercase().contains("usd") {
            format!("{}USD", symbol)
        } else {
            symbol
        }
    }

    async fn get_from_cache_or_fetch<F, Fut>(
        &self,
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) -> Result<Value, ForexError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, reqwest::Error>>,
    {
        let mut cache = self.cache.lock().await;

        // Check cache
        if let Some((value, timestamp)) = cache.get(key).await {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            } else {
                cache.pop(key); // Expired
            }
        }

        // Fetch and cache
        match fetch_fn().await {
            Ok(value) => {
                cache.put(key.to_string(), (value.clone(), Instant::now())).await;
                Ok(value)
            }
            Err(err) => Err(ForexError::FetchError(err.to_string())),
        }
    }


    pub async fn intraday(
        &self,
        symbol: &str, 
        timeframe: &str, 
        from: Option<&str>, 
        to: Option<&str>
    ) -> Result<Value, reqwest::Error> {

        let symbol = Self::normalize_symbol(symbol);
        let mut query_params = json!({});

        if let Some(from) = from {
            let from = DateTime::from_str(&from)
                .expect("Invalid `from` date arguments")
                .to_string();
            query_params["from"] = json!(from);
        }
   
        if let Some(to) = to {
            let to = DateTime::from_str(&to)
                .expect("Invalid `to` date arguments")
                .to_string();
            query_params["to"] = json!(to);
        }
        
        let query_params = self.http_client.build_query_from_value(query_params);
        let timeframe_value = TimeFrame::from_str(&timeframe)
            .unwrap_or(TimeFrame::FiveMinutes);
        let timeframe = timeframe_value.to_str();
        let path = self.http_client.join(vec![INTRADAY_PATH, timeframe, &symbol]);
        self.http_client.get(
            path.as_str(), 
            Some(query_params))
        .await
        
    }

    pub async fn daily(&self, symbol: &str) -> Result<Value, reqwest::Error> {
        let symbol = Self::normalize_symbol(symbol);
        let path = self.http_client.join(vec![DAILY_PATH, &symbol]);
        self.http_client.get(&path, None).await
    }
    
    async fn ticker_level_concurrency(
        &self, 
        batch: Vec<String>,
        fetch_type: FetchType,
        timeframe: Option<String>,
        from: Option<String>,
        to: Option<String>,
    ) -> Value {
        if batch.is_empty() {
            return Value::Null;
        }
    
        let mut tasks = vec![];
    
        for symbol in batch {
            let retry_config_clone = self.retry_config.clone();
            let (timeframe, from, to) = clone_str_options((&timeframe, &from, &to));
            let symbol_clone = symbol.clone();
            let self_clone = self.clone();
    
            let task = tokio::spawn(async move {
                
                retry(&retry_config_clone, || async {
                    self_clone.fetch_crypto_data(&symbol_clone, fetch_type, timeframe.as_deref(), from.as_deref(), to.as_deref()).await
                }).await
            });
            tasks.push(task);
        }
    
        let results: Vec<_> = futures::future::join_all(tasks).await;
        let mut values = vec![];

        for result in results {
            match result {
                Ok(Ok(value)) => values.push(value),
                Ok(Err(e)) => {
                    error!("Failed to fetch data: {:?}", e);
                    values.push(json!({"error": e.to_string()}));
                }
                Err(e) => {
                    error!("Task panicked: {:?}", e);
                    values.push(json!({"error": "Task panicked"}));
                }
            }
        }

        serde_json::json!(values)
    }
    

    async fn fetch_crypto_data(
        &self, 
        symbol: &str, 
        fetch_type: FetchType,
        timeframe: Option<&str>, 
        from: Option<&str>,
        to: Option<&str>
    ) -> Result<Value, ForexError> {
        match fetch_type {
            FetchType::IntraDay => {
                let key = format!("{}-{}-{}-{}", symbol, timeframe.unwrap_or(""), from.unwrap_or(""), to.unwrap_or(""));
                let retry_cfg = self.retry_config.clone();
                let timeframe = timeframe.unwrap_or(TimeFrame::FiveMinutes.to_str());

                retry(&retry_cfg, || async {
                    self.get_from_cache_or_fetch(&key, || async {
                        self.intraday(symbol, timeframe, from, to).await
                    }, self.batch_config.cache_ttl).await
                })
                .await
                .map_err(|err| ForexError::FetchError(err.to_string()))
            },
            FetchType::Daily => {
                let key = format!("forex_daily_{}", symbol);
                let retry_cfg = self.retry_config.clone();

                retry(&retry_cfg, || async {
                    self.get_from_cache_or_fetch(&key, || async {
                        self.daily(symbol).await
                    }, self.batch_config.cache_ttl).await
                })
                .await
                .map_err(|err| ForexError::FetchError(err.to_string()))
            },
            _ => Err(ForexError::TaskError(format!("Invalid fecth type: {:?}", fetch_type))),
        }
    }

    async fn validate_tickers(&self, tickers: Vec<String>) -> Result<Vec<String>, ForexError> {
        if tickers.len() > self.batch_config.batch_size {
            return Err(ForexError::TooManyTickersError(format!("Too many tickers: {}", tickers.len())));
        } else if tickers.is_empty() {
            return Err(ForexError::VoidTickersError("No tickers provided".to_string()));
        }
        Ok(tickers)
    }

    pub async fn batch_level_concurrency(
        &self, 
        tickers: Vec<String>,
        fetch_type: FetchType,
        timeframe: Option<String>,
        from: Option<String>,
        to: Option<String>
    ) -> Result<Value, ForexError> {
        let config = self.batch_config.clone();
        
        let symbols = self.validate_tickers(tickers).await?;
    
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        let mut tasks = vec![];
    
        for chunk in symbols.chunks(config.batch_size) {
            let batch = chunk.to_vec();
            let (batc_config, semaphore, retry_config) = clone_arc_refs((&self.batch_config, &semaphore, &self.retry_config));
            let (timeframe, from, to) = clone_str_options((&timeframe, &from, &to));

    
            let self_clone = self.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire().await
                    .map_err(|e| ForexError::TaskError(e.to_string()));
                self_clone.ticker_level_concurrency(batch, fetch_type, timeframe, from, to).await
            });
            tasks.push(task);
        }
    
        let results: Vec<_> = futures_util::future::join_all(tasks).await;
        let mut values = vec![];
    
        for result in results {
            match result {
                Ok(value) => values.push(value),
                Err(e) => {
                    error!("Task failure: {:?}", e);
                    values.push(json!({"error": "Task panicked"}));
                }
            }
        }
    
        Ok(Value::Array(values))
    }

    pub async fn poll(&self, value: &Value) -> Result<Value, ForexError> {
        let tickers = value.get("tickers")
        .and_then(Value::as_array)
        .ok_or(ForexError::ParseError("Missing 'tickers' field".to_string()))?
        .iter()
        .filter_map(Value::as_str)
        .map(Self::normalize_symbol)
        .collect::<Vec<String>>();

    let fetch_type_str = value.get("fetch_type")
        .and_then(Value::as_str)
        .ok_or(ForexError::ParseError("Missing 'fetch_type' field".to_string()))?;

    let fetch_type = FetchType::from_str(fetch_type_str);

    let timeframe = value.get("timeframe").and_then(Value::as_str).map(String::from);
    let from = value.get("from").and_then(Value::as_str).map(String::from);
    let to = value.get("to").and_then(Value::as_str).map(String::from);
    
    self.batch_level_concurrency(tickers, fetch_type, timeframe, from, to).await
    }
}
