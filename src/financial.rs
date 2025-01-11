#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::collections::HashMap;
use std::sync::Arc;
use crate::request::HTTPClient;
use serde_json::{json, Value};

const INCOME_PATH: &str = "income-statement";
const BALANCE_PATH: &str = "balance-sheet-statement";
const CASHFLOW_PATH: &str = "cash-flow-statement";
const METRICS_PATH: &str = "key-metrics-ttm";
const GROWTH_PATH: &str = "financial-growth";
const VALUE_PATH: &str = "enterprise-values";
const RATIO_PATH: &str = "ratios-ttm";
const DCF_PATH: &str = "advanced_discounted_cash_flow";

/// Functions for accessing financial statement data from the FMP API
pub struct Financial<'a> {
    symbol: &'a str,
    http_client: Arc<HTTPClient>,
}

impl<'a> Financial<'a> {
    pub fn new(symbol: &'a str, http_client: Arc<HTTPClient>) -> Self {
        Self { symbol, http_client }
    }

    pub async fn income(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });
        let path = self.http_client.join(vec![INCOME_PATH, self.symbol]);
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(path.as_str(), Some(query_params)).await
    }

    pub async fn balance(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });
        let path = self.http_client.join(vec![BALANCE_PATH, self.symbol]);
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(path.as_str(), Some(query_params)).await
    }

    pub async fn cashflow(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });
        let path = self.http_client.join(vec![CASHFLOW_PATH, self.symbol]);
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(path.as_str(), Some(query_params)).await
    }

    pub async fn metrics(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let path = self.http_client.join(vec![METRICS_PATH, self.symbol]);
        self.http_client.get(METRICS_PATH, None).await
    }

    pub async fn growth(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("annual")
        });
        let path = self.http_client.join(vec![GROWTH_PATH, self.symbol]);
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(path.as_str(), Some(query_params)).await
    }

    pub async fn company_value(&self, period: Option<&str>) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "period": period.unwrap_or("quarter")
        });
        let path = self.http_client.join(vec![VALUE_PATH, self.symbol]);
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get(path.as_str(), Some(query_params)).await
    }

    pub async fn ratios(&self) -> Result<Value, reqwest::Error> {
        let path = self.http_client.join(vec![RATIO_PATH, self.symbol]);
        self.http_client.get(path.as_str(), None).await
    }

    pub async fn dcf(&self) -> Result<Value, reqwest::Error> {
        let query_params = json!({
            "symbol": self.symbol
        });
        let query_params = self.http_client.build_query_from_value(query_params);
        self.http_client.get_v4(DCF_PATH, Some(query_params)).await
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
    let aapl = Financial::new("AAPL", Arc::new(HTTPClient::new().unwrap()));
    
    // Get annual income statement
    let income = aapl.income(None).await?;
    
    // Get quarterly balance sheet
    let balance = aapl.balance(Some("quarter")).await?;
    
    // Get financial ratios
    let ratios = aapl.ratios().await?;
    
    Ok(())
}