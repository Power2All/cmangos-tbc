// Logging module
// Rust equivalent of Log.h/cpp using the tracing crate
//
// The C++ code uses a custom logging system with multiple output files and
// log levels. In Rust, we use the `tracing` ecosystem which provides:
// - Structured logging
// - Multiple subscribers (file, stdout)
// - Log levels (ERROR, WARN, INFO, DEBUG, TRACE)
// - Filtering

use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling;
use std::path::Path;

/// Initialize the logging system
/// Maps the C++ log configuration to tracing subscribers
pub fn initialize_logging(log_dir: Option<&str>, log_level: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level));

    if let Some(dir) = log_dir {
        let path = Path::new(dir);
        if !path.exists() {
            let _ = std::fs::create_dir_all(path);
        }

        let file_appender = rolling::daily(dir, "realmd.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

        // Keep the guard alive by leaking it (it lives for the program duration)
        std::mem::forget(_guard);

        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .with_ansi(true)
                    .with_target(false)
                    .with_thread_ids(false),
            )
            .with(
                fmt::layer()
                    .with_writer(non_blocking)
                    .with_ansi(false)
                    .with_target(true),
            )
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
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
