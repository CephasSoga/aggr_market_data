use std::fmt;
use std::hash::Hash;
use std::time::Duration;

use serde::Deserialize;
use config::{builder::DefaultState, ConfigBuilder, ConfigError, File};

#[derive(Debug, Clone, Hash, Deserialize)]
pub struct ConfigHeader {
    msg: String,
}


#[derive(Debug, Clone, Hash, Deserialize)]
pub struct AuthConfig {
    pub token: String,
}

#[derive(Debug, Clone, Hash, Deserialize)]
pub struct WebsocketConfig {
    pub concurrency_limit: usize,
    pub batch_size: usize,
    pub retry_attempts: u32,
    pub backoff_ms: u64,
    pub rate_limit_per_second: u32,
    pub cache_ttl: u32,
}

#[derive(Clone, Hash, Debug, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
}

#[derive(Debug, Clone, Hash, Deserialize)]
pub struct Config {
    pub header: ConfigHeader,
    pub auth: AuthConfig,
    pub websocket: WebsocketConfig,
}
impl Config {
    pub fn new() -> Result<Self, ConfigError> {
    // Builder
    let mut builder: ConfigBuilder<DefaultState> = ConfigBuilder::default(); // Use default() instead of new()

    // Start off by merging in the "default" configuration file
    builder = builder.add_source(File::with_name("config")); // Example of adding a file source


    // Build the configuration
    let config = builder.build()
        .map_err(|e| {
            return ConfigError::FileParse { uri: Some(e.to_string()), cause: Box::new(e) }
        })?;

    // Deserialize the configuration into our Config struct
    // return it
    config.try_deserialize()

    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Format the fields of ValueConfig as needed
        write!(f, "{}", self.header.msg)
    }
}


#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub concurrency_limit: usize,
    pub batch_size: usize,
    pub retry_attempts: u32,
    pub backoff_ms: u64,
    pub rate_limit_per_second: u32,
    pub cache_ttl: Duration,
}
impl Default for BatchConfig {
    fn default() -> Self {
        let config = Config::new().unwrap();
        Self {
            concurrency_limit: config.websocket.concurrency_limit,
            batch_size: config.websocket.batch_size,
            retry_attempts: config.websocket.retry_attempts,
            backoff_ms: config.websocket.backoff_ms,
            rate_limit_per_second: config.websocket.rate_limit_per_second,
            cache_ttl: Duration::from_secs(config.websocket.cache_ttl as u64),
        }
    }
}


#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub rate_limit_per_second: u32,
    pub concurrency_limit: usize,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
            rate_limit_per_second: 50,
            concurrency_limit: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimeConfig {
    pub ms_back: u64,
    pub days_back: u64,
}
impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            ms_back: 1000,
            days_back: 1,
        }
    }
    
}