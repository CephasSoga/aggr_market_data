#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]


use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use futures_util::Future;
use metrics::{counter, gauge};
use tracing::{info, error};
use tokio::sync::Semaphore;
use crate::auth_config::BatchConfig;
use serde_json::Value;
use crate::request::make_request;

#[derive(Debug)]
pub enum MarketError {
    FetchError(String),
    TaskError(String),
    CacheError(String),
}

pub struct MarketPolling {
    cache: Arc<RwLock<HashMap<String, (Value, Instant)>>>,
    batch_config: BatchConfig,
}

impl MarketPolling {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            batch_config: BatchConfig::default(),
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
            Err(err) => Err(MarketError::FetchError(err.to_string())),
        }
    }

    async fn fetch_market_data(&self, endpoint: &str) -> Result<Value, MarketError> {
        let cache_key = format!("market_{}", endpoint);
        
        self.get_from_cache_or_fetch(
            &cache_key,
            || async {
                make_request(endpoint, HashMap::new()).await
            },
            Duration::from_secs(60), // 1 minute cache
        ).await
    }

    pub async fn most_active(&self) -> Result<Value, MarketError> {
        self.fetch_market_data("most_active").await
    }

    pub async fn most_gainer(&self) -> Result<Value, MarketError> {
        self.fetch_market_data("most_gainer").await
    }

    pub async fn most_loser(&self) -> Result<Value, MarketError> {
        self.fetch_market_data("most_loser").await
    }

    pub async fn sector_performance(&self) -> Result<Value, MarketError> {
        self.fetch_market_data("sector-performance").await
    }

    async fn process_batch(&self, endpoints: Vec<&str>, config: &BatchConfig) -> Result<(), MarketError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / config.rate_limit_per_second as f32
        ));

        for endpoint in endpoints {
            rate_limiter.tick().await;
            let mut attempts = 0;
            while attempts < config.retry_attempts {
                match self.fetch_market_data(endpoint).await {
                    Ok(_) => {
                        counter!("market.success").increment(1);
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        counter!("market.retries").increment(1);
                        if attempts == config.retry_attempts {
                            error!("Final retry failed for {}: {:?}", endpoint, e);
                            counter!("market.failures").increment(1);
                        } else {
                            tokio::time::sleep(Duration::from_millis(config.backoff_ms * attempts as u64)).await;
                        }
                    }
                }
            }
        }
        
        gauge!("market.batch_duration", "rate_limit" => format!("{}", start.elapsed().as_secs_f64()));
        Ok(())
    }

    pub async fn poll(&self) -> Result<(), MarketError> {
        let config = self.batch_config.clone();
        let endpoints = vec!["most_active", "most_gainer", "most_loser", "sector_performance"];
        
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        let tasks = endpoints.chunks(config.batch_size)
            .map(|chunk| {
                let batch = chunk.to_vec();
                let semaphore_clone = semaphore.clone();
                let config_clone = config.clone();
                let self_clone = Self::new();

                tokio::spawn(async move {
                    let _permit = semaphore_clone.acquire().await
                        .map_err(|e| MarketError::TaskError(e.to_string()))?;
                    self_clone.process_batch(batch, &config_clone).await
                })
            })
            .collect::<Vec<_>>();

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