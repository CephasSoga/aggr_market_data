#![allow(dead_code)]
#![allow(warnings)]
#![allow(unused_variables)]

use std::sync::Arc;
use std::time::{Instant, Duration, SystemTime};

use chrono::{DateTime, Utc};
use tokio::time::sleep;
use futures_util::Future;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use crate::config::{RetryConfig, BatchConfig};
use crate::cache::{Cache,SharedCache, SharedLockedCache};

pub fn now() -> String {
    let now = SystemTime::now();
    let datetime: DateTime<Utc> = now.into();
    let formatted = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
    formatted
}

pub fn ago_secs(n: u64) -> String {
    let time = SystemTime::now() - Duration::from_secs(n);
    let datetime: DateTime<Utc> = time.into();
    let formatted = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
    formatted

}


pub async fn retry<F, Fut, T, E>(
    config: &Arc<RetryConfig>,
    mut operation: F,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempts = 0;

    loop {
        attempts += 1;
        match operation().await {
            Ok(value) => return Ok(value),
            Err(_err) if attempts < config.max_attempts => {
                let delay = std::cmp::min(
                    config.base_delay_ms * (2u64.pow(attempts - 1)),
                    config.max_delay_ms,
                );
                sleep(Duration::from_millis(delay)).await;
            }
            Err(err) => return Err(err),
        }
    }
}


pub async fn get_from_cache_or_fetch<F: Future<Output = Result<Value, Box<dyn std::error::Error>>>>(
    cache: &Arc<Mutex<SharedLockedCache>>,
    key: &str,
    fetch_fn: F,
    ttl: Duration,
) ->   Result<Value, Box<dyn std::error::Error>> 
where F: Future<Output = Result<Value, Box<dyn std::error::Error>>> {
    println!("Looking in cache");
    let mut cache = cache.lock().await;
    if let Some((value, instant)) = cache.get(key).await {
        println!("Found in cache");
        if instant.elapsed() < Duration::from_secs(60) {
            return Ok(value.clone());
        } else {
            println!("Expired");
            cache.pop(key);// Expired
        }
    }
    println!("Fetching...");
    // Fetch and cache the value
    let result = fetch_fn.await;
    match result {
        Ok(value) => {
            println!("Got value: {:?}", value);
            cache.put(key.to_string(), (value.clone(), Instant::now()));
            Ok(value)
        }
        Err(e) => Err(e),
    }
}

//-----------------------CloneOptions---------------: START
pub trait CloneOptions {
    type Output;

    fn clone_options(&self) -> Self::Output;
}
impl CloneOptions for (&Option<String>,) {
    type Output = (Option<String>,);

    fn clone_options(&self) -> Self::Output {
        (self.0.clone(),)
    }
}

impl CloneOptions for (&Option<String>, &Option<String>) {
    type Output = (Option<String>, Option<String>);

    fn clone_options(&self) -> Self::Output {
        (self.0.clone(), self.1.clone())
    }
}
impl CloneOptions for (&Option<String>, &Option<String>, &Option<String>) {
    type Output = (Option<String>, Option<String>, Option<String>);

    fn clone_options(&self) -> Self::Output {
        (self.0.clone(), self.1.clone(), self.2.clone())
    }
}
impl CloneOptions for (&Option<String>, &Option<String>, &Option<String>, &Option<String>) {
    type Output = (Option<String>, Option<String>, Option<String>, Option<String>);
    
    fn clone_options(&self) -> Self::Output {
        (self.0.clone(), self.1.clone(), self.2.clone(), self.3.clone())
    }
}

// Add more implementations for larger tuples as needed
pub fn clone_str_options<T>(options: T) -> <T as CloneOptions>::Output
where
    T: CloneOptions,
{
    options.clone_options()
}

//------------------CloneOptions---------------------: END


//-----------------------CloneArcs---------------: START

pub trait  CloneArcs {
    type Output;
    fn clone_arc_refs(&self) -> Self::Output;
   
}
impl CloneArcs for (Arc<BatchConfig>,) {
    type Output = (Arc<BatchConfig>,);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0),)
    }
}

impl CloneArcs for (Arc<SharedLockedCache>,) {
    type Output = (Arc<SharedLockedCache>,);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0),)
    }
}

impl CloneArcs for (Arc<Semaphore>,) {
    type Output = (Arc<Semaphore>,);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0),)
    }
}

impl CloneArcs for (Arc<RetryConfig>,) {
    type Output = (Arc<RetryConfig>,);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0),)
    }
}

impl CloneArcs for (Arc<BatchConfig>, Arc<SharedLockedCache>) {
    type Output = (Arc<BatchConfig>, Arc<SharedLockedCache>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<BatchConfig>, Arc<Semaphore>) {
    type Output = (Arc<BatchConfig>, Arc<Semaphore>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<BatchConfig>, Arc<RetryConfig>) {
    type Output = (Arc<BatchConfig>, Arc<RetryConfig>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<SharedLockedCache>, Arc<Semaphore>) {
    type Output = (Arc<SharedLockedCache>, Arc<Semaphore>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<SharedLockedCache>, Arc<RetryConfig>) {
    type Output = (Arc<SharedLockedCache>, Arc<RetryConfig>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<Semaphore>, Arc<RetryConfig>) {
    type Output = (Arc<Semaphore>, Arc<RetryConfig>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1))
    }
}

impl CloneArcs for (Arc<BatchConfig>, Arc<SharedLockedCache>, Arc<Semaphore>) {
    type Output = (Arc<BatchConfig>, Arc<SharedLockedCache>, Arc<Semaphore>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(&self.0), Arc::clone(&self.1), Arc::clone(&self.2))
    }   
}

impl CloneArcs for (&Arc<BatchConfig>, &Arc<SharedLockedCache>, &Arc<RetryConfig>) {
    type Output = (Arc<BatchConfig>, Arc<SharedLockedCache>, Arc<RetryConfig>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(self.0), Arc::clone(self.1), Arc::clone(self.2))
    }   
}

impl CloneArcs for (&Arc<BatchConfig>, &Arc<Semaphore>, &Arc<RetryConfig>) {
    type Output = (Arc<BatchConfig>, Arc<Semaphore>, Arc<RetryConfig>);

    fn clone_arc_refs(&self) -> Self::Output {
        (Arc::clone(self.0), Arc::clone(self.1), Arc::clone(self.2))
    }   
}

// Add more implementations for larger tuples as needed
pub fn clone_arc_refs<T>(arcs: T) -> <T as CloneArcs>::Output
where 
    T: CloneArcs,
{
    arcs.clone_arc_refs()
}
//---------------CloneArcs---------------------: END