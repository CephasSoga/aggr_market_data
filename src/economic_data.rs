#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tracing_subscriber::fmt::format;
use std::collections::HashMap;
use tokio::sync::{Mutex, Semaphore};
use metrics::{counter, gauge};
use tracing::{info, error};
use serde::{Deserialize, Serialize};
use futures_util::Future;
use std::fmt::Display;
use thiserror::Error;

use crate::utils::retry;
use crate::request::HTTPClient;
use crate::config::{TimeConfig, BatchConfig, RetryConfig};
use crate::options::{FetchType, EconomicData, EconomicIndicatorType};
use crate::cache::{Cache, SharedLockedCache};


const INDICATOR_PATH_V4: &str = "economic";
const TREASURY_RATE_PATH_V4: &str = "treasury";
const MARKET_RISK_PREMIUM_PATH_V4: &str = "market_risk_premium";
const UPCOMING_MARKET_DATA_CALENDAR_PATH: &str = "economic_calendar";

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum EconomicDataError {
    #[error("Failed to fetch data: {0}")]
    FetchError(String),
    
    #[error("Task encountered an error: {0}")]
    TaskError(String),
    
    #[error("Failed to parse data: {0}")]
    ParseError(String),
    
    #[error("No economic data provided: {0}")]
    VoidEconomicDataError(String),
}

pub struct EconomicDataPolling {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}
impl EconomicDataPolling {
    pub fn new(
        http_client: Arc<HTTPClient>, 
        cache: Arc<Mutex<SharedLockedCache>>, 
        batch_config: Arc<BatchConfig>, 
        retry_config: Arc<RetryConfig>) -> Self {
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
            retry_config: self.retry_config.clone()
        }
    }

    async fn get_from_cache_or_fetch<F, Fut>(
        &self, 
        key: &str, 
        fetch_data: F, 
        ttl: Duration)-> Result<Value, reqwest::Error>
    where  F : FnOnce() -> Fut,
           Fut : Future<Output = Result<Value, reqwest::Error>> {
        let mut cache = self.cache.lock().await;
        if let Some((value, timestamp)) = cache.get(key).await {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            }
            cache.pop(key);
        }
        let value = fetch_data().await?;
        cache.put(key.to_string(), (value.clone(), Instant::now()));
        Ok(value)
        
    }

    async fn  treasury_rates(&self, from: Option<String>, to: Option<String>) -> Result<Value, reqwest::Error> {
       let query_params = json!({
        "from": from,
        "to": to
       });
       let query_params = self.http_client.build_query_from_value(query_params);
       self.http_client.get_v4(TREASURY_RATE_PATH_V4, Some(query_params)).await
    }

    async fn indicator_value(&self, indicator: &EconomicIndicatorType) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "name": indicator.as_str()
        });
        let query_params = self.http_client.build_query_from_value(query_params);

        self.http_client.get_v4(INDICATOR_PATH_V4, Some(query_params)).await
    }

    async fn risk_premium(&self) -> Result<Value, reqwest::Error> {
        self.http_client.get_v4(MARKET_RISK_PREMIUM_PATH_V4, None).await
    }

    async fn calendar(&self, from: Option<String>, to: Option<String>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "from": from,
            "to": to
        });
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(UPCOMING_MARKET_DATA_CALENDAR_PATH, Some(query_params)).await
    }

    async fn fetch_market_data(
        &self,
        fetch_type: FetchType, 
        indicator:Option<EconomicIndicatorType>,
        from: Option<String>,
        to: Option<String>,
    ) -> Result<Value, EconomicDataError
    > {
        match fetch_type {
            FetchType::MarketIndicator => {
                match indicator {
                    Some(indicator_type) => {
                        let key = format!("{}", indicator_type);
                        let retry_cfg = self.retry_config.clone();
                        retry(
                            &retry_cfg, 
                            || async {
                                self.get_from_cache_or_fetch(
                                    &key, 
                                    || async {
                                        self.indicator_value(&indicator_type).await
                                    }, 
                                    self.batch_config.cache_ttl).await
                            }).await
                            .map_err(|err| EconomicDataError::FetchError(err.to_string()))
                    },
                    None => return Err(EconomicDataError::FetchError("Indicator type is required by this task but is  not provided".to_string())),
                }
            }
            FetchType::TreasuryRate => {
                let key = format!("tresaury_rates-{}-{}", &from.clone().unwrap_or("".to_string()), &to.clone().unwrap_or("".to_string()));
                let retry_cfg = self.retry_config.clone();
                retry(
                    &retry_cfg, 
                    || async {
                        let (from, to) = (from.clone(), to.clone());
                        self.get_from_cache_or_fetch(
                            &key, 
                            || async {
                                self.treasury_rates(from, to).await
                            }, 
                            self.batch_config.cache_ttl).await
                    }).await
                    .map_err(|err| EconomicDataError::FetchError(err.to_string()))
            }
            FetchType::MarketRiskPremium => {
               let key = "market_risk_premium";
                let retry_cfg = self.retry_config.clone();
                retry(
                    &retry_cfg, 
                    || async {
                        self.get_from_cache_or_fetch(
                            &key, 
                            || async {
                                self.risk_premium().await
                            }, 
                            self.batch_config.cache_ttl).await
                    }).await
                    .map_err(|err| EconomicDataError::FetchError(err.to_string()))
            }

            FetchType::MarketCalendar => {
                let key = format!("calendar-{}-{}", &from.clone().unwrap_or("".to_string()), &to.clone().unwrap_or("".to_string()));
                let retry_cfg = self.retry_config.clone();
                retry(
                    &retry_cfg, 
                    || async {
                        self.get_from_cache_or_fetch(
                            &key, 
                            || async {
                                let (from, to) = (from.clone(), to.clone());
                                self.calendar(from, to).await
                            }, 
                            self.batch_config.cache_ttl).await
                    }).await
                    .map_err(|err| EconomicDataError::FetchError(err.to_string()))
            }

            _ => return  Err(EconomicDataError::TaskError(format!("Invalid Task: {}", fetch_type)))
        }
    }


    pub async fn poll(&self, args: &Value) -> Result<Value, EconomicDataError> {
        let fetch_type = args.get("fetch_type")
            .and_then(Value::as_str)
            .map(FetchType::from_str)
            .ok_or(EconomicDataError::ParseError("Fetch type is not provided".to_string()))?;
        
        let indicator = args.get("indicator")
            .and_then(Value::as_str)
            .and_then(|s| EconomicIndicatorType::from_str(s));

        let from = args.get("from")
            .and_then(Value::as_str)
            .map(String::from);
        let to = args.get("to")
            .and_then(Value::as_str)
            .map(String::from);

        self.fetch_market_data(fetch_type, indicator, from, to).await
    }
    
}