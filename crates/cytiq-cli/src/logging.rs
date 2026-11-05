use std::env;
use std::io::{self, IsTerminal, Write};
use std::sync::OnceLock;

use log::{LevelFilter, Log, Metadata, Record, SetLoggerError};

pub(crate) static LOGGER: SimpleLogger = SimpleLogger::new();

pub(crate) struct SimpleLogger {
    level: OnceLock<LevelFilter>,
}

impl SimpleLogger {
    const fn new() -> Self {
        Self {
            level: OnceLock::new(),
        }
    }
}

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.level.get().copied().unwrap_or(LevelFilter::Off) >= metadata.level().to_level_filter()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let target = record.module_path_static().unwrap_or(record.target());
        // The binary logs as `cytiq` from every one of its modules.
        let target = if target.starts_with("cytiq::") {
            "cytiq"
        } else {
            target
        };
        let mut stderr = io::stderr();
        let _ = writeln!(stderr, "[{}] {}: {}", record.level(), target, record.args());
    }

    fn flush(&self) {}
}

pub(crate) fn init_logging() -> Result<(), SetLoggerError> {
    let level = configured_log_level();
    let _ = LOGGER.level.set(level);
    log::set_logger(&LOGGER)?;
    log::set_max_level(level);
    Ok(())
}

pub(crate) fn configured_log_level() -> LevelFilter {
    env::var("CYTIQ_LOG")
        .ok()
        .or_else(|| env::var("RUST_LOG").ok())
        .as_deref()
        .and_then(parse_log_level)
        .unwrap_or_else(default_log_level)
}

pub(crate) fn default_log_level() -> LevelFilter {
    if io::stderr().is_terminal() {
        LevelFilter::Info
    } else {
        LevelFilter::Warn
    }
}

pub(crate) fn parse_log_level(raw: &str) -> Option<LevelFilter> {
    raw.split(',')
        .rev()
        .find_map(|token| parse_log_level_token(token.trim()))
}

pub(crate) fn parse_log_level_token(token: &str) -> Option<LevelFilter> {
    let candidate = token.rsplit('=').next().unwrap_or(token).trim();
    match candidate.to_ascii_lowercase().as_str() {
        "off" => Some(LevelFilter::Off),
        "error" => Some(LevelFilter::Error),
        "warn" | "warning" => Some(LevelFilter::Warn),
        "info" => Some(LevelFilter::Info),
        "debug" => Some(LevelFilter::Debug),
        "trace" => Some(LevelFilter::Trace),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_each_level_and_directive_suffix() {
        for (name, level) in [
            ("off", LevelFilter::Off),
            ("error", LevelFilter::Error),
            ("warn", LevelFilter::Warn),
            ("warning", LevelFilter::Warn),
            ("info", LevelFilter::Info),
            ("debug", LevelFilter::Debug),
            ("trace", LevelFilter::Trace),
        ] {
            assert_eq!(parse_log_level_token(name), Some(level));
            assert_eq!(
                parse_log_level_token(&format!("cytiq= {} ", name.to_uppercase())),
                Some(level)
            );
        }
        for name in ["", "cytiq=", "verbose", "info extra"] {
            assert_eq!(parse_log_level_token(name), None);
        }
        assert_eq!(
            parse_log_level("info, cytiq=trace,unknown"),
            Some(LevelFilter::Trace)
        );
    }
}
