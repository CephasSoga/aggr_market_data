#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::collections::HashMap;

use crate::request::{make_request, generate_json};
use crate::financial::Financial;
use clap::builder::Str;
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
pub enum IndicatorType {
    sma,
    ema,
    wma,
    dema,
    tema,
    williams,
    rsi,
    adx,
    standardDeviation,
}
impl IndicatorType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::sma => "sma",
            Self::ema => "ema",
            Self::wma => "wma",
            Self::dema => "dema",
            Self::tema => "tema",
            Self::williams => "williams",
            Self::rsi => "rsi",
            Self::adx => "adx",
            Self::standardDeviation => "standardDeviation",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Timeframe {
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
    TwoHours,
    FourHours,
    SixHours,
    TwelveHours,
    OneDay,
    ThreeDays,
    OneWeek,
    OneMonth,
}
impl Timeframe {
    fn as_str(&self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::FiveMinutes => "5m",
            Self::FifteenMinutes => "15m",
            Self::ThirtyMinutes => "30m",
            Self::OneHour => "1h",
            Self::TwoHours => "2h",
            Self::FourHours => "4h",
            Self::SixHours => "6h",
            Self::TwelveHours => "12h",
            Self::OneDay => "1d",
            Self::ThreeDays => "3d",
            Self::OneWeek => "1w",
            Self::OneMonth => "1M",
        }
    }
}

#[derive(Debug, Error)]
pub enum IndicatorError {
    #[error("Failed to fetch data: {0}")]
    FetchError(String),
    
    #[error("Task encountered an error: {0}")]
    TaskError(String),
    
    #[error("Failed to parse data: {0}")]
    ParseError(String),
    
    #[error("No tickers provided: {0}")]
    VoidTickersError(String),
}

pub struct TechnicalIndicatorPolling {
    cache: Arc<Mutex<LruCache<String, (Value, Instant)>>>,
    batch_config: Arc<BatchConfig>,
}
impl TechnicalIndicatorPolling {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(LruCache::new(std::num::NonZeroUsize::new(100).unwrap()))),
            batch_config: Arc::new(BatchConfig::default()),
        }
    }

    pub async fn get_from_cache_or_fetch<F: Future<Output = Result<Value, IndicatorError>>>(
        &self, 
        key: &str, 
        fetch_fn: F, 
        ttl: Duration
    ) -> Result<Value, IndicatorError> where F: Future<Output = Result<Value, IndicatorError>> {
        let mut cache = self.cache.lock().await;
        if let Some((value, instant)) = cache.get(key) {
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
            Err(e) => Err(IndicatorError::FetchError(e.to_string())),
        }
    }

    async fn get_indicator_value(&self, ticker: &str, indicator: &IndicatorType, timeframe: &Timeframe, period: Option<u32>) -> Result<Value, IndicatorError> {
        let partial_path = format!("technical_indicator/{}/{}", timeframe.as_str(), ticker);

        let mut query_param: HashMap<String, _> = HashMap::new();
        query_param.insert(
            "type".to_string(), Value::String(indicator.as_str().to_string())
        );
        query_param.insert(
            "period".to_string(), Value::Number(serde_json::Number::from(period.unwrap_or(10)))
        );

        make_request(partial_path.as_str(), query_param)
            .await
            .map_err(|e| IndicatorError::FetchError(e.to_string()))

    }

    pub async fn get_indicators_for_ticker(&self, ticker: &str, timeframe: &Timeframe) -> Result<Value, IndicatorError> {
        let indicators = vec![
            IndicatorType::sma,
            IndicatorType::ema,
            IndicatorType::wma,
            IndicatorType::dema,
            IndicatorType::tema,
            IndicatorType::williams,
            IndicatorType::rsi,
            IndicatorType::adx,
            IndicatorType::standardDeviation,
        ];

        let mut result = json!({});
        for indicator in indicators {
            let key = format!("{}_{}_{}", ticker, indicator.as_str(), timeframe.as_str());
            let indicator_value = self.get_from_cache_or_fetch(&key, self.get_indicator_value(ticker, &indicator, &timeframe, None), Duration::from_secs(60)).await?;
            result[indicator.as_str()] = indicator_value;
        }

        Ok(result)
    }

    async fn validate_tickers(tickers: Option<Vec<String>>) -> Result<Vec<String>, IndicatorError> {
        tickers.map_or(Err(IndicatorError::VoidTickersError("No tickers provided".to_string())), |t| {
            if t.is_empty() {
                Err(IndicatorError::VoidTickersError("No tickers provided".to_string()))
            } else {
                Ok(t)
            }
        })
    }

    async fn process_batch(
        &self, 
        batch: Vec<String>,
        timeframe: &Timeframe,
        config: &BatchConfig
    ) -> Result<(), IndicatorError> {
        let start = Instant::now();
        let mut rate_limiter = tokio::time::interval(Duration::from_secs_f32(
            1.0 / config.rate_limit_per_second as f32
        ));

        for symbol in batch {
            rate_limiter.tick().await;
            let mut attempts = 0;
            while attempts < config.retry_attempts {
                match self.get_indicators_for_ticker(&symbol, timeframe).await {
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

    pub async fn poll(&self, tickers: Option<Vec<String>>, timeframe: Option<Timeframe>) -> Result<(), IndicatorError> {
        let config = self.batch_config.clone();
        let symbols = Self::validate_tickers(tickers).await?;
        let semaphore = Arc::new(Semaphore::new(config.concurrency_limit));
        let timeframe = timeframe.unwrap_or(Timeframe::OneDay);

        let mut tasks = vec![];

        for chunk in symbols.chunks(config.batch_size) {
            let batch = chunk.to_vec();
            let semaphore_clone = semaphore.clone();
            let config_clone = config.clone();
            let self_clone = Self::new();
            let timeframe = timeframe.clone();

            let task = tokio::spawn(async move {
                let _permit = semaphore_clone.acquire().await
                    .map_err(|e| IndicatorError::TaskError(e.to_string()))?;

                self_clone.process_batch(batch, &timeframe, &config_clone).await
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