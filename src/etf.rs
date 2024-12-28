#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]
#![allow(unused_imports)]

use crate::request::{make_request, generate_json};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use metrics::{counter, gauge};
use tracing::{info, error};
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use futures_util::Future;
use std::fmt::Display;
use crate::auth_config::{TimeConfig, BatchConfig};
use crate::utils::{now, ago_secs};


#[derive(Debug)]
pub enum EtfError {
    FetchError(String),
    TaskError(String),
    ParseError(String),
    VoidTickersError(String),
}

impl Display for EtfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Etf {
    symbol: String,
    name: String,
    currency: String,
    stock_exchange: String,
    exchange_short_name: String,
}

pub struct EtfPolling {
    cache: Arc<tokio::sync::RwLock<HashMap<String, (Value, Instant)>>>,
    batch_config: BatchConfig,
}

impl EtfPolling {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            batch_config: BatchConfig::default(),
        }
    }

    async fn get_from_cache_or_fetch<F, Fut>(
        &self,
        key: &str,
        fetch_fn: F,
        ttl: Duration,
    ) -> Result<Value, EtfError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, reqwest::Error>>,
    {
        let mut cache = self.cache.write().await;
        if let Some((value, timestamp)) = cache.get(key) {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            }
            cache.remove(key);
        }

        match fetch_fn().await {
            Ok(value) => {
                cache.insert(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            }
            Err(err) => Err(EtfError::FetchError(err.to_string())),
        }
    }

    pub async fn history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, EtfError> {
        let cache_key = format!("history_{}_{:?}_{:?}_{:?}_{:?}", 
            symbol, start_date, end_date, data_type, limit);
        
        self.get_from_cache_or_fetch(
            &cache_key,
            || async {
                let query_params = json!({
                    "from": start_date,
                    "to": end_date,
                    "serietype": data_type,
                    "timeseries": limit
                });
                
                make_request(
                    "historical-price-full/etf",
                    generate_json(Value::String(symbol.to_string()), Some(query_params))
                ).await
            },
            Duration::from_secs(300), // 5 minute cache
        ).await
    }

    pub async fn dividend_history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, EtfError> {
        let cache_key = format!("dividend_{}_{:?}_{:?}_{:?}_{:?}", 
            symbol, start_date, end_date, data_type, limit);

        self.get_from_cache_or_fetch(
            &cache_key,
            || async {
                let query_params = json!({
                    "from": start_date,
                    "to": end_date,
                    "serietype": data_type,
                    "timeseries": limit
                });
                
                make_request(
                    "historical-price-full/etf/dividend",
                    generate_json(Value::String(symbol.to_string()), Some(query_params))
                ).await
            },
            Duration::from_secs(3600), // 1 hour cache
        ).await
    }

    pub async fn split_history(
        &self,
        symbol: &str,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, EtfError> {
        let cache_key = format!("split_{}_{:?}_{:?}_{:?}_{:?}", 
            symbol, start_date, end_date, data_type, limit);

        self.get_from_cache_or_fetch(
            &cache_key,
            || async {
                let query_params = json!({
                    "from": start_date,
                    "to": end_date,
                    "serietype": data_type,
                    "timeseries": limit
                });
                
                make_request(
                    "historical-price-full/etf/split",
                    generate_json(Value::String(symbol.to_string()), Some(query_params))
                ).await
            },
            Duration::from_secs(3600), // 1 hour cache
        ).await
    }

    async fn process_batch(
        &self, 
        batch: Vec<String>,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
        config: &BatchConfig
    ) -> Result<(), EtfError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / config.rate_limit_per_second as f32
        ));

        for symbol in batch {
            rate_limiter.tick().await;
            let mut attempts = 0;
            while attempts < config.retry_attempts {
                match self.fetch_etf_data(&symbol, start_date, end_date, data_type, limit).await {
                    Ok(_) => {
                        counter!("etf.success").increment(1);
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        counter!("etf.retries").increment(1);
                        if attempts == config.retry_attempts {
                            error!("Final retry failed for {}: {:?}", symbol, e);
                            counter!("etf.failures").increment(1);
                        } else {
                            tokio::time::sleep(Duration::from_millis(config.backoff_ms * attempts as u64)).await;
                        }
                    }
                }
            }
        }
        
        gauge!("etf.batch_time", "rate limit" => format!("{}", start.elapsed().as_secs_f64()));
        Ok(())
    }

    async fn fetch_etf_data(
        &self, 
        symbol: &str, 
        start_date: Option<&str>, 
        end_date: Option<&str>, 
        data_type: Option<&str>, 
        limit: Option<i32>, 
    ) -> Result<(), EtfError> {
        let (history, dividend, split) = tokio::join!(
            self.history(symbol, start_date, end_date, data_type, limit),
            self.dividend_history(symbol, start_date, end_date, data_type, limit),
            self.split_history(symbol, start_date, end_date, data_type, limit),
        );
        split.map_err(  |e| EtfError::FetchError(e.to_string()))?;
        dividend.map_err(|e| EtfError::FetchError(e.to_string()))?;
        history.map_err(|e| EtfError::FetchError(e.to_string()))?;
        Ok(())
    }

    async fn validate_tickers(tickers: Option<Vec<String>>) -> Result<Vec<String>, EtfError> {
        tickers.map_or(Err(EtfError::VoidTickersError("No tickers provided".to_string())), |t| {
            if t.is_empty() {
                Err(EtfError::VoidTickersError("No tickers provided".to_string()))
            } else {
                Ok(t)
            }
        })
    }

    pub async fn poll(&self, tickers: Option<Vec<String>>) -> Result<(), EtfError> {
        let config = self.batch_config.clone();
        let symbols = Self::validate_tickers(tickers).await?;
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        let mut tasks = vec![];

        for chunk in symbols.chunks(config.batch_size) {
            let batch = chunk.to_vec();
            let semaphore_clone = semaphore.clone();
            let config_clone = config.clone();
            let self_clone = Self::new();

            let task = tokio::spawn(async move {
                let _permit = semaphore_clone.acquire().await
                    .map_err(|e| EtfError::TaskError(e.to_string()))?;

                let time_config = TimeConfig::default();
                let end_date = now();
                let start_date = ago_secs(time_config.days_back);
                self_clone.process_batch(batch, Some(&start_date), Some(&end_date), None, None, &config_clone).await
            });
            tasks.push(task);
        }

        for task in tasks {
            match task.await {
                Ok(result) => if let Err(e) = result {
                    error!("Batch processing error: {:?}", e);
                },
                Err(e) => error!("Task failure: {:?}", e),
            }
        }

        Ok(())
    }
}