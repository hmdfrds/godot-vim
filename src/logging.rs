use godot::prelude::*;
use log::{Level, Metadata, Record};

struct GodotLogger;

/// Shorten a Rust module target path for readable log output.
///
/// Strips the crate prefix and keeps the last 2 path segments:
///   "godot_vim::bridge::vim_adapter::handlers::mode" → "handlers::mode"
///   "vim_core::runtime::execute"                     → "runtime::execute"
///   "godot_vim::logging"                             → "logging"
fn shorten_target(target: &str) -> &str {
    let stripped = target
        .strip_prefix("godot_vim::bridge::vim_adapter::")
        .or_else(|| target.strip_prefix("godot_vim::bridge::"))
        .or_else(|| target.strip_prefix("godot_vim::"))
        .or_else(|| target.strip_prefix("vim_core::"))
        .unwrap_or(target);

    // Keep last 2 segments for context
    let mut last_sep = None;
    let mut second_last_sep = None;
    for (i, c) in stripped.char_indices() {
        if c == ':' && stripped.as_bytes().get(i + 1) == Some(&b':') {
            second_last_sep = last_sep;
            last_sep = Some(i);
        }
    }

    match second_last_sep {
        Some(pos) => &stripped[pos + 2..],
        None => stripped,
    }
}

impl log::Log for GodotLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Trace
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let target = shorten_target(record.target());

            match record.level() {
                Level::Error => godot_error!("[{}] {}", target, record.args()),
                Level::Warn => godot_warn!("[{}] {}", target, record.args()),
                Level::Info => godot_print!("[{}] {}", target, record.args()),
                Level::Debug => godot_print!("[DBG][{}] {}", target, record.args()),
                Level::Trace => godot_print!("[TRC][{}] {}", target, record.args()),
            }
        }
    }

    fn flush(&self) {}
}

static LOGGER: GodotLogger = GodotLogger;

use crate::bridge::settings::types::LogLevel;

pub fn set_level(level: LogLevel) {
    let filter = match level {
        LogLevel::Off => log::LevelFilter::Off,
        LogLevel::Error => log::LevelFilter::Error,
        LogLevel::Warn => log::LevelFilter::Warn,
        LogLevel::Info => log::LevelFilter::Info,
        LogLevel::Debug => log::LevelFilter::Debug,
        LogLevel::Trace => log::LevelFilter::Trace,
    };
    log::set_max_level(filter);
}

pub fn init_logging() {
    // Silently ignore "already initialized" error during hot-reload
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Off));
    // Default to Off until settings load
}

