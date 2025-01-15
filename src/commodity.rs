#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]


use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;
use serde_json::{json, Value};
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use metrics::{counter, gauge};
use tokio::time::sleep;
use thiserror::Error;
use futures_util::Future;
use tokio::sync::Mutex;

use crate::request::HTTPClient;
use crate::config::{RetryConfig, BatchConfig};
use crate::options::{TimeFrame, DateTime, FetchType};
use crate::cache::{Cache,  SharedLockedCache};

const LIST_PATH: &str = "symbol/available-commodities";
const INTRADAY_PATH: &str = "historical-chart";
const DAILY_PATH: &str = "historical-price-full";

#[derive(Debug, Error)]
pub enum CommodityError {
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

#[derive(Debug, Deserialize, Serialize)]
struct Commodity {
    symbol: String,
    name: String,
    currency: String,
    stock_exchange: String,
    exchange_short_name: String,
}

pub struct CommodityPolling{
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}

impl CommodityPolling {
    pub fn new(
        http_client: Arc<HTTPClient>,
        cache: Arc<Mutex<SharedLockedCache>>, 
        batch_config: Arc<BatchConfig>,
        retry_config: Arc<RetryConfig>,
    ) -> Self {
        Self {http_client,  cache, batch_config, retry_config}
    }

    fn clone(&self) -> Self {
        Self {
            http_client: self.http_client.clone(),
            cache: self.cache.clone(),
            batch_config: self.batch_config.clone(),
            retry_config: self.retry_config.clone(),
        }
    }

    async fn get_cached_or_fetch<F: Future<Output = Result<Value, CommodityError>>>(
        &self, 
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) ->   Result<Value, CommodityError> 
    where F: Future<Output = Result<Value, CommodityError>> {
        println!("Looking in cache");
        let mut cache = self.cache.lock().await;
        if let Some((value, instant)) = cache.get(key).await {
            println!("Found in cache");
            if instant.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            } else {
                println!("Expired");
                cache.pop(key);// Expired
            }
        }
        println!("Fetching...");
        // Fetch and cache the value
        let result = fetch_fn.await;
        match result {
            Ok(value) => {
                println!("Got value: {:?}", value);
                cache.put(key.to_string(), (value.clone(), Instant::now())).await;
                Ok(value)
            }
            Err(e) => Err(CommodityError::FetchError(e.to_string())),
        }
    }

    pub async fn list(&self) -> Result<Value, reqwest::Error> {
        self.http_client.get(LIST_PATH, None).await
    }

    pub async fn intraday(
        &self,symbol: &str, 
        timeframe: &str, 
        from: Option<String>, 
        to: Option<String>
    ) -> Result<Value, reqwest::Error> {
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
        let timeframe_value = TimeFrame::from_str(timeframe)
            .unwrap_or(TimeFrame::FiveMinutes);
        let timeframe = timeframe_value.to_str();
        let path = self.http_client.join(vec![INTRADAY_PATH, timeframe, symbol]);
        self.http_client.get(
            path.as_str(), 
            Some(query_params))
        .await
        
    }

    pub async fn daily(&self, symbol: &str) -> Result<Value, reqwest::Error> {
        let path = self.http_client.join(vec![DAILY_PATH, symbol]);
        self.http_client.get(&path, None).await
    }

    async fn process_batch(&self, 
        batch: Vec<String>,
        fetch_type: FetchType, 
        timeframe: &str, 
        from: &Option<String>, 
        to: &Option<String>
    ) -> Result<(), CommodityError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / self.batch_config.rate_limit_per_second as f32
        ));

        for symbol in batch {
            rate_limiter.tick().await;

            let mut attempts = 0;
            while attempts < self.batch_config.retry_attempts {
                match self.fetch_symbol_data(&symbol, fetch_type, timeframe, from.clone(), to.clone()).await {
                    Ok(_) => {
                        counter!("commodity.success").increment(1);
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        counter!("commodity.retries").increment(1);
                        if attempts == self.batch_config.retry_attempts {
                            error!("Final retry failed for {}: {:?}", symbol, e);
                            counter!("commodity.failures").increment(1);
                        } else {
                            sleep(Duration::from_millis(self.batch_config.backoff_ms * attempts as u64)).await;
                        }
                    }
                }
            }
        }

        let gauge = gauge!("commodity.batch_duration", "rate limit" => format!("{}", start.elapsed().as_secs_f64()));
        gauge.decrement(1);
        Ok(())
    }

    async fn fetch_symbol_data(
        &self, 
        symbol: &str, 
        fetch_type: FetchType, 
        timeframe: &str, 
        from: Option<String>, 
        to: Option<String>
    ) -> Result<Value, CommodityError> {
        let data = match fetch_type {
            FetchType::IntraDay => {
                self.intraday(symbol, timeframe, from, to).await
            }
            FetchType::Daily => {
                self.daily(symbol).await
            }
            _ => {Ok(Value::Null)}
        }
        .map_err(|e| CommodityError::FetchError(e.to_string()))?;

        
        Ok(data)
    }


    async fn validate_tickers(&self, tickers: Vec<String>) -> Result<Vec<String>, CommodityError> {
        if tickers.len() > self.batch_config.batch_size {
            return Err(CommodityError::TooManyTickersError(format!("Too many tickers: {}", tickers.len())));
        } else if tickers.is_empty() {
            return Err(CommodityError::VoidTickersError("No tickers provided".to_string()));
        }
        Ok(tickers)
    }

    pub async fn poll(&self, 
        tickers: Vec<String>,
        fetch_type: FetchType,
        timeframe: &str,
        from: Option<String>,
        to: Option<String>
    ) -> Result<(), CommodityError> {
        let config = self.batch_config.clone();

        let symbols = self.validate_tickers(tickers).await?;

        info!("Polling {} available commodities", symbols.len());
        let semaphore = std::sync::Arc::new(Semaphore::new(config.concurrency_limit));
        let mut tasks = vec![];

        for chunk in symbols.chunks(config.clone().batch_size) {
            let batch = chunk.to_vec();
            let semaphore_clone = semaphore.clone();

            let self_clone = self.clone();

            let fetch_type_clone = fetch_type.clone();
            let timeframe_clone = timeframe.to_string();
            let from_clone = from.clone();
            let to_clone = to.clone();

            let config_clone = config.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore_clone.acquire().await.map_err(|e| CommodityError::TaskError(e.to_string()))?;
                self_clone.process_batch(batch, fetch_type_clone, &timeframe_clone.to_string(), &from_clone, &to_clone).await
            });
            tasks.push(task);
        }

        for task in tasks {
            match task.await {
                Ok(result) => {
                    if let Err(e) = result {
                        error!("Batch processing error: {:?}", e);
                    }
                }
                Err(e) => error!("Task failure: {:?}", e),
            }
        }

        Ok(())
    }
}
