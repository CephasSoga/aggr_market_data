use reqwest::Client;
use std::sync::Arc;
use std::collections::HashMap;
use crate::config::Config;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct HTTPClient {
    client: Arc<Client>,
    headers: HashMap<String, String>,
    base_url: String,
    base_url_v4: String,
    config: Config,
}

const BASE_URL_V3: &str = "https://financialmodelingprep.com/api/v3/";
const BASE_URL_V4: &str = "https://financialmodelingprep.com/api/v4/";
const MAX_CLIENT_POOL_SIZE: usize = 1024;

impl HTTPClient {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            client: Arc::new(Client::builder()
            .pool_max_idle_per_host(MAX_CLIENT_POOL_SIZE)
            .build()?),
            headers: HashMap::new(),
            base_url: BASE_URL_V3.to_string(),
            base_url_v4: BASE_URL_V4.to_string(),
            config: Config::new()?,
        })
    }

    fn build_query(&self, mut query_params: Vec<(String, String)>) -> Vec<(String, String)> {
                query_params.push(("apikey".to_string(), self.config.auth.token.clone()));
                query_params
    }

    pub fn build_query_from_value(&self, query_params: Value) -> Vec<(String, String)> {
        // Convert the JSON object to a Vec<(String, String)>
        let query_vec: Vec<(String, String)> = query_params.as_object()
            .unwrap()
            .iter()
            .filter_map(|(key, value)| {
                value.as_str().map(|v| (key.clone(), v.to_string()))
            })
            .collect();
        query_vec
    }

    pub fn join<I, S>(&self, path_parts: I) -> String
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        path_parts
            .into_iter()
            .map(|part| part.as_ref().trim_matches('/').to_string())
            .collect::<Vec<_>>()
            .join("/")
    }


    pub fn set_header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    pub async fn get(&self, url: &str, query_params: Option<Vec<(String, String)>>) -> Result<Value, reqwest::Error> {
        println!("Maiking request with query: {:?}", query_params);
        let url = format!("{}/{}", self.base_url.trim_end_matches("/"), url.trim_start_matches("/"));

        if let Some(query_params) = query_params {
            let query_params = self.build_query(query_params);
            let response = self.client.get(&url).query(&query_params).send().await?.json().await?;
            Ok(response)
        }
        else {
            let response = self.client.get(&url)
                .query(&vec![("apikey".to_string(), self.config.auth.token.clone())])
                .send().await?.json().await?;
            Ok(response)
        }
        
    }

    pub async fn get_v4(&self, url: &str, query_params: Option<Vec<(String, String)>>) -> Result<Value, reqwest::Error> {
        println!("Maiking request with query: {:?}", query_params);
        let url = format!("{}/{}", self.base_url_v4.trim_end_matches("/"), url.trim_start_matches("/"));

        if let Some(query_params) = query_params {
            let query_params = self.build_query(query_params);
            let response = self.client.get(&url).query(&query_params).send().await?.json().await?;
            Ok(response)
        }
        else {
            let response = self.client.get(&url)
                .query(&vec![("apikey".to_string(), self.config.auth.token.clone())])
                .send().await?.json().await?;
            Ok(response)
        }
    }
}