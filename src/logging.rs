use tracing::{info, debug, error, warn, trace};
use tracing_subscriber;

pub enum LogLevel {
    Trace, Info, Debug, Warn, Error
}
impl LogLevel {
    fn to_log_level(&self) -> tracing::Level {
        match self {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        }
    }

    fn from_str(s: &str) -> Self{
        match s {
            "trace" => LogLevel::Trace,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Trace,
        }
    }
}
impl Default for LogLevel {
    fn default() -> Self {
        LogLevel::Trace
    }
}

pub struct Logger;

impl Logger {
    /// Initialize the logger
    pub fn init(level: LogLevel) {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE) // Set the maximum log level
            .init();
    }

    /// Log a trace-level message
    pub fn trace(message: &str) {
        trace!(target: "custom_logger", "{}", message);
    }

    /// Log a debug-level message
    pub fn debug(message: &str) {
        debug!(target: "custom_logger", "{}", message);
    }

    /// Log an info-level message
    pub fn info(message: &str) {
        info!(target: "custom_logger", "{}", message);
    }

    /// Log a warn-level message
    pub fn warn(message: &str) {
        warn!(target: "custom_logger", "{}", message);
    }

    /// Log an error-level message
    pub fn error(message: &str) {
        error!(target: "custom_logger", "{}", message);
    }
}

pub fn test_() {
    // Initialize the logger
    Logger::init(LogLevel::Trace);

    // Example logs
    Logger::trace("This is a trace message.");
    Logger::debug("This is a debug message.");
    Logger::info("This is an info message.");
    Logger::warn("This is a warning message.");
    Logger::error("This is an error message.");
}
