#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::collections::HashMap;

use crate::request::{make_request, generate_json};
use serde_json::{json, Value};

/// Functions for accessing financial statement data from the FMP API
pub struct Financial<'a> {
    symbol: &'a str,
}

impl<'a> Financial<'a> {
    pub fn new(symbol: &'a str) -> Self {
        Self { symbol }
    }

    pub async fn income(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "financials/income-statement",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn balance(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "financials/balance-sheet-statement",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn cashflow(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "financials/cash-flow-statement",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn metrics(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "company-key-metrics",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn growth(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "financial-statement-growth",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn company_value(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });

        make_request(
            "enterprise-value",
            generate_json(Value::String(self.symbol.to_string()), Some(query_params))
        ).await
    }

    pub async fn ratios(&self) -> Result<Value, reqwest::Error> {
        make_request(
            "financial-ratios",
            generate_json(Value::String(self.symbol.to_string()), None)
        ).await
    }

    pub async fn dcf(&self) -> Result<Value, reqwest::Error> {
        make_request(
            "discounted-cash-flow",
            generate_json(Value::String(self.symbol.to_string()), None)
        ).await
    }

    pub async fn all(&self) -> Result<HashMap<String, Value>, reqwest::Error> {
        let mut data = HashMap::new();

        data.insert("income".to_string(), self.income(None).await?);
        data.insert("balance".to_string(), self.balance(None).await?);
        data.insert("cashflow".to_string(), self.cashflow(None).await?);
        data.insert("metrics".to_string(), self.metrics(None).await?);
        data.insert("growth".to_string(), self.growth(None).await?);
        data.insert("company_value".to_string(), self.company_value(None).await?);
        data.insert("ratios".to_string(), self.ratios().await?);
        data.insert("dcf".to_string(), self.dcf().await?);

        Ok(data)
    }
}


pub async fn example() -> Result<(), reqwest::Error> {
    let aapl = Financial::new("AAPL");
    
    // Get annual income statement
    let income = aapl.income(None).await?;
    
    // Get quarterly balance sheet
    let balance = aapl.balance(Some("quarter")).await?;
    
    // Get financial ratios
    let ratios = aapl.ratios().await?;
    
    Ok(())
}