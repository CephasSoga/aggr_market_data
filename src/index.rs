#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use crate::request::{make_request, generate_json};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Functions for accessing market index data from the FMP API.
pub struct Index;

impl Index {
    pub async fn list() -> Result<Value, reqwest::Error> {
        make_request("symbol/available-indexes", HashMap::new()).await
    }

    pub async fn quote(symbol: Option<&str>) -> Result<Value, reqwest::Error> {
        match symbol {
            Some(s) => make_request("quote", generate_json(Value::String(s.to_string()), None)).await,
            None => make_request("quotes/index", HashMap::new()).await,
        }
    }

    pub async fn history(
        symbol: &str,
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
            "historical-price-full/index",
            generate_json(Value::String(symbol.to_string()), Some(query_params))
        ).await
    }
}
