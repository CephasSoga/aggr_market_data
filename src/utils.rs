use std::time::{Duration, SystemTime};
use chrono::{DateTime, Utc};

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
