#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;
use thiserror::Error;
use serde::{Deserialize, Serialize};

use crate::utils::retry;
use crate::cache::{Cache, SharedLockedCache};
use crate::config::{BatchConfig, RetryConfig};
use crate::request::HTTPClient;


const SEARCH_PATH: &str = "search";

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum SearchError {
    #[error("Failed to fetch data: {0}")]
    FetchError(String),
}


/// Functions for searching financial instruments in the FMP API.
pub struct Search {
    http_client: Arc<HTTPClient>,
    cache: Arc<Mutex<SharedLockedCache>>,
    batch_config: Arc<BatchConfig>,
    retry_config: Arc<RetryConfig>,
}

impl Search {

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
        
    async fn find_with_args(
        &self,
        keywords: &str,
        limit: Option<u64>,
        exchange: Option<String>,
    ) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "query": keywords,
            "limit": limit,
            "exchange": exchange
        });

        let query_params = self.http_client.build_query_from_value(query_params);

        let retry_cfg = self.retry_config.clone();

        retry(&retry_cfg, || async {
            self.http_client.get(SEARCH_PATH, Some(query_params.clone())).await
        }).await
    }

    pub async fn find(&self, args: &Value) -> Result<Value, SearchError> {
        let kwd = args.get("keyword")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or(SearchError::FetchError("Missing 'keyword' field".to_string()))?;

        let limit = args.get("limit").and_then(Value::as_u64);
        let exchange = args.get("exchange").and_then(Value::as_str).map(String::from);

        self.find_with_args(&kwd, limit, exchange)
            .await
            .map_err(|err| SearchError::FetchError(err.to_string()))
    }
}
