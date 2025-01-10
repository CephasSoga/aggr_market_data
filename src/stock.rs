#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::collections::HashMap;

use crate::request::{make_request, generate_json};
use crate::financial::Financial;
use serde::de::value;
use tokio::time::sleep;
use serde_json::{json, to_value, Value};
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use metrics::{counter, gauge};
use tracing::{info, error};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use futures_util::Future;
use std::fmt::Display;
use lru::LruCache;
use tokio::sync::Mutex;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::auth_config::{RetryConfig, BatchConfig};
use crate::cache::{Cache, SharedCache, SharedLockedCache};


pub trait FunctionArgs {
    fn expected_args() -> Vec<&'static str>;
}

#[derive(Clone, Copy)]
pub enum FetchType {
    Quote,
    Financial,
    Profile,
    Rating,
    CurrentPrice,
    History,
    DividendHistory,
    SplitHistory,
}
impl FetchType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "quote" => FetchType::Quote,
            "financial" => FetchType::Financial,
            "profile" => FetchType::Profile,
            "rating" => FetchType::Rating,
            "current_price" => FetchType::CurrentPrice,
            "history" => FetchType::History,
            "dividend_history" => FetchType::DividendHistory,
            "split_history" => FetchType::SplitHistory,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Stock{
    pub symbol: String,
    pub exchange: String,
    pub exchange_short_name: String,
    pub price: String,
    pub name: String,
}

#[derive(Debug, Error)]
pub enum StockError {
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

/// Functions for accessing stock-related data from the FMP API.
pub struct StockPolling {
   cache: Arc<Mutex<SharedLockedCache>>,
   batch_config: Arc<BatchConfig>,
}

impl StockPolling {
    pub fn new(cache: Arc<Mutex<SharedLockedCache>>, batch_config:Arc<BatchConfig>) -> Self {
        Self {
            cache,
            batch_config,
        }
    }

    async fn get_cached_or_fetch<F: Future<Output = Result<Value, StockError>>>(
        &self, 
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) ->   Result<Value, StockError> 
    where F: Future<Output = Result<Value, StockError>> {
        let mut cache = self.cache.lock().await;
        if let Some((value, instant)) = cache.get(key).await {
            if instant.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            } else {
                cache.pop(key);// Expired
            }
        }
        
        // Fetch and cache the value
        let result = fetch_fn.await;
        match result {
            Ok(value) => {
                cache.put(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            }
            Err(e) => Err(StockError::FetchError(e.to_string())),
        }
    }

