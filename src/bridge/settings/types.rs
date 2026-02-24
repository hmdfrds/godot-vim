//! Type definitions for settings.

use strum::Display;

/// Log verbosity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display)]
#[repr(i64)]
pub enum LogLevel {
    Off = 0,
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl From<i64> for LogLevel {
    fn from(value: i64) -> Self {
        match value {
            0 => LogLevel::Off,
            1 => LogLevel::Error,
            2 => LogLevel::Warn,
            3 => LogLevel::Info,
            4 => LogLevel::Debug,
            5 => LogLevel::Trace,
            // Any unknown value defaults to Off
            _ => LogLevel::Off,
        }
    }
}

/// Mode for line number display in the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
#[repr(i32)]
pub enum LineNumberMode {
    /// No line numbers.
    None = 0,
    /// Absolute line numbers (standard).
    #[default]
    Absolute = 1,
    /// Relative line numbers (distance from cursor).
    Relative = 2,
    /// Hybrid: Absolute on current line, Relative on others.
    Hybrid = 3,
}

impl From<i64> for LineNumberMode {
    fn from(val: i64) -> Self {
        match val {
            0 => Self::None,
            1 => Self::Absolute,
            2 => Self::Relative,
            3 => Self::Hybrid,
            _ => Self::Absolute, // Default for unknown values
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_from_int() {
        assert_eq!(LogLevel::from(0), LogLevel::Off);
        assert_eq!(LogLevel::from(1), LogLevel::Error);
        assert_eq!(LogLevel::from(2), LogLevel::Warn);
        assert_eq!(LogLevel::from(3), LogLevel::Info);
        assert_eq!(LogLevel::from(4), LogLevel::Debug);
        assert_eq!(LogLevel::from(5), LogLevel::Trace);
        assert_eq!(LogLevel::from(99), LogLevel::Off); // Invalid falls back to Off
    }

    #[test]
    fn test_log_level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
    }
}
