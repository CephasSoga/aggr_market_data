#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use crate::request::{make_request, generate_json};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Functions for accessing ETF-related data from the FMP API
pub struct Etf;

impl Etf {
    pub async fn list() -> Result<Value, reqwest::Error> {
        make_request("symbol/available-etfs", HashMap::new()).await
    }

    pub async fn quote(symbol: Option<&str>) -> Result<Value, reqwest::Error> {
        match symbol {
            Some(s) => make_request("quote", generate_json(Value::String(s.to_string()), None)).await,
            None => make_request("quotes/etf", HashMap::new()).await,
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
            "historical-price-full/etf",
            generate_json(Value::String(symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn dividend_history(
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
            "historical-price-full/stock_dividend",
            generate_json(Value::String(symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn split_history(
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
            "historical-price-full/stock_split",
            generate_json(Value::String(symbol.to_string()), Some(query_params))
        ).await
    }
}