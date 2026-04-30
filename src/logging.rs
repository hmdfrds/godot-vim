//! Logging bridge: routes Rust's `log` crate facade through Godot's output macros.
//!
//! Why a custom logger instead of `env_logger` or `tracing-subscriber`? Neither
//! can target Godot's Output/Debugger panels. This logger translates `log`
//! levels to `godot_print!` / `godot_warn!` / `godot_error!` so messages appear
//! in the same panels as Godot's own diagnostics, with severity-based coloring.
//!
//! The log level is configured at runtime via EditorSettings, not at compile
//! time, because users need to toggle verbosity without rebuilding the plugin.

use godot::prelude::*;
use log::{Level, LevelFilter, Metadata, Record};

/// Typed mirror of the log-level enum exposed in EditorSettings.
///
/// Exists because EditorSettings stores the value as a string; parsing into
/// this enum at read time catches typos immediately instead of silently
/// producing no output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LogLevel {
    #[default]
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Parse from the case-sensitive EditorSettings enum string.
    /// Unknown values silently default to `Off` to avoid flooding the console
    /// on a misconfigured `.godot-vimrc`.
    pub(crate) fn from_setting(s: &str) -> Self {
        match s {
            "Off" => Self::Off,
            "Error" => Self::Error,
            "Warn" => Self::Warn,
            "Info" => Self::Info,
            "Debug" => Self::Debug,
            "Trace" => Self::Trace,
            _ => {
                log::warn!("Unknown log level {s:?}, defaulting to Off");
                Self::Off
            }
        }
    }

    fn to_level_filter(self) -> LevelFilter {
        match self {
            Self::Off => LevelFilter::Off,
            Self::Error => LevelFilter::Error,
            Self::Warn => LevelFilter::Warn,
            Self::Info => LevelFilter::Info,
            Self::Debug => LevelFilter::Debug,
            Self::Trace => LevelFilter::Trace,
        }
    }
}

#[derive(Debug)]
struct GodotLogger;

/// Shorten a `log` target path for display in Godot's Output panel.
///
/// Strips the `godot_vim::` or `vim_core::` crate prefix (if present),
/// then keeps at most the last two `::` segments. This keeps log lines readable
/// without sacrificing enough context to locate the source module:
///
/// - `godot_vim::effects::marks` -> `effects::marks`
/// - `vim_core::commands::helpers` -> `commands::helpers`
/// - `godot_vim::ui::cursor_shape::CursorOverlay` -> `cursor_shape::CursorOverlay`
/// - `some_other_crate::foo::bar` -> `foo::bar`
/// - `logging` -> `logging` (no separators -- returned as-is)
fn shorten_target(target: &str) -> &str {
    let stripped = target
        .strip_prefix("godot_vim::")
        .or_else(|| target.strip_prefix("vim_core::"))
        .unwrap_or(target);

    // Single-pass scan tracking the two most recent `::` positions.
    // When there are >=2 separators, we slice from the second-to-last one.
    let mut last_sep = None;
    let mut second_last_sep = None;
    let bytes = stripped.as_bytes();

    for (i, &b) in bytes.iter().enumerate() {
        if b == b':' && bytes.get(i + 1) == Some(&b':') {
            second_last_sep = last_sep;
            last_sep = Some(i);
        }
    }

    // Fewer than two separators means the stripped string already fits in two segments.
    match second_last_sep {
        Some(pos) => &stripped[pos + 2..],
        None => stripped,
    }
}

impl log::Log for GodotLogger {
    // Always return true; filtering is handled by `log::set_max_level` which
    // gates before `log()` is ever called. This avoids duplicate threshold logic.
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        // godot_print! et al. convert to GString via the Godot FFI, which
        // requires the engine to be initialized (MAIN_THREAD_ID set). In
        // `cargo test` the engine is never started, so this guard prevents
        // a "Godot engine not available" panic on any log macro.
        if !godot::sys::is_initialized() {
            return;
        }

        let target = shorten_target(record.target());
        let level = record.level();
        let args = record.args();

        match level {
            Level::Error => godot_error!("[{}] {}", target, args),
            Level::Warn => godot_warn!("[{}] {}", target, args),
            Level::Info => godot_print!("[{}] {}", target, args),
            Level::Debug => godot_print!("[DBG][{}] {}", target, args),
            Level::Trace => godot_print!("[TRC][{}] {}", target, args),
        }
    }

    fn flush(&self) {}
}

static LOGGER: GodotLogger = GodotLogger;

/// Install the global logger. Must be called exactly once per process.
///
/// Starts at `LevelFilter::Off`; the caller must follow up with [`set_level`]
/// once EditorSettings are available. The `let _ =` swallows the
/// `SetLoggerError` that occurs on hot-reload (gdext re-enters `init` but
/// `log` only allows one logger per process).
pub(crate) fn init() {
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(LevelFilter::Off));
}

/// Update the global log level filter at runtime (called when EditorSettings change).
pub(crate) fn set_level(level: LogLevel) {
    log::set_max_level(level.to_level_filter());
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // shorten_target
    // ---------------------------------------------------------------

    #[test]
    fn shorten_godot_vim_two_segments() {
        assert_eq!(
            shorten_target("godot_vim::effects::marks"),
            "effects::marks"
        );
    }

    #[test]
    fn shorten_vim_core_two_segments() {
        assert_eq!(
            shorten_target("vim_core::commands::helpers"),
            "commands::helpers"
        );
    }

    #[test]
    fn shorten_deep_path_three_plus_segments() {
        assert_eq!(
            shorten_target("godot_vim::ui::cursor_shape::CursorOverlay"),
            "cursor_shape::CursorOverlay"
        );
    }

    #[test]
    fn shorten_single_segment_after_strip() {
        assert_eq!(shorten_target("godot_vim::logging"), "logging");
    }

    #[test]
    fn shorten_unknown_crate() {
        assert_eq!(shorten_target("some_other_crate::foo::bar"), "foo::bar");
    }

    #[test]
    fn shorten_no_separators() {
        assert_eq!(shorten_target("logging"), "logging");
    }

    #[test]
    fn shorten_empty_string() {
        assert_eq!(shorten_target(""), "");
    }

    // ---------------------------------------------------------------
    // set_level
    // ---------------------------------------------------------------

    // NOTE: These tests mutate global state (`log::max_level`) and must NOT run
    // concurrently with anything that depends on a specific max level. Since
    // `cargo test` runs tests on multiple threads by default, each assertion is
    // made immediately after the set call to minimise interleaving risk. In
    // practice the `log` crate uses an atomic for the max level, so reading right
    // after writing is safe.

    #[test]
    fn set_level_all_variants() {
        // Ensure the logger is installed so set_max_level takes effect.
        init();

        set_level(LogLevel::Off);
        assert_eq!(log::max_level(), LevelFilter::Off);

        set_level(LogLevel::Error);
        assert_eq!(log::max_level(), LevelFilter::Error);

        set_level(LogLevel::Warn);
        assert_eq!(log::max_level(), LevelFilter::Warn);

        set_level(LogLevel::Info);
        assert_eq!(log::max_level(), LevelFilter::Info);

        set_level(LogLevel::Debug);
        assert_eq!(log::max_level(), LevelFilter::Debug);

        set_level(LogLevel::Trace);
        assert_eq!(log::max_level(), LevelFilter::Trace);

        // Reset to Off: if other tests trigger log macros while the Godot
        // engine is not running, GodotLogger would panic on GString creation.
        set_level(LogLevel::Off);
    }

    #[test]
    fn from_setting_valid_strings() {
        assert_eq!(LogLevel::from_setting("Off"), LogLevel::Off);
        assert_eq!(LogLevel::from_setting("Error"), LogLevel::Error);
        assert_eq!(LogLevel::from_setting("Warn"), LogLevel::Warn);
        assert_eq!(LogLevel::from_setting("Info"), LogLevel::Info);
        assert_eq!(LogLevel::from_setting("Debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from_setting("Trace"), LogLevel::Trace);
    }

    #[test]
    fn from_setting_unknown_defaults_to_off() {
        assert_eq!(LogLevel::from_setting("Verbose"), LogLevel::Off);
        assert_eq!(LogLevel::from_setting("info"), LogLevel::Off); // wrong case
        assert_eq!(LogLevel::from_setting(""), LogLevel::Off);
    }
}
