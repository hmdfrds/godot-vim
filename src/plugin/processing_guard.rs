//! RAII guard that sets a `bool` flag to `true` on creation and resets it
//! to `false` on drop — including during panic unwind. This makes it
//! impossible to leave `processing_key` stuck `true` after early returns,
//! panics, or forgotten cleanup.

/// Guard that sets `*flag = true` on construction and `*flag = false` on drop.
///
/// Uses disjoint field borrowing: the caller passes `&mut self.processing_key`
/// while separately borrowing `self.controller`. No `Cell` or `RefCell` needed.
pub(super) struct ProcessingKeyGuard<'a> {
    flag: &'a mut bool,
}

impl<'a> ProcessingKeyGuard<'a> {
    /// Set the flag to `true` and return a guard that resets it on drop.
    pub(super) fn new(flag: &'a mut bool) -> Self {
        *flag = true;
        Self { flag }
    }

    /// Returns the current value of the guarded flag.
    #[cfg(test)]
    fn is_active(&self) -> bool {
        *self.flag
    }
}

impl Drop for ProcessingKeyGuard<'_> {
    fn drop(&mut self) {
        *self.flag = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_sets_true_on_creation() {
        let mut flag = false;
        let guard = ProcessingKeyGuard::new(&mut flag);
        assert!(guard.is_active());
    }

    #[test]
    fn guard_resets_on_drop() {
        let mut flag = false;
        {
            let guard = ProcessingKeyGuard::new(&mut flag);
            assert!(guard.is_active());
        }
        assert!(!flag);
    }

    #[test]
    fn guard_resets_on_early_return() {
        fn returns_early(flag: &mut bool) -> Option<()> {
            let guard = ProcessingKeyGuard::new(flag);
            assert!(guard.is_active());
            None
        }
        let mut flag = false;
        let result = returns_early(&mut flag);
        assert!(result.is_none());
        assert!(!flag, "flag should be false after early return");
    }

    #[test]
    fn guard_resets_on_panic_unwind() {
        let mut flag = false;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let guard = ProcessingKeyGuard::new(&mut flag);
            assert!(guard.is_active());
            panic!("simulated panic");
        }));
        assert!(result.is_err());
        assert!(!flag, "flag should be false after panic unwind");
    }
}