    pub async fn list(&self) -> Result<Value, StockError> {
        let cache_key = format!("stock/list");
        
        let fetch_fn = async  {
            make_request("stock/list", HashMap::new()).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch stock list: {}", e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn profile(&self, symbol: &str) -> Result<Value, StockError> {
        let cache_key = format!("stock/{}", symbol);
        
        let fetch_fn = async  {
            make_request(
                "profile",
                generate_json(Value::String(symbol.to_string()), None)
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch stock profile for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn quote(&self, symbol: &str) -> Result<Value, StockError> {
        let cache_key = format!("quote/{}", symbol);
        
        let fetch_fn = async {
            make_request(
                "quote",
                generate_json(Value::String(symbol.to_string()), None)
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch quote for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn financial(&self, symbol: &str) -> Result<Value, StockError> {
        let cache_key = format!("financial/{}", symbol);
        
        let fetch_fn = async {
            let financial_req = Financial::new(symbol);
            let res_hash = financial_req.all().await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch financial data for {}: {}", symbol, e.to_string())))
            .unwrap();

        let result = to_value(res_hash)
            .map_err(|e| StockError::ParseError(e.to_string()));
        match result {
            Ok(value) => Ok(value),
            Err(e) => Err(StockError::ParseError(format!("Failed to parse financial data for {}: {}", symbol, e.to_string()))),
        }
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn rating(&self, symbol: &str) -> Result<Value, StockError> {
        let cache_key = format!("rating/{}", symbol);
        
        let fetch_fn = async {
            make_request(
                "company/rating",
                generate_json(Value::String(symbol.to_string()), None)
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch rating for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn current_price(&self, symbol: &str) -> Result<Value, StockError> {
        let cache_key = format!("current_price/{}", symbol);
        
        let fetch_fn = async {
            make_request(
                "stock/real-time-price",
                generate_json(Value::String(symbol.to_string()), None)
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch current price for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, StockError> {
        let cache_key = format!("history/{}", symbol);
        
        let fetch_fn = async {
            let query_params = json!({
                "from": start_date,
                "to": end_date,
                "serietype": data_type,
                "timeseries": limit
            });

            make_request(
                "historical-price-full",
                generate_json(Value::String(symbol.to_string()), Some(query_params))
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch history for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn dividend_history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, StockError> {
        let cache_key = format!("dividend_history/{}", symbol);
        
        let fetch_fn = async {
            let query_params = json!({
                "from": start_date,
                "to": end_date,
                "serietype": data_type,
                "timeseries": limit
            });

            make_request(
                "historical-price-full/stock_dividend",
                generate_json(Value::String(symbol.to_string()), Some(query_params))
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch dividend history for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    pub async fn split_history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, StockError> {
        let cache_key = format!("split_history/{}", symbol);
        
        let fetch_fn = async {
            let query_params = json!({
                "from": start_date,
                "to": end_date,
                "serietype": data_type,
                "timeseries": limit
            });

            make_request(
                "historical-price-full/stock_split",
                generate_json(Value::String(symbol.to_string()), Some(query_params))
            ).await
            .map_err(|e| StockError::FetchError(format!("Failed to fetch split history for {}: {}", symbol, e.to_string())))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn, self.batch_config.cache_ttl).await
    }

    async fn validate_tickers(&self, tickers: Vec<String>) -> Result<Vec<String>, StockError> {
        //let valid_tickers = self.list().await?;
        //if valid_tickers.is_null() {
        //    return Err(StockError::VoidTickersError("No tickers found (which is weird...)".to_string()));
        //}
        //let valid_tickers_array = valid_tickers.as_array().ok_or_else(|| StockError::ParseError("Failed to parse valid tickers".to_string()))?;
        //let valid_tickers = valid_tickers_array.iter()
        //    .filter_map(|v| serde_json::from_value::<Stock>(v.clone()).ok())
        //    .map(|stock| stock.symbol)
        //    .collect::<HashSet<_>>();
        //let invalid_tickers = tickers.iter().filter(|t| !valid_tickers.contains(t.as_str())).collect::<Vec<_>>();
        //if !invalid_tickers.is_empty() {
        //    return Err(StockError::VoidTickersError(format!("Invalid tickers: {:?}", invalid_tickers)));
        //}
        if tickers.len() > self.batch_config.batch_size {
            return Err(StockError::TooManyTickersError(format!("Too many tickers: {}", tickers.len())));
        } else if tickers.is_empty() {
            return Err(StockError::VoidTickersError("No tickers provided".to_string()));
        }
        Ok(tickers)
    }

    async fn retry<F, Fut, T, E>(
        config: &RetryConfig,
        mut operation: F,
    ) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let mut attempts = 0;
    
        loop {
            attempts += 1;
            match operation().await {
                Ok(value) => return Ok(value),
                Err(err) if attempts < config.max_attempts => {
                    let delay = std::cmp::min(
                        config.base_delay_ms * (2u64.pow(attempts - 1)),
                        config.max_delay_ms,
                    );
                    sleep(Duration::from_millis(delay)).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
    
    async fn process_batch(
        &self,
        tickers: Vec<String>,
        fetch_type: FetchType,
    ) -> Value {
        if tickers.is_empty() {
            return Value::Null;
        }
    
        // Configurable concurrency limit
        let concurrency_limit = 10; // Adjust as needed
        let semaphore = Arc::new(Semaphore::new(concurrency_limit));
        let retry_config = RetryConfig::default();
    
        let start = Instant::now();
        let mut tasks = Vec::new();

        let cache_clone = Arc::clone(&self.cache);
        let config_clone = Arc::clone(&self.batch_config);

        // Shared self
        let shared_self = Arc::new(Self::new(cache_clone, config_clone));
    
        for ticker in tickers {
            let permit = semaphore.clone().acquire_owned().await.unwrap(); // Acquire semaphore permit
            let retry_config = retry_config.clone(); // Clone retry config for each task
            let fetch_type = fetch_type.clone(); // Clone fetch type
            let ticker = ticker.clone(); // Clone ticker for task

            //Recursively clone Self
            let self_clone = Arc::clone(&shared_self);   
    
            let task = tokio::spawn(async move {
                let operation = || async {
                    match fetch_type {
                        FetchType::Quote => self_clone.quote(&ticker).await,
                        FetchType::Financial => self_clone.financial(&ticker).await,
                        FetchType::Profile => self_clone.profile(&ticker).await,
                        FetchType::Rating => self_clone.rating(&ticker).await,
                        FetchType::CurrentPrice => self_clone.current_price(&ticker).await,
                        FetchType::History => self_clone.history(&ticker, None, None, None, None).await,
                        FetchType::DividendHistory => self_clone.dividend_history(&ticker, None, None, None, None).await,
                        FetchType::SplitHistory => self_clone.split_history(&ticker, None, None, None, None).await,
                        _ => unreachable!(),
                    }
                };
    
                let result = Self::retry(&retry_config, operation).await;
    
                // Release semaphore automatically when task completes
                drop(permit);
    
                match result {
                    Ok(value) => {
                        counter!("stock.success").increment(1);
                        value
                    }
                    Err(e) => {
                        error!("Failed to fetch data for {}: {:?}", ticker, e);
                        counter!("stock.failures").increment(1);
                        Value::Null
                    }
                }
            });
    
            tasks.push(task);
        }
    
        // Await all tasks
        let mut results = Vec::new();
        for task in tasks {
            match task.await {
                Ok(value) => results.push(value),
                Err(_) => results.push(Value::Null), // Task panicked
            }
        }
    
        let elapsed = start.elapsed();
        gauge!("stock.batch_time", "rate limit" => format!("{}", elapsed.as_secs_f64()));
    
        Value::Array(results)
    }

   

    async fn poll_(&self, tickers: Vec<String>, fetch_type: FetchType) -> Result<Value, StockError> {
        let config = Arc::clone(&self.batch_config);

        let tickers = self.validate_tickers(tickers).await?;

        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        let mut futures= vec![];
        for chunk in tickers.chunks(config.batch_size) {
            let semaphore = Arc::clone(&semaphore);
            let batch = chunk.to_vec();

            let cache_clone = Arc::clone(&self.cache);

            let self_clone = Self::new(cache_clone, config.clone());

            let future = tokio::spawn({
                async move {
                    let _permit = semaphore.acquire().await.unwrap();
                    let value= self_clone.process_batch(batch, fetch_type).await;
                    drop(_permit);
                    value
                }
                
            });
            futures.push(future);
        }

        let mut results = Vec::new();
        for future in futures {
            match future.await {
                Ok(value) => results.push(value),
                Err(e) => {
                    error!("Failed to fetch data: {}", e);
                    results.push(Value::Null)
                }
            }
        }

        Ok(Value::Array(results))
    }


    pub async fn poll(&self, value: &Value) -> Result<Value, StockError> {
        let tickers = value.get("tickers")
        .and_then(Value::as_array)
        .ok_or(StockError::ParseError("Missing 'tickers' field".to_string()))?
        .iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect::<Vec<String>>();

    let fetch_type_str = value.get("fetch_type")
        .and_then(Value::as_str)
        .ok_or(StockError::ParseError("Missing 'fetch_type' field".to_string()))?;

    let fetch_type = FetchType::from_str(fetch_type_str);
    
    self.poll_(tickers, fetch_type).await
    }
}

impl FunctionArgs for fn(Value) -> Result<(), StockError> {
    fn expected_args() -> Vec<&'static str> {
        vec!["tickers", "fetch_type"]
    }
}
