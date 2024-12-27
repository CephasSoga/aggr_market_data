#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]


use crate::request::{make_request, generate_json};
use serde_json::{json, Value};
use std::collections::HashMap;
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
use std::sync::Mutex;

use crate::auth_config::BatchConfig;



#[derive(Debug, Deserialize, Serialize)]
pub struct Crypto {
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub stockExchange: String,
    pub exchangeShortName: String,
}

/// Helper enum to handle single value or array of values.
pub enum Either<L, R> {
    Single(L),
    Array(R),
}

#[derive(Debug)]
pub enum CryptoError {
    FetchError(String),
    TaskError(String),
    ParseError(String),
    VoidTickersError(String),
}
impl Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}


/// Functions for accessing cryptocurrency-related data from the FMP API
pub struct CryptoPolling {
    cache: Arc<tokio::sync::RwLock<HashMap<String, (Value, Instant)>>>,
    batch_config: BatchConfig,
}

impl CryptoPolling {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            batch_config: BatchConfig::default(),
        }
    }

    pub async fn list() -> Result<Value, reqwest::Error> {
        make_request("symbol/available-cryptocurrencies", HashMap::new()).await
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
        let mut cache = self.cache.write().await;

        // Check cache
        if let Some((value, timestamp)) = cache.get(key) {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            } else {
                cache.remove(key); // Expired
            }
        }

        // Fetch and cache
        match fetch_fn().await {
            Ok(value) => {
                cache.insert(key.to_string(), (value.clone(), Instant::now()));
                Ok(value)
            }
            Err(err) => Err(CryptoError::FetchError(err.to_string())),
        }
    }

    fn normalize_symbol(symbol: &str) -> String {
        let symbol = symbol.to_uppercase();
        if !symbol.to_lowercase().contains("usd") {
            format!("{}USD", symbol)
        } else {
            symbol
        }
    }

    pub async fn quote(
        &self,
        symbols: Option<Either<&str, &[&str]>>,
        config: &BatchConfig,
    ) -> Result<Value, CryptoError> {
        match symbols {
            Some(Either::Single(symbol)) => {
                let normalized = Self::normalize_symbol(symbol);
                let fetch_fn = || async {
                    make_request("quote", generate_json(Value::String(normalized.clone()), None)).await
                };
                self.get_from_cache_or_fetch(&normalized, fetch_fn, config.cache_ttl).await
            }
            Some(Either::Array(symbols)) => {
                let normalized: Vec<String> = symbols.iter().map(|s| Self::normalize_symbol(s)).collect();
                let mut results = Vec::new();
    
                for symbol in &normalized {
                    let fetch_fn = || async {
                        make_request("quote", generate_json(Value::String(symbol.clone()), None)).await
                    };
                    let result = self.get_from_cache_or_fetch(symbol, fetch_fn, config.cache_ttl).await?;
                    results.push(result);
                }
    
                Ok(Value::Array(results))
            }
            None => make_request("cryptocurrencies", HashMap::new()).await.map_err(|e| CryptoError::FetchError(e.to_string())),
        }
    }
    
    pub async fn history(
        &self,
        symbols: Either<&str, &[&str]>,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
        config: &BatchConfig,
    ) -> Result<Value, CryptoError> {
        let normalized = match symbols {
            Either::Single(symbol) => Self::normalize_symbol(symbol),
            Either::Array(symbols) => symbols.iter().map(|s| Self::normalize_symbol(s)).collect::<Vec<_>>().join(","),
        };
    
        let cache_key = format!(
            "history:{}:{}:{}:{}:{}",
            normalized, start_date.unwrap_or(""), end_date.unwrap_or(""), data_type.unwrap_or(""), limit.unwrap_or(0)
        );
    
        let fetch_fn = || async {
            let query_params = json!({
                "from": start_date,
                "to": end_date,
                "serietype": data_type,
                "timeseries": limit
            });
    
            make_request(
                "historical-price-full/crypto",
                generate_json(Value::String(normalized.clone()), Some(query_params)),
            )
            .await
        };
    
        self.get_from_cache_or_fetch(&cache_key, fetch_fn, config.cache_ttl).await
    }
    
    async fn process_batch(&self, batch: Vec<String>, config: &BatchConfig) -> Result<(), CryptoError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / config.rate_limit_per_second as f32
        ));

        for symbol in batch {
            rate_limiter.tick().await;
            let mut attempts = 0;
            while attempts < config.retry_attempts {
                match self.fetch_crypto_data(&symbol, config).await {
                    Ok(_) => {
                        counter!("crypto.success").increment(1);
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        counter!("crypto.retries").increment(1);
                        if attempts == config.retry_attempts {
                            error!("Final retry failed for {}: {:?}", symbol, e);
                            counter!("crypto.failures").increment(1);
                        } else {
                            tokio::time::sleep(Duration::from_millis(config.backoff_ms * attempts as u64)).await;
                        }
                    }
                }
            }
        }
        
        gauge!("crypto.batch_duration", "rate limit" => format!("{}", start.elapsed().as_secs_f64()));
        Ok(())
    }

    async fn fetch_crypto_data(&self, symbol: &str, config: &BatchConfig) -> Result<(), CryptoError> {
        let (quote, history) = tokio::join!(
            self.quote(Some(Either::Single(symbol)), config),
            self.history(Either::Single(symbol), None, None, None, None, config)
        );

        quote.map_err(|e| CryptoError::FetchError(e.to_string()))?;
        history.map_err(|e| CryptoError::FetchError(e.to_string()))?;
        Ok(())
    }

    async fn validate_tickers(tickers: Option<Vec<String>>) -> Result<Vec<String>, CryptoError> {
        let cryptos = Self::list()
            .await
            .map_err(|e| CryptoError::FetchError(e.to_string()))?
            .as_array()
            .ok_or_else(|| CryptoError::ParseError("Invalid response format".to_string()))?
            .iter()
            .map(|c| serde_json::from_value(c.clone()))
            .collect::<Result<Vec<Crypto>, _>>()
            .map_err(|e| CryptoError::ParseError(e.to_string()))?;

        let available_symbols: HashSet<_> = cryptos.iter().map(|c| c.symbol.clone()).collect();
        match tickers {
            Some(input_symbols) => {
                let invalid_symbols: Vec<_> = input_symbols
                    .iter()
                    .filter(|s| !available_symbols.contains(*s))
                    .cloned()
                    .collect();
                
                if invalid_symbols.is_empty() {
                    Ok(input_symbols)
                } else {
                    Err(CryptoError::ParseError(format!(
                        "Invalid ticker symbols: {:?}",
                        invalid_symbols
                    )))
                }
            }
            None => Err(CryptoError::VoidTickersError("No tickers provided".to_string())),
        }
    }

    pub async fn poll(&self, tickers: Option<Vec<String>>) -> Result<(), CryptoError> {
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
                    .map_err(|e| CryptoError::TaskError(e.to_string()))?;
                self_clone.process_batch(batch, &config_clone).await
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




