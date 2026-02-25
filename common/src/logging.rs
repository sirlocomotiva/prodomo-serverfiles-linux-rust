use std::path::Path;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::{Layer, SubscriberExt},
    util::SubscriberInitExt,
    EnvFilter,
};

/// Log configuration for the server
pub struct LogConfig {
    /// Minimum log level for console output
    pub console_level: Level,
    /// Minimum log level for file output
    pub file_level: Level,
    /// Directory for log files
    pub log_dir: String,
    /// Whether to enable ANSI colors in console output
    pub ansi: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            console_level: Level::INFO,
            file_level: Level::DEBUG,
            log_dir: "./log".to_string(),
            ansi: true,
        }
    }
}

/// Initialize logging with the given configuration
///
/// Sets up tracing-subscriber with:
/// - Console output with timestamp, level, and target
/// - File output with daily rotation
/// - Configurable log levels via environment variable `RUST_LOG`
///
/// # Arguments
///
/// * `config` - Log configuration options
///
/// # Panics
///
/// Panics if the configured log directory cannot be created.
///
/// # Examples
///
/// ```no_run
/// use common::logging::{init_logging, LogConfig};
///
/// let config = LogConfig::default();
/// init_logging(&config);
/// ```
pub fn init_logging(config: &LogConfig) {
    let log_dir = Path::new(&config.log_dir);
    std::fs::create_dir_all(log_dir).expect("Failed to create log directory");

    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, "syslog");

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "common={},game_server={},db_server={},world={},quest={},net={},protocol={}",
            config.file_level,
            config.file_level,
            config.file_level,
            config.file_level,
            config.file_level,
            config.file_level,
            config.file_level,
        ))
    });

    let console_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE)
        .with_ansi(config.ansi)
        .with_filter(EnvFilter::new(config.console_level.to_string()));

    let file_layer = fmt::layer()
        .with_target(true)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE)
        .with_ansi(false)
        .with_writer(file_appender)
        .with_filter(EnvFilter::new(config.file_level.to_string()));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .init();
}

/// Initialize logging with default configuration
///
/// Uses INFO level for console, DEBUG for file output,
/// and writes logs to ./log directory.
pub fn init_default_logging() {
    init_logging(&LogConfig::default());
}

/// Initialize logging from environment variables
///
/// Reads configuration from:
/// - `RUST_LOG`: Log level filter (e.g., "debug", "info,common=trace")
/// - `LOG_DIR`: Log directory (default: "./log")
/// - `LOG_ANSI`: Enable ANSI colors (default: "true")
pub fn init_from_env() {
    let console_level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(Level::INFO);

    let log_dir = std::env::var("LOG_DIR").unwrap_or_else(|_| "./log".to_string());

    let ansi = std::env::var("LOG_ANSI")
        .map(|s| s.to_lowercase() == "true" || s == "1")
        .unwrap_or(true);

    let config = LogConfig {
        console_level,
        file_level: Level::DEBUG,
        log_dir,
        ansi,
    };

    init_logging(&config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert_eq!(config.console_level, Level::INFO);
        assert_eq!(config.file_level, Level::DEBUG);
        assert_eq!(config.log_dir, "./log");
        assert!(config.ansi);
    }

    #[test]
    fn test_init_logging_creates_dir() {
        let test_dir = "./test_log_dir";
        let config = LogConfig {
            log_dir: test_dir.to_string(),
            ..Default::default()
        };

        init_logging(&config);

        assert!(Path::new(test_dir).exists());

        let _ = std::fs::remove_dir_all(test_dir);
    }
}
