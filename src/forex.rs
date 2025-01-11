#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::clone;
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

use crate::config::{RetryConfig, BatchConfig};

#[derive(Debug, Clone)]
pub enum FetchType {
    List,
    Rate,
    Historical

}

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
}

/// Functions for accessing foreign exchange rate data from the FMP API.
pub struct ForexPolling {
    cache: Arc<Mutex<LruCache<String, (Value, Instant)>>>,
    retry_config: Arc<RetryConfig>, //RetryConfig,
    batch_config: Arc<BatchConfig>,
    semaphore: Arc<Mutex<Semaphore>>,
}

impl ForexPolling {
    pub fn new() -> Self {
        let retry_config = Arc::new(RetryConfig::default());
        let batch_config = Arc::new(BatchConfig::default());
        Self { 
            cache: Arc::new(Mutex::new(LruCache::new(std::num::NonZeroUsize::new(100).unwrap()))),
            retry_config: Arc::clone(&retry_config),
            batch_config: Arc::clone(&batch_config),
            semaphore: Arc::new(Mutex::new(Semaphore::new(retry_config.rate_limit_per_second as usize))),
        }
    }

    pub async fn list(&self) -> Result<Value, ForexError> {
        let cahe_key = "forex/list";

        let fetch_fn = async {
            make_request("symbol/available-forex-currency-pairs", HashMap::new())
            .await
            .map_err(|e| ForexError::FetchError(format!("Failed to fetch forex list: {}", e.to_string())))
        };
        self.get_cached_or_fetch(&cahe_key, fetch_fn).await
    }

    pub async fn get_cached_or_fetch<F: Future<Output = Result<Value, ForexError>>>(
        &self, 
        key: &str, 
        fetch_fn: F
    ) -> Result<Value, ForexError> 
    where F: Future<Output = Result<Value, ForexError>> {
        let mut cache = self.cache.lock().await;
        if let Some((value, instant)) = cache.get(key) {
            if instant.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            } else {
                cache.pop(key);// Expired
            }
        }

        // Fetch data and store in cache
        let result = fetch_fn.await;
        match result {
            Ok(value) => {
                cache.put(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            },
            Err(e) => Err(ForexError::FetchError(e.to_string())),
        }
    }

    pub async fn rate(&self, from_and_to: (&str, &str)) -> Result<Value, ForexError> {
        let (from_curr, to_curr) = from_and_to;
        let symbol = format!("{}{}", from_curr, to_curr);

        let cache_key = format!("rate/{}", symbol);

        let fecth_fn = async {
            make_request(
                "quote",
                generate_json(Value::String(symbol), None)
            ).await
            .map_err(|e| ForexError::FetchError(e.to_string()))
        };

        self.get_cached_or_fetch(&cache_key, fecth_fn).await

    }

