#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::sync::Arc;
use std::fmt::Display;
use std::time::{Duration, Instant};

use crate::utils::{clone_str_options, clone_arc_refs, retry};
use clap::builder::Str;
use serde::de::value;
use tokio::time::sleep;
use serde_json::{json, to_value, Value};
use tokio::sync::Semaphore;
use metrics::{counter, gauge};
use tracing::{info, error};
use serde::{Deserialize, Serialize};
use futures_util::Future;
use tokio::sync::Mutex;
use thiserror::Error;

use crate::request::HTTPClient;
use crate::financial::Financial;
use crate::options::{DateTime, TimeFrame, FetchType, IndicatorType};
use crate::cache::{Cache, SharedLockedCache};
use crate::config::{RetryConfig, BatchConfig};

const INDICATOR_PATH: &str = "technical_indicator";




#[derive(Debug, Error, Serialize, Deserialize)]
pub enum IndicatorError {
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

pub struct TechnicalIndicatorPolling {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}
impl TechnicalIndicatorPolling {
    pub fn new(
        http_client: Arc<HTTPClient>,
        cache: Arc<Mutex<SharedLockedCache>>, 
        batch_config: Arc<BatchConfig>, 
        retry_config: Arc<RetryConfig>
    ) -> Self {
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

    async fn get_from_cache_or_fetch<F, Fut>(
        &self, 
        key: &str, 
        fetch_fn: F, 
        ttl: Duration
    ) 
    -> Result<Value, IndicatorError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, IndicatorError>>,
    {
        let mut cache = self.cache.lock().await;
        if let Some((value, instant)) = cache.get(key).await {
            if instant.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            } else {
                cache.pop(key).await;// Expired
            }
        }
        
        // Fetch and cache the value
        let result = fetch_fn().await;
        match result {
            Ok(value) => {
                cache.put(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            }
            Err(e) => Err(IndicatorError::FetchError(e.to_string())),
        }
    }

    async fn get_indicator_value(
        &self, 
        ticker: &str, 
        indicator: &IndicatorType, 
        timeframe: &TimeFrame, 
        period: u64,
        from: Option<DateTime>,
        to: Option<DateTime>
    ) -> Result<Value, IndicatorError> {
        let path = self.http_client.join(vec![INDICATOR_PATH, timeframe.to_str(), ticker]);
        println!("Path: {}", path);
        let query_params = json!({
            "type": indicator.as_str(),
            "period": period.to_string(),
            "from": from.map(|d| d.year_month_and_day()),
            "to": to.map(|d| d.year_month_and_day()),
        });

        let query_params = self.http_client.build_query_from_value(query_params);

        self.http_client.get(path.as_str(), Some(query_params))
            .await
            .map_err(|e| IndicatorError::FetchError(e.to_string()))

    }

    async fn get_indicators_for_ticker(
        &self, 
        ticker: &str,
        fetch_type: FetchType, 
        indicator: String, 
        timeframe: String, 
        period: u64,
        from: Option<String>,
        to: Option<String>
    ) -> Result<Value, IndicatorError> {
        match fetch_type {
            FetchType::TechnicalIndicator => {
                let indicator = IndicatorType::from_str(&indicator).unwrap_or(IndicatorType::sma);
                let timeframe = TimeFrame::from_str(&timeframe).unwrap_or(TimeFrame::OneDay);
                let from = from.map(|s| DateTime::from_str(&s)
                    .map_err(|e| IndicatorError::ParseError(e.to_string()))
                    .unwrap().to_owned());
                let to = to.map(|s| DateTime::from_str(&s)
                    .map_err(|e| IndicatorError::ParseError(e.to_string()))
                    .unwrap().to_owned());
                
                let retry_cfg = self.retry_config.clone();
                let key = format!("{}-{}-{}-{}-{:?}-{:?}", ticker, indicator, timeframe, period, &from, &to);
                retry(&retry_cfg,  || async {
                    let from  = from.clone();
                    let to = to.clone();
                    self.get_from_cache_or_fetch(
                        &key, 
                        || async {
                            self.get_indicator_value(ticker, &indicator, &timeframe, period, from, to).await
                        }, 
                        self.batch_config.cache_ttl).await
                }).await.map_err(|e| IndicatorError::TaskError(e.to_string()))
            }
            _ => Err(IndicatorError::FetchError("Invalid fetch type".to_string()))
        }
    }

    async fn validate_tickers(&self, tickers: Vec<String>) -> Result<Vec<String>, IndicatorError> {
        if tickers.len() > self.batch_config.batch_size {
            return Err(IndicatorError::TooManyTickersError(format!("Too many tickers: {}", tickers.len())));
        } else if tickers.is_empty() {
            return Err(IndicatorError::VoidTickersError("No tickers provided".to_string()));
        }
        Ok(tickers)
    }

    async fn ticker_level_concurrency(
        &self, 
        tickers: Vec<String>, 
        fetch_type: FetchType, 
        indicator: String, 
        timeframe: String, 
        period: u64, 
        from: Option<String>, 
        to: Option<String>
    ) -> Result<Value, IndicatorError> {
        let config = self.batch_config.clone();
        let tickers = self.validate_tickers(tickers).await?;
        let semaphore = std::sync::Arc::new(Semaphore::new(config.concurrency_limit));
        let mut tasks = Vec::new();

        for ticker in tickers {
            let semaphore = semaphore.clone();
            let fetch_type = fetch_type.clone();
            let (tickker, indicator, timeframe, period) = (ticker.clone(), indicator.clone(), timeframe.clone(), period.clone());
            let (from, to) = clone_str_options((&from, &to));
            let self_clone = self.clone();

            tasks.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();    
                self_clone.get_indicators_for_ticker(
                    &ticker, 
                    fetch_type, 
                    indicator, 
                    timeframe, 
                    period, 
                    from.map(|s| s), 
                    to.map(|s| s)).await
            }));
        }
        let mut results = Vec::new();    
        for task in tasks {
            let result = task.await.unwrap();
            match result {
                Ok(value) => results.push(value),
                Err(e) => results.push(Value::Null),
            };
        }
        Ok(Value::Array(results))
    }

    async fn batch_level_concurrency(
        &self, 
        tickers: Vec<String>, 
        fetch_type: FetchType, 
        indicator: String, 
        timeframe: String, 
        period: u64, 
        from: Option<String>, 
        to: Option<String>
    ) -> Result<Value, IndicatorError> {
        let config = self.batch_config.clone();
        let tickers = self.validate_tickers(tickers).await?;
        let semaphore = std::sync::Arc::new(Semaphore::new(config.concurrency_limit));
        
        let mut tasks = Vec::new();

        for chunk in tickers.chunks(config.batch_size) {

            let batch = chunk.to_vec();
    
            let (semaphore, fetch_type, indicator, timeframe, period) = (
                semaphore.clone(), fetch_type.clone(), indicator.clone(), timeframe.clone(), period.clone());     
            let (from, to) = clone_str_options((&from, &to));
            let self_clone = self.clone();

            tasks.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                self_clone.ticker_level_concurrency(batch, fetch_type, indicator, timeframe, period, from, to).await
            }));
        }

        let mut results = Vec::new();    
        for task in tasks {
            let result = task.await.unwrap();
            match result {
                Ok(result) => results.push(result),
                Err(e) => results.push(Value::Null),
            };
        }
        Ok(Value::Array(results))

    }
    pub async fn poll(&self, args: &Value) -> Result<Value, IndicatorError> {
        let tickers = args.get("tickers")
            .and_then(Value::as_array)
            .ok_or(IndicatorError::ParseError("Missing 'tickers' field".to_string()))?
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect::<Vec<String>>();

        let fetch_type = args.get("fetch_type")
            .and_then(Value::as_str)
            .map(FetchType::from_str)
            .ok_or(IndicatorError::ParseError("Missing 'fetch_type' field".to_string()))?;


        let indicator = args.get("indicator")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(IndicatorError::ParseError("Missing 'indicator' field".to_string()))?;

        let period = args.get("period")
            .and_then(Value::as_u64)
            .ok_or(IndicatorError::ParseError("Missing 'period' field".to_string()))?;

        let timeframe = args.get("timeframe")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(IndicatorError::ParseError("Missing 'timeframe' field".to_string()))?;

        let from = args.get("from").and_then(Value::as_str).map(String::from);
        let to = args.get("to").and_then(Value::as_str).map(String::from);
        
        self.batch_level_concurrency(tickers, fetch_type, indicator, timeframe, period, from, to).await
    }
        
            
}