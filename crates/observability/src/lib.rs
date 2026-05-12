//! Shared logging and tracing setup for COAT Rust services.
//!
//! The coordinator and service processes should expose enough local signal for
//! operators to understand request flow, runner routing, projections, and
//! wait-state transitions without each binary inventing its own logging knobs.
//! This crate only initializes process-local tracing; it does not own durable
//! state or export telemetry by itself.

use tracing_subscriber::EnvFilter;

/// Initialize process-local tracing for a COAT service.
///
/// Resolution order:
/// 1. `RUST_LOG`
/// 2. `COAT_RUST_LOG`
/// 3. `COAT_LOG_LEVEL` plus optional `COAT_LOG_TARGETS`
/// 4. the service's compiled default filter
///
/// Format knobs:
/// - `COAT_LOG_FORMAT=compact|pretty|json`
/// - `COAT_LOG_ANSI=true|false`
/// - `COAT_LOG_THREAD_IDS=true|false`
/// - `COAT_LOG_FILE=true|false`
/// - `COAT_LOG_LINE=true|false`
pub fn init_tracing(service_name: &'static str, default_filter: &'static str) {
    let filter = log_filter(service_name, default_filter);
    let format = std::env::var("COAT_LOG_FORMAT")
        .unwrap_or_else(|_| "compact".to_string())
        .to_ascii_lowercase();
    let ansi = env_bool("COAT_LOG_ANSI", format != "json");
    let thread_ids = env_bool("COAT_LOG_THREAD_IDS", false);
    let file = env_bool("COAT_LOG_FILE", false);
    let line = env_bool("COAT_LOG_LINE", false);

    let initialized = match format.as_str() {
        "json" => tracing_subscriber::fmt()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_env_filter(EnvFilter::new(filter.clone()))
            .with_target(true)
            .with_thread_ids(thread_ids)
            .with_file(file)
            .with_line_number(line)
            .try_init(),
        "pretty" => tracing_subscriber::fmt()
            .pretty()
            .with_env_filter(EnvFilter::new(filter.clone()))
            .with_target(true)
            .with_ansi(ansi)
            .with_thread_ids(thread_ids)
            .with_file(file)
            .with_line_number(line)
            .try_init(),
        _ => tracing_subscriber::fmt()
            .compact()
            .with_env_filter(EnvFilter::new(filter.clone()))
            .with_target(true)
            .with_ansi(ansi)
            .with_thread_ids(thread_ids)
            .with_file(file)
            .with_line_number(line)
            .try_init(),
    };

    if initialized.is_ok() {
        tracing::info!(
            service.name = service_name,
            log.filter = %filter,
            log.format = %format,
            log.ansi = ansi,
            "tracing initialized"
        );
    }
}

fn log_filter(service_name: &str, default_filter: &str) -> String {
    std::env::var("RUST_LOG")
        .or_else(|_| std::env::var("COAT_RUST_LOG"))
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            let Some(level) = std::env::var("COAT_LOG_LEVEL")
                .ok()
                .filter(|value| !value.trim().is_empty())
            else {
                return default_filter.to_string();
            };

            let module = service_name.replace('-', "_");
            let targets = std::env::var("COAT_LOG_TARGETS")
                .ok()
                .filter(|value| !value.trim().is_empty());
            match targets {
                Some(targets) => format!("{targets},{module}={level}"),
                None => format!("info,tower_http={level},{module}={level}"),
            }
        })
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn local_level_builds_service_and_http_filter() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let old_rust_log = std::env::var("RUST_LOG").ok();
        let old_coat_rust_log = std::env::var("COAT_RUST_LOG").ok();
        let old_level = std::env::var("COAT_LOG_LEVEL").ok();
        let old_targets = std::env::var("COAT_LOG_TARGETS").ok();
        unsafe {
            std::env::remove_var("RUST_LOG");
            std::env::remove_var("COAT_RUST_LOG");
            std::env::set_var("COAT_LOG_LEVEL", "debug");
            std::env::remove_var("COAT_LOG_TARGETS");
        }

        assert_eq!(
            log_filter("coat-goal-store", "coat_goal_store=info"),
            "info,tower_http=debug,coat_goal_store=debug"
        );

        restore_env("RUST_LOG", old_rust_log);
        restore_env("COAT_RUST_LOG", old_coat_rust_log);
        restore_env("COAT_LOG_LEVEL", old_level);
        restore_env("COAT_LOG_TARGETS", old_targets);
    }

    #[test]
    fn rust_log_wins_over_local_level() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let old_rust_log = std::env::var("RUST_LOG").ok();
        let old_level = std::env::var("COAT_LOG_LEVEL").ok();
        unsafe {
            std::env::set_var("RUST_LOG", "coat_coordinator=trace");
            std::env::set_var("COAT_LOG_LEVEL", "debug");
        }

        assert_eq!(
            log_filter("coat-coordinator", "coat_coordinator=info"),
            "coat_coordinator=trace"
        );

        restore_env("RUST_LOG", old_rust_log);
        restore_env("COAT_LOG_LEVEL", old_level);
    }

    fn restore_env(name: &str, value: Option<String>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