    pub async fn history(
        &self,
        from_and_to: (&str, &str),
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, ForexError> {
        let (from_curr, to_curr) = from_and_to;
        let symbol = format!("{}{}", from_curr, to_curr);

        let cache_key = format!("history/{}", symbol);

        let fetch_fn = async {
            let query_params = json!({
                "from": start_date,
                "to": end_date,
                "serietype": data_type,
                "timeseries": limit
            });

            make_request(
                "historical-price-full/forex",
                generate_json(Value::String(symbol), Some(query_params))
            ).await
            .map_err(|e| ForexError::FetchError(e.to_string()))
        };
        self.get_cached_or_fetch(&cache_key, fetch_fn).await
    }

    async fn validate_tickers(&self, tickers: Vec<String>) -> Result<Vec<String>, ForexError> {
        let valid_tickers = self.list().await?;
        let valid_tickers = valid_tickers.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect::<HashSet<_>>();
        let invalid_tickers = tickers.iter().filter(|t| !valid_tickers.contains(t.as_str())).collect::<Vec<_>>();
        if !invalid_tickers.is_empty() {
            return Err(ForexError::VoidTickersError(format!("Invalid tickers: {:?}", invalid_tickers)));
        }
        Ok(tickers)
    }

    async fn retry<F, Fut, T, E>(
        retry_config:  &RetryConfig,
        mut operation: F,
    )-> Result<T, E> 
    where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    {
        let mut attempts = 0;
        let mut delay = retry_config.base_delay_ms;
        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    if attempts >= retry_config.max_attempts {
                        return Err(e);
                    }
                    attempts += 1;
                    sleep(Duration::from_millis(delay)).await;
                    delay = delay * 2;
                    if delay > retry_config.max_delay_ms {
                        delay = retry_config.max_delay_ms;
                    }
                }
            }
        }
    }

    async fn process_batch(
        &self,
        tickers: Vec<(String, String)>,
        fetch_type: FetchType, 
    ) -> Result<Value, ForexError> {
        let tickers = tickers
            .into_iter()
            .map(|(from, to)| format!("{}{}", from, to))
            .collect::<Vec<_>>();
        let tickers = self.validate_tickers(tickers).await?;
        let tickers = tickers
            .into_iter()
            .map(|ticker| (ticker[..3].to_string(), ticker[3..].to_string()))
            .collect::<Vec<_>>();

        let concurrrency_limit = self.retry_config.concurrency_limit;

        let start = Instant::now();
        let mut tasks = Vec::new();

        //Shared self
        let shared_self = Arc::new(Self::new());
    
        
        for ticker in tickers {
            let retry_config = Arc::clone(&self.retry_config);
            let fetch_type = fetch_type.clone();
            let ticker = ticker.clone();
            let semaphore = self.semaphore.clone();

            let self_ref = Arc::clone(&shared_self);
            
            let task = tokio::spawn(async move {
                let bind = Arc::clone(&semaphore);
                let semaphore = bind.lock().await;
                let permit = &semaphore.acquire().await.unwrap();
                let operation = || async {
                    match fetch_type {
                        FetchType::List => self_ref.list().await,
                        FetchType::Rate => self_ref.rate((&ticker.0, &ticker.1)).await,
                        FetchType::Historical => {
                            self_ref.history((&ticker.0, &ticker.1), None, None, None, None).await
                        }
                        _ => unreachable!(),
                    }
                };

                let result = Self::retry(&retry_config, operation).await;

                // Drop the semaphore permit
                drop(permit);

                match result {
                    Ok(value) => {
                        counter!("forex.success").increment(1);
                        value
                    },
                    Err(e) => {
                        error!("Failed to fetch data for {}: {:?}{:?}", ticker.0, ticker.1, e);
                        counter!("forex.failures").increment(1);
                        Value::Null
                    },
                }
                    
            });
            tasks.push(task);

        }
    
        let results = futures::future::join_all(tasks).await;
        let mut values = Vec::new();
        for result in results {
            values.push(result.unwrap());
        }

        let elapsed = start.elapsed();
        gauge!("forex.batch_duration", "rate limit" => format!("{}", elapsed.as_secs_f64()));
        
        Ok(Value::Array(values))
    }

    pub async fn poll(&self, tickers: Vec<(String, String)>, fetch_type: FetchType) -> Result<(), ForexError> {
        let config = Arc::clone(&self.batch_config);
        
        let mut futures = vec![];

        for chunk in tickers.chunks(config.batch_size) {
            let chunk = chunk.to_vec();
            let fetch_type = fetch_type.clone();
            let self_ref = Self::new();
            let future = tokio::spawn(async move {
                self_ref.process_batch(chunk, fetch_type).await
            });
            futures.push(future);
        }

        for future in futures {
            match future.await {
                Ok(value) => info!("Task completed successfully: {:?}", value),
                Err(e) => error!("Task encountered an error: {:?}", e),
                
            }
        }

        Ok(())
    }    
}
