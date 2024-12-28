#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use crate::request::{make_request, generate_json};
use serde_json::{json, Value};

/// Functions for searching financial instruments in the FMP API.
pub struct Search;

impl Search {
    pub async fn query(
        keywords: &str,
        limit: Option<i32>,
        exchange: Option<&str>,
    ) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "query": keywords,
            "limit": limit,
            "exchange": exchange
        });

        make_request(
            "search",
            generate_json(Value::Null, Some(query_params))
        ).await
    }
}


pub async fn example() -> Result<(), reqwest::Error> {
    let results = Search::query("apple", Some(10), Some("NASDAQ")).await?;

    let all_results = Search::query("tesla", None, None).await?;
    
    Ok(())
}
