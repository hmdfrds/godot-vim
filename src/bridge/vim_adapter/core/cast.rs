//! Safe integer conversion helpers for Godot interop.
//!
//! Godot uses `i32` for line/column indices. Rust uses `usize` for indexing.
//! These helpers provide explicit, safe conversions with saturating semantics.
//!
//! # Rationale
//!
//! Saturating conversions are used rather than `TryFrom` with error handling because:
//! 1. Godot editor limits are well under `i32::MAX` (practical line counts < 1M)
//! 2. Saturating provides predictable behavior without panic paths
//! 3. Simpler API for code that bridges Godot and Rust index types
//!
//! # Examples
//!
//! ```ignore
//! use crate::bridge::vim_adapter::core::cast::{i32_to_usize, usize_to_i32};
//!
//! let godot_line: i32 = 42;
//! let rust_idx: usize = i32_to_usize(godot_line);
//!
//! let rust_col: usize = 100;
//! let godot_col: i32 = usize_to_i32(rust_col);
//! ```

/// Converts Godot `i32` to Rust `usize` (saturating to 0 for negatives).
///
/// Godot APIs may return -1 or negative values for "not found" scenarios.
/// This helper safely handles those cases by saturating to 0.
#[inline]
#[must_use]
#[allow(clippy::cast_sign_loss)] // Intentional: we've checked val >= 0
pub const fn i32_to_usize(val: i32) -> usize {
    if val < 0 {
        0
    } else {
        val as usize
    }
}

/// Converts Rust `usize` to Godot `i32` (saturating to `i32::MAX` for large values).
///
/// On 64-bit systems, `usize` can exceed `i32::MAX`. This helper saturates
/// to prevent overflow/wrap-around when passing to Godot APIs.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_truncation)] // Intentional: we've checked val <= i32::MAX
#[allow(clippy::cast_possible_wrap)] // Intentional: we've checked val <= i32::MAX
pub const fn usize_to_i32(val: usize) -> i32 {
    if val > i32::MAX as usize {
        i32::MAX
    } else {
        val as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_i32_to_usize_positive() {
        assert_eq!(i32_to_usize(0), 0);
        assert_eq!(i32_to_usize(1), 1);
        assert_eq!(i32_to_usize(100), 100);
        assert_eq!(i32_to_usize(i32::MAX), i32::MAX as usize);
    }

    #[test]
    fn test_i32_to_usize_negative_saturates() {
        assert_eq!(i32_to_usize(-1), 0);
        assert_eq!(i32_to_usize(-100), 0);
        assert_eq!(i32_to_usize(i32::MIN), 0);
    }

    #[test]
    fn test_usize_to_i32_small() {
        assert_eq!(usize_to_i32(0), 0);
        assert_eq!(usize_to_i32(1), 1);
        assert_eq!(usize_to_i32(100), 100);
    }

    #[test]
    fn test_usize_to_i32_max_boundary() {
        assert_eq!(usize_to_i32(i32::MAX as usize), i32::MAX);
        assert_eq!(usize_to_i32((i32::MAX as usize) - 1), i32::MAX - 1);
    }

    #[test]
    fn test_usize_to_i32_overflow_saturates() {
        assert_eq!(usize_to_i32((i32::MAX as usize) + 1), i32::MAX);
        assert_eq!(usize_to_i32(usize::MAX), i32::MAX);
    }
}
