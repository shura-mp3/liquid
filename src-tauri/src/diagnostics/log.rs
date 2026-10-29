//! Structured logging to a rotating file plus stderr in development.
//!
//! Logs go to the app data directory, never to the install directory, which may be
//! read-only. Nothing in a log line is sent anywhere: there is no telemetry in
//! LIQUID, so these files exist purely for the user and for bug reports they choose
//! to share.

use std::path::Path;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Initialise logging. Returns the guard that must be kept alive for the lifetime of
/// the process — dropping it stops the background writer and loses buffered lines.
pub fn init(log_dir: &Path) -> Option<tracing_appender::non_blocking::WorkerGuard> {
    if let Err(e) = std::fs::create_dir_all(log_dir) {
        eprintln!("LIQUID: could not create log directory {log_dir:?}: {e}");
        return None;
    }

    let appender = tracing_appender::rolling::daily(log_dir, "liquid.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_env("LIQUID_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,liquid=debug"));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true);

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    // A console layer is only useful in a dev build; the release build is a
    // windows_subsystem="windows" binary with no attached console.
    #[cfg(debug_assertions)]
    let registry = registry.with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false),
    );

    if registry.try_init().is_err() {
        // Already initialised, e.g. by a test harness. Not fatal.
        return Some(guard);
    }

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        ?log_dir,
        "LIQUID starting"
    );
    Some(guard)
}
