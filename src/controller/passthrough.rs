//! Key passthrough classification: decides which keys bypass the Vim engine
//! and flow through to Godot's native input handling.
//!
//! Three sources compose the decision (checked in this order by the caller):
//! 1. **Mapping priority** — pending/startable mappings always go to the engine.
//! 2. **User overrides** — explicit passthrough keys from EditorSettings.
//! 3. **Host policy** — F-keys and Alt/Meta always pass through (IDE shortcuts).
//! 4. **Engine query** — `would_handle_key()` is the final arbiter.
//!
//! This module implements source #3. Sources #1, #2, and #4 live in
//! `ProcessContext::should_passthrough_key`.

use vim_core::keymap::{Key, KeyEvent, Modifiers};

/// F-keys belong to the IDE (run project, debug, step). Alt/Meta combos
/// are OS-level or editor-level shortcuts (Alt+Tab, Cmd+S).
pub(crate) fn is_always_passthrough(key: KeyEvent) -> bool {
    if matches!(key.key(), Key::F(_)) {
        return true;
    }

    let mods = key.modifiers();

    if mods.contains(Modifiers::ALT) || mods.contains(Modifiers::META) {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── F-keys ─────────────────────────────────────────────────────────

    #[test]
    fn f_keys_always_passthrough() {
        for n in 1..=12 {
            let key = KeyEvent::new(Key::F(n), Modifiers::NONE);
            assert!(is_always_passthrough(key), "F{n} should passthrough");
        }
    }

    #[test]
    fn f_keys_with_modifiers_passthrough() {
        let key = KeyEvent::new(Key::F(5), Modifiers::SHIFT);
        assert!(is_always_passthrough(key));

        let key = KeyEvent::new(Key::F(1), Modifiers::CTRL);
        assert!(is_always_passthrough(key));
    }

    // ── Alt / Meta ─────────────────────────────────────────────────────

    #[test]
    fn alt_combos_always_passthrough() {
        assert!(is_always_passthrough(KeyEvent::alt('x')));
        assert!(is_always_passthrough(KeyEvent::alt('s')));
    }

    #[test]
    fn meta_combos_always_passthrough() {
        let key = KeyEvent::new(Key::Char('s'), Modifiers::META);
        assert!(is_always_passthrough(key));
    }

    #[test]
    fn meta_keys_always_passthrough() {
        for c in 'a'..='z' {
            let key = KeyEvent::new(Key::Char(c), Modifiers::META);
            assert!(
                is_always_passthrough(key),
                "Meta+{} should pass through",
                c
            );
        }
    }

    // ── Non-passthrough keys ───────────────────────────────────────────

    #[test]
    fn plain_keys_not_always_passthrough() {
        assert!(!is_always_passthrough(KeyEvent::char('j')));
        assert!(!is_always_passthrough(KeyEvent::char(' ')));
        assert!(!is_always_passthrough(KeyEvent::escape()));
    }

    #[test]
    fn ctrl_keys_not_always_passthrough() {
        assert!(!is_always_passthrough(KeyEvent::ctrl('s')));
        assert!(!is_always_passthrough(KeyEvent::ctrl('d')));
    }
}
