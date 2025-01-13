#![allow(dead_code)]
#![allow(warnings)]

use crate::request::HTTPClient;
use crate::utils::retry;
use crate::config::{RetryConfig, BatchConfig};
use crate::cache::SharedLockedCache;
use crate::options::{TimeFrame, DateTime, FetchType};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use futures_util::Future;
use thiserror::Error;
use tracing::{info, error};

const LIST_PATH: &str = "available-forex-currency-pairs";
const INTRADAY_PATH: &str = "history-chart";
const DAILY_PATH: &str = "historical-price-full";

#[derive(Debug, Deserialize, Serialize)]
pub struct Crypto {
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub stock_exchange: String,
    pub exchange_short_name: String,
}

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Failed to fetch data: {0}")]
    FetchError(String),
    #[error("Task error: {0}")]
    TaskError(String),
    #[error("Parsing error: {0}")]
    ParseError(String),
    #[error("No tickers provided")]
    VoidTickersError,
    #[error("Too many tickers: {0}")]
    TooManyTickersError(usize),
}

pub struct CryptoPolling {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}

impl CryptoPolling {
    pub fn new(
        http_client: Arc<HTTPClient>,
        cache: Arc<Mutex<SharedLockedCache>>,
        batch_config: Arc<BatchConfig>,
        retry_config: Arc<RetryConfig>,
    ) -> Self {
        Self {
            http_client,
            cache,
            batch_config,
            retry_config,
        }
    }

    pub async fn list(&self) -> Result<Value, reqwest::Error> {
        self.http_client.get(LIST_PATH, None).await
    }

    fn normalize_symbol(symbol: &str) -> String {
        let upper = symbol.to_uppercase();
        if !upper.ends_with("USD") {
            format!("{upper}USD")
        } else {
            upper
        }
    }

    async fn get_from_cache_or_fetch<F, Fut>(
        &self,
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) -> Result<Value, CryptoError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, reqwest::Error>>,
    {
        let mut cache = self.cache.lock().await;

        if let Some((value, timestamp)) = cache.get(key).await {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            }
            cache.pop(key);
        }

        fetch_fn().await.map_or_else(
            |err| Err(CryptoError::FetchError(err.to_string())),
            |value| {
                cache.put(key.to_string(), (value.clone(), Instant::now())).await;
                Ok(value)
            },
        )
    }

    pub async fn intraday(
        &self,
        symbol: &str,
        timeframe: &str,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, reqwest::Error> {
        let mut query_params = json!({});
        if let Some(f) = from {
            query_params["from"] = json!(DateTime::from_str(f).unwrap().to_string());
        }
        if let Some(t) = to {
            query_params["to"] = json!(DateTime::from_str(t).unwrap().to_string());
        }

        let path = self
            .http_client
            .join(vec![INTRADAY_PATH, timeframe, symbol]);
        self.http_client.get(&path, Some(self.http_client.build_query_from_value(query_params))).await
    }

    pub async fn daily(&self, symbol: &str) -> Result<Value, reqwest::Error> {
        let path = self.http_client.join(vec![DAILY_PATH, symbol]);
        self.http_client.get(&path, None).await
    }

    async fn validate_tickers(&self, tickers: &[String]) -> Result<Vec<String>, CryptoError> {
        match tickers.len() {
            0 => Err(CryptoError::VoidTickersError),
            len if len > self.batch_config.batch_size => Err(CryptoError::TooManyTickersError(len)),
            _ => Ok(tickers.to_vec()),
        }
    }

    async fn fetch_crypto_data(
        &self,
        symbol: &str,
        fetch_type: FetchType,
        timeframe: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, CryptoError> {
        match fetch_type {
            FetchType::IntraDay => {
                self.intraday(symbol, timeframe.unwrap_or("5min"), from, to)
                    .await
                    .map_err(|e| CryptoError::FetchError(e.to_string()))
            }
            FetchType::Daily => self.daily(symbol).await.map_err(|e| CryptoError::FetchError(e.to_string())),
            _ => Err(CryptoError::TaskError("Invalid fetch type".to_string())),
        }
    }

    pub async fn batch_level_concurrency(
        &self,
        tickers: Vec<String>,
        fetch_type: FetchType,
        timeframe: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Value, CryptoError> {
        let validated_tickers = self.validate_tickers(&tickers).await?;
        let semaphore = Arc::new(Semaphore::new(self.batch_config.concurrency_limit));
        let tasks: Vec<_> = validated_tickers
            .chunks(self.batch_config.batch_size)
            .map(|chunk| {
                let batch = chunk.to_vec();
                let sem = semaphore.clone();
                let retry_cfg = self.retry_config.clone();
                let this = self.clone();

                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    this.ticker_level_concurrency(batch, fetch_type, timeframe, from, to).await
                })
            })
            .collect();

        let results = futures_util::future::join_all(tasks).await;
        let values: Vec<_> = results
            .into_iter()
            .map(|res| res.unwrap_or_else(|_| json!({"error": "Task panicked"})))
            .collect();

        Ok(Value::Array(values))
    }

    async fn ticker_level_concurrency(
        &self,
        batch: Vec<String>,
        fetch_type: FetchType,
        timeframe: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Value {
        let tasks: Vec<_> = batch
            .into_iter()
            .map(|symbol| {
                let retry_cfg = self.retry_config.clone();
                let this = self.clone();
                tokio::spawn(async move {
                    retry(&retry_cfg, || async {
                        this.fetch_crypto_data(&symbol, fetch_type, timeframe, from, to)
                            .await
                    })
                    .await
                })
            })
            .collect();

        let results = futures_util::future::join_all(tasks).await;
        json!(results.into_iter().map(|res| res.unwrap_or(json!({"error": "Task panicked"}))).collect::<Vec<_>>())
    }

    fn clone(&self) -> Self {
        Self {
            http_client: Arc::clone(&self.http_client),
            cache: Arc::clone(&self.cache),
            batch_config: Arc::clone(&self.batch_config),
            retry_config: Arc::clone(&self.retry_config),
        }
    }
}
