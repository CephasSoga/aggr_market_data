#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
        }
    }
}

async fn process_batch(&self, tickers: Vec<String>, fetch_type: FetchType) -> Result<Value, StockError> {
    if tickers.is_empty() {
        return Err(StockError::VoidTickersError("No tickers provided".to_string()));
    }

    let retry_config = RetryConfig::default();
    let mut results = Vec::new();
    
    for ticker in tickers {
        let mut attempts = 0;
        let result = loop {
            attempts += 1;
            match fetch_type {
                FetchType::Quote => match self.quote(&ticker).await {
                    Ok(value) => {
                        counter!("stock.success").increment(1);
                        break Ok(value);
                    }
                    Err(e) => {
                        counter!("stock.retries").increment(1);
                        if attempts >= retry_config.max_attempts {
                            error!("Final retry failed for {}: {:?}", ticker, e);
                            counter!("stock.failures").increment(1);
                            break Err(e);
                        }
                        let delay = std::cmp::min(
                            retry_config.base_delay_ms * (2u64.pow(attempts - 1)),
                            retry_config.max_delay_ms
                        );
                        sleep(Duration::from_millis(delay)).await;
                        continue;
                    }
                },
                // Same pattern for other FetchType variants
                _ => self.fetch_with_retry(&ticker, fetch_type, &retry_config).await,
            }
        };

        match result {
            Ok(value) => results.push(value),
            Err(e) => {
                error!("Failed to fetch data for {}: {}", ticker, e);
                results.push(Value::Null);
            }
        }
    }

    Ok(Value::Array(results))
}

async fn fetch_with_retry(&self, ticker: &str, fetch_type: FetchType, config: &RetryConfig) -> Result<Value, StockError> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let result = match fetch_type {
            FetchType::Financial => self.financial(ticker).await,
            FetchType::Profile => self.profile(ticker).await,
            FetchType::Rating => self.rating(ticker).await,
            FetchType::CurrentPrice => self.current_price(ticker).await,
            FetchType::History => self.history(ticker, None, None, None, None).await,
            FetchType::DividendHistory => self.dividend_history(ticker, None, None, None, None).await,
            FetchType::SplitHistory => self.split_history(ticker, None, None, None, None).await,
            _ => unreachable!(),
        };

        match result {
            Ok(value) => {
                counter!("stock.success").increment(1);
                return Ok(value);
            }
            Err(e) => {
                counter!("stock.retries").increment(1);
                if attempts >= config.max_attempts {
                    error!("Final retry failed for {}: {:?}", ticker, e);
                    counter!("stock.failures").increment(1);
                    return Err(e);
                }
                let delay = std::cmp::min(
                    config.base_delay_ms * (2u64.pow(attempts - 1)),
                    config.max_delay_ms
                );
                sleep(Duration::from_millis(delay)).await;
            }
        }
    }
}