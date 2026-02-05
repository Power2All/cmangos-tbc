// Logging module
// Rust equivalent of Log.h/cpp using the tracing crate
//
// The C++ code uses a custom logging system with multiple output files and
// log levels. In Rust, we use the `tracing` ecosystem which provides:
// - Structured logging
// - Multiple subscribers (file, stdout)
// - Log levels (ERROR, WARN, INFO, DEBUG, TRACE)
// - Filtering
//
// C++ LogLevel mapping:
//   0 = Minimum  -> ERROR  (only errors)
//   1 = Error    -> WARN   (errors + warnings)
//   2 = Detail   -> INFO   (normal operation)
//   3 = Full     -> DEBUG  (detailed activity)
//   4 = Trace    -> TRACE  (packet-level debugging)

use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling;
use std::path::Path;

/// Map the C++ LogLevel integer (0-4) to a tracing filter string.
///
/// C++ levels:
///   0 = Minimum  -> only errors
///   1 = Error    -> errors and warnings
///   2 = Detail   -> normal informational (default)
///   3 = Full     -> debug-level detail
///   4 = Trace    -> everything including packet data
pub fn map_log_level(level: i32) -> &'static str {
    match level {
        0 => "error",
        1 => "warn",
        2 => "info",
        3 => "debug",
        4 => "trace",
        _ if level >= 4 => "trace",
        _ => "error",
    }
}

/// Initialize the logging system
/// Maps the C++ log configuration to tracing subscribers
///
/// Parameters:
///   log_dir       - Optional directory for log files
///   console_level - Tracing filter for console output (e.g., "info", "debug", "trace")
///   file_level    - Optional tracing filter for file output (defaults to console_level)
pub fn initialize_logging(log_dir: Option<&str>, console_level: &str, file_level: Option<&str>) {
    // RUST_LOG env var always takes precedence over config
    let console_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(console_level));

    if let Some(dir) = log_dir {
        let path = Path::new(dir);
        if !path.exists() {
            let _ = std::fs::create_dir_all(path);
        }

        let file_appender = rolling::daily(dir, "realmd.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

        // Keep the guard alive by leaking it (it lives for the program duration)
        std::mem::forget(_guard);

        let file_filter_str = file_level.unwrap_or(console_level);
        let file_filter = EnvFilter::new(file_filter_str);

        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_ansi(true)
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_filter(console_filter),
            )
            .with(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_target(true)
                    .with_filter(file_filter),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(console_filter)
            .with(
                fmt::layer()
                    .with_ansi(true)
                    .with_target(false)
                    .with_thread_ids(false),
            )
            .init();
    }
}

/// Convenience macros that map to the C++ logging functions
/// These re-export tracing macros with the naming convention from the C++ code
#[macro_export]
macro_rules! basic_log {
    ($($arg:tt)*) => { tracing::info!($($arg)*) };
}

#[macro_export]
macro_rules! detail_log {
    ($($arg:tt)*) => { tracing::debug!($($arg)*) };
}

#[macro_export]
macro_rules! debug_log {
    ($($arg:tt)*) => { tracing::trace!($($arg)*) };
}

#[macro_export]
macro_rules! error_log {
    ($($arg:tt)*) => { tracing::error!($($arg)*) };
}
