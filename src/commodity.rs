use crate::request::{make_request, generate_json};
use tokio::sync::Semaphore;
use serde_json::{json, Value};
use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use metrics::{counter, gauge};
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[derive(Debug)]
pub enum CommodityError {
    FetchError(String),
    TaskError(String),
    ParseError(String),
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub concurrency_limit: usize,
    pub batch_size: usize,
    pub retry_attempts: u32,
    pub backoff_ms: u64,
    pub rate_limit_per_second: u32,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            concurrency_limit: 200,  // Increased from 100
            batch_size: 100,         // Increased from 50
            retry_attempts: 3,
            backoff_ms: 1000,
            rate_limit_per_second: 50,
        }
    }
}
#[derive(Debug, Deserialize, Serialize)]
struct Commodity {
    symbol: String,
    name: String,
    currency: String,
    stock_exchange: String,
    exchange_short_name: String,
}

pub struct CommodityPolling;

impl CommodityPolling {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn list() -> Result<Value, reqwest::Error> {
        make_request("symbol/available-commodities", HashMap::new()).await
    }

    pub async fn quote(symbol: Option<String>) -> Result<Value, reqwest::Error> {
        match symbol {
            Some(s) => make_request("quote", generate_json(Value::String(s), None)).await,
            None => make_request("quotes/commodity", HashMap::new()).await,
        }
    }

    pub async fn history(
        symbol: String,
        start_date: Option<&str>,
        end_date: Option<&str>,
        data_type: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "from": start_date,
            "to": end_date,
            "serietype": data_type,
            "timeseries": limit
        });

        make_request(
            "historical-price-full/commodity",
            generate_json(Value::String(symbol), Some(query_params))
        ).await
    }

    async fn process_batch(batch: Vec<String>, config: &BatchConfig) -> Result<(), CommodityError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / config.rate_limit_per_second as f32
        ));

        for symbol in batch {
            rate_limiter.tick().await;

            let mut attempts = 0;
            while attempts < config.retry_attempts {
                match Self::fetch_symbol_data(&symbol).await {
                    Ok(_) => {
                        counter!("commodity.success").increment(1);
                        break;
                    }
                    Err(e) => {
                        attempts += 1;
                        counter!("commodity.retries").increment(1);
                        if attempts == config.retry_attempts {
                            error!("Final retry failed for {}: {:?}", symbol, e);
                            counter!("commodity.failures").increment(1);
                        } else {
                            sleep(Duration::from_millis(config.backoff_ms * attempts as u64)).await;
                        }
                    }
                }
            }
        }

        gauge!("commodity.batch_duration", "rate limit" => format!("{}", start.elapsed().as_secs_f64()));
        Ok(())
    }

    async fn fetch_symbol_data(symbol: &str) -> Result<(), CommodityError> {
        let (quote, history) = tokio::join!(
            Self::quote(Some(symbol.to_string())),
            Self::history(symbol.to_string(), None, None, None, None)
        );

        quote.map_err(|e| CommodityError::FetchError(e.to_string()))?;
        history.map_err(|e| CommodityError::FetchError(e.to_string()))?;
        Ok(())
    }

/// Polls available commodities, processing them in batches with concurrency control.
/// 
/// ## Arguments
///
/// * `config` - Optional batch configuration for polling. If None, default configuration is used.
///
/// ## Returns
///
/// A Result indicating success or a `CommodityError`.
///
/// ## Behavior
///
/// The function retrieves a list of commodities, splits them into batches, and processes each batch asynchronously.
/// A semaphore is used to limit concurrent processing. The function handles errors at different stages, including
/// fetching the list of commodities, parsing them, and processing each batch. Errors are logged, and retries are
/// attempted based on the provided configuration.

    pub async fn poll(config: Option<BatchConfig>) -> Result<(), CommodityError> {
        let config = config.unwrap_or_default();

        let commodities = Self::list()
            .await
            .map_err(|e| CommodityError::FetchError(e.to_string()))?
            .as_array()
            .ok_or_else(|| CommodityError::ParseError("Invalid response format".to_string()))?
            .iter()
            .map(|c| serde_json::from_value(c.clone()))
            .collect::<Result<Vec<Commodity>, _>>()
            .map_err(|e| CommodityError::ParseError(e.to_string()))?;

        let symbols: Vec<String> = commodities.iter().map(|c| c.symbol.clone()).collect();
        let semaphore = std::sync::Arc::new(Semaphore::new(config.concurrency_limit));
        let mut tasks = vec![];

        for chunk in symbols.chunks(config.clone().batch_size) {
            let batch = chunk.to_vec();
            let semaphore_clone = semaphore.clone();

            let config_clone = config.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore_clone.acquire().await.map_err(|e| CommodityError::TaskError(e.to_string()))?;
                Self::process_batch(batch, &config_clone).await
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
