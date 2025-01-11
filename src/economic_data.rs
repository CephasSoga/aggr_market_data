#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use crate::request::{make_request_2, generate_json};
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
use crate::config::{TimeConfig, BatchConfig};
use crate::utils::{now, ago_secs};



pub enum IndicatorType {
    GDP, 
    realGDP, 
    nominalPotentialGDP, 
    realGDPPerCapita, 
    federalFunds, 
    CPI, 
    inflationRate, 
    inflation, 
    retailSales, 
    consumerSentiment, 
    durableGoods, 
    unemploymentRate
}
impl IndicatorType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::GDP => "GDP",
            Self::realGDP => "realGDP",
            Self::nominalPotentialGDP => "nominalPotentialGDP",
            Self::realGDPPerCapita => "realGDPPerCapita",
            Self::federalFunds => "federalFunds",
            Self::CPI => "CPI",
            Self::inflationRate => "inflationRate",
            Self::inflation => "inflation",
            Self::retailSales => "retailSales",
            Self::consumerSentiment => "consumerSentiment",
            Self::durableGoods => "durableGoods",
            Self::unemploymentRate => "unemploymentRate"
        }
    }
}

pub enum EconomicData {
    Treasury,
    Indicator,
    RiskPremium,
}

pub struct EconomicDataPolling {
    cache: Arc<tokio::sync::RwLock<HashMap<String, (Value, Instant)>>>,
    batch_config: BatchConfig,
}
impl EconomicDataPolling {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            batch_config: BatchConfig::default(),
        }
    }
    async fn get_from_cache_or_fetch<F, Fut>(&self, key: &str, fetch_data: F, ttl: Duration)
    -> Result<Value, reqwest::Error>
    where  F : FnOnce() -> Fut,
           Fut : Future<Output = Result<Value, reqwest::Error>> {
        let mut cache = self.cache.write().await;
        if let Some((value, timestamp)) = cache.get(key) {
            if timestamp.elapsed() < ttl {
                return Ok(value.clone());
            }
            cache.remove(key);
        }
        let value = fetch_data().await?;
        cache.insert(key.to_string(), (value.clone(), Instant::now()));
        Ok(value)
        
    }

    pub async fn  get_treasury (&self, from: &str, to: &str) -> Result<Value, reqwest::Error> {
        let key = "treasury";
        let ttl = Duration::from_secs(60);

        let mut query_params = HashMap::new();
        query_params.insert("from".to_string(), json!(from));
        query_params.insert("to".to_string(), json!(to));

        self.get_from_cache_or_fetch(key, || async {
            let url = "treasury";
            
            make_request_2(url, query_params).await
        }, ttl).await
    }

    pub async fn get_indicator(&self, indicator: IndicatorType, from: &str, to: &str) -> Result<Value, reqwest::Error> {
        let key = format!("indicator_{}", indicator.as_str());
        let ttl = Duration::from_secs(60);

        let mut query_params = HashMap::new();
        query_params.insert("from".to_string(), json!(from));
        query_params.insert("to".to_string(), json!(to));

        self.get_from_cache_or_fetch(&key, || async {
            let url = "indicator";
            let mut query_params = HashMap::new();
            query_params.insert("indicator".to_string(), json!(indicator.as_str()));
            query_params.insert("from".to_string(), json!(from));
            query_params.insert("to".to_string(), json!(to));

            make_request_2(url, query_params).await
        }, ttl).await
    }
    
}