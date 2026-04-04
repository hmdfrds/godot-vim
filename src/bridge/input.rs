//! Translates Godot [`InputEventKey`] into vim-core [`KeyEvent`].
//!
//! The translation has three paths (in priority order):
//! 1. **Named keys** — Enter, Escape, arrows, F-keys map directly.
//! 2. **Ctrl+key** — uses the raw keycode (not unicode), because Godot's
//!    unicode for Ctrl+letter is a control code (U+0001..U+001A) that the
//!    engine cannot distinguish from the desired `Key::Char('a')..='z'`.
//! 3. **Printable characters** — uses `get_unicode()` and strips the Shift
//!    modifier, since the shifted state is already encoded in the character
//!    (e.g. `'A'` = Shift+a, `'@'` = Shift+2).

use godot::classes::InputEventKey;
use godot::global::Key as GodotKey;
use godot::prelude::*;
use vim_core::keymap::{Key, KeyEvent, Modifiers};

/// Map Godot keycode to a named `Key`, or `None` to fall through to the
/// unicode/Ctrl paths. Keypad digits are normalized to their ASCII equivalents
/// so `KP_5` behaves like `5` in Vim commands.
fn get_named_key(raw: GodotKey) -> Option<Key> {
    match raw {
        GodotKey::ESCAPE => Some(Key::Escape),
        GodotKey::ENTER | GodotKey::KP_ENTER => Some(Key::Enter),
        GodotKey::BACKSPACE => Some(Key::Backspace),
        GodotKey::TAB => Some(Key::Tab),
        GodotKey::DELETE => Some(Key::Delete),
        GodotKey::INSERT => Some(Key::Insert),
        GodotKey::UP => Some(Key::Up),
        GodotKey::DOWN => Some(Key::Down),
        GodotKey::LEFT => Some(Key::Left),
        GodotKey::RIGHT => Some(Key::Right),
        GodotKey::HOME => Some(Key::Home),
        GodotKey::END => Some(Key::End),
        GodotKey::PAGEUP => Some(Key::PageUp),
        GodotKey::PAGEDOWN => Some(Key::PageDown),
        GodotKey::F1 => Some(Key::F(1)),
        GodotKey::F2 => Some(Key::F(2)),
        GodotKey::F3 => Some(Key::F(3)),
        GodotKey::F4 => Some(Key::F(4)),
        GodotKey::F5 => Some(Key::F(5)),
        GodotKey::F6 => Some(Key::F(6)),
        GodotKey::F7 => Some(Key::F(7)),
        GodotKey::F8 => Some(Key::F(8)),
        GodotKey::F9 => Some(Key::F(9)),
        GodotKey::F10 => Some(Key::F(10)),
        GodotKey::F11 => Some(Key::F(11)),
        GodotKey::F12 => Some(Key::F(12)),
        GodotKey::KP_0 => Some(Key::Char('0')),
        GodotKey::KP_1 => Some(Key::Char('1')),
        GodotKey::KP_2 => Some(Key::Char('2')),
        GodotKey::KP_3 => Some(Key::Char('3')),
        GodotKey::KP_4 => Some(Key::Char('4')),
        GodotKey::KP_5 => Some(Key::Char('5')),
        GodotKey::KP_6 => Some(Key::Char('6')),
        GodotKey::KP_7 => Some(Key::Char('7')),
        GodotKey::KP_8 => Some(Key::Char('8')),
        GodotKey::KP_9 => Some(Key::Char('9')),
        GodotKey::KP_MULTIPLY => Some(Key::Char('*')),
        GodotKey::KP_SUBTRACT => Some(Key::Char('-')),
        GodotKey::KP_ADD => Some(Key::Char('+')),
        GodotKey::KP_PERIOD => Some(Key::Char('.')),
        GodotKey::KP_DIVIDE => Some(Key::Char('/')),
        _ => None,
    }
}

/// Pure translation from raw key parameters to a vim-core [`KeyEvent`].
///
/// Contains all mapping logic without any Godot FFI calls, making it
/// independently testable.
///
/// Returns `None` for events we don't handle:
/// - Bare modifier keys (Shift/Ctrl/Alt/Meta/CapsLock pressed alone)
/// - Unrecognized keys with no unicode representation
/// - Control characters not produced by Ctrl+letter
pub(crate) fn translate_key(
    keycode: GodotKey,
    unicode: u32,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Option<KeyEvent> {
    // Build modifiers from bool parameters.
    let mut modifiers = Modifiers::NONE;
    if ctrl {
        modifiers |= Modifiers::CTRL;
    }
    if alt {
        modifiers |= Modifiers::ALT;
    }
    if shift {
        modifiers |= Modifiers::SHIFT;
    }
    if meta {
        modifiers |= Modifiers::META;
    }

    if matches!(
        keycode,
        GodotKey::SHIFT
            | GodotKey::CTRL
            | GodotKey::ALT
            | GodotKey::META
            | GodotKey::CAPSLOCK
    ) {
        log::trace!("parse_godot_key: filtered bare modifier {:?}", keycode);
        return None;
    }

    if let Some(key) = get_named_key(keycode) {
        log::trace!("parse_godot_key: named key {} mods={}", key, modifiers);
        return Some(KeyEvent::new(key, modifiers));
    }

    // Ctrl+letter: Godot's unicode is a control code (U+0001 for Ctrl+A, etc.)
    // which is useless. Use the raw keycode instead — Godot's Key enum maps
    // A-Z to ASCII ordinals 65-90, so we can safely convert.
    if modifiers.contains(Modifiers::CTRL) {
        let key_val = keycode.ord();
        let key_a = GodotKey::A.ord();
        let key_z = GodotKey::Z.ord();
        if (key_a..=key_z).contains(&key_val) {
            if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
                return Some(KeyEvent::new(
                    Key::Char(ch.to_ascii_lowercase()),
                    modifiers,
                ));
            }
            log::warn!(
                "parse_godot_key: Ctrl+letter keycode={} char conversion failed",
                key_val
            );
            return None;
        }
    }

    // Ctrl+Shift+non-letter: when Shift is held with Ctrl and the key is NOT
    // a letter (A-Z), prefer the unicode value if it's a printable non-control
    // character. This captures Ctrl+Shift+2 → '@', Ctrl+Shift+6 → '^', etc.
    // The shifted symbol already encodes Shift, so strip it from modifiers.
    if modifiers.contains(Modifiers::CTRL | Modifiers::SHIFT) {
        if unicode != 0 {
            if let Some(ch) = char::from_u32(unicode) {
                if !ch.is_control() && !ch.is_ascii_alphabetic() {
                    let mods = modifiers & !Modifiers::SHIFT;
                    return Some(KeyEvent::new(Key::Char(ch), mods));
                }
            }
        }
    }

    // Ctrl+non-letter (e.g. Ctrl+[, Ctrl+]): Godot's unicode is again a
    // control code (Ctrl+[ = U+001B = ESC). We need Key::Char('[') + Ctrl
    // so the engine can match Vim's <C-[>, <C-]>, <C-^> notation.
    if modifiers.contains(Modifiers::CTRL) {
        let key_val = keycode.ord();
        if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
            if !ch.is_control() {
                log::trace!(
                    "parse_godot_key: Ctrl+non-letter {:?} -> Key::Char({:?})",
                    keycode,
                    ch
                );
                return Some(KeyEvent::new(
                    Key::Char(ch.to_ascii_lowercase()),
                    modifiers,
                ));
            }
        }
    }

    if unicode == 0 {
        log::trace!("parse_godot_key: zero unicode for {:?}", keycode);
        return None;
    }
    let ch = char::from_u32(unicode)?;
    if ch.is_control() {
        log::trace!("parse_godot_key: control char U+{:04X} filtered", unicode);
        return None;
    }

    // For plain printable characters, Shift is already encoded in the unicode
    // value ('A' = Shift+a, '@' = Shift+2). Reporting Shift as a separate
    // modifier would cause the engine to see <S-A> instead of just 'A'.
    // Keep Shift only when combined with Ctrl/Alt/Meta (e.g. <C-S-f>);
    // named keys (<S-Tab>, <S-Left>) were already handled above.
    if !modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META) {
        modifiers &= !Modifiers::SHIFT;
    }

    // When unicode produced a non-ASCII character, check if the physical key
    // is a Latin letter. If so, provide the Latin equivalent for command
    // dispatch in non-insert modes (the engine normalizes via latin_key).
    // Only letters A-Z are remapped — symbols can't be derived from
    // keycode+shift and vary across Latin layouts.
    let event = KeyEvent::new(Key::Char(ch), modifiers);
    let event = if !ch.is_ascii()
        && !modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META)
    {
        let key_val = keycode.ord();
        let key_a = GodotKey::A.ord();
        let key_z = GodotKey::Z.ord();
        if (key_a..=key_z).contains(&key_val) {
            // Use the original `shift` parameter (not `modifiers`, which had
            // Shift stripped above) to determine upper/lower case.
            let latin_ch = if shift {
                (key_val as u8 as char).to_ascii_uppercase()
            } else {
                (key_val as u8 as char).to_ascii_lowercase()
            };
            log::trace!(
                "parse_godot_key: non-Latin '{}' with Latin equivalent '{}'",
                ch, latin_ch
            );
            event.with_latin(Key::Char(latin_ch))
        } else {
            event
        }
    } else {
        event
    };

    log::trace!("parse_godot_key: char='{}' mods={}", ch, modifiers);
    Some(event)
}

/// Convert a Godot `InputEventKey` into a vim-core `KeyEvent`.
///
/// Thin wrapper that extracts fields from the event and delegates to
/// [`translate_key`].
///
/// Returns `None` for events we don't handle:
/// - Bare modifier keys (Shift/Ctrl/Alt/Meta/CapsLock pressed alone)
/// - Unrecognized keys with no unicode representation
/// - Control characters not produced by Ctrl+letter
pub(crate) fn parse_godot_key(event: &Gd<InputEventKey>) -> Option<KeyEvent> {
    translate_key(
        event.get_keycode(),
        event.get_unicode(),
        event.is_ctrl_pressed(),
        event.is_alt_pressed(),
        event.is_shift_pressed(),
        event.is_meta_pressed(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Named keys ──────────────────────────────────────────────────────

    #[test]
    fn named_key_escape() {
        let result = translate_key(GodotKey::ESCAPE, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Escape, Modifiers::NONE)));
    }

    #[test]
    fn named_key_enter() {
        let result = translate_key(GodotKey::ENTER, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Enter, Modifiers::NONE)));
    }

    #[test]
    fn named_key_kp_enter() {
        let result = translate_key(GodotKey::KP_ENTER, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Enter, Modifiers::NONE)));
    }

    #[test]
    fn named_key_backspace() {
        let result = translate_key(GodotKey::BACKSPACE, 0, false, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Backspace, Modifiers::NONE))
        );
    }

    #[test]
    fn named_key_tab() {
        let result = translate_key(GodotKey::TAB, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Tab, Modifiers::NONE)));
    }

    #[test]
    fn named_key_delete() {
        let result = translate_key(GodotKey::DELETE, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Delete, Modifiers::NONE)));
    }

    #[test]
    fn named_key_insert() {
        let result = translate_key(GodotKey::INSERT, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Insert, Modifiers::NONE)));
    }

    #[test]
    fn named_key_arrows() {
        assert_eq!(
            translate_key(GodotKey::UP, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Up, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::DOWN, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Down, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::LEFT, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Left, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::RIGHT, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Right, Modifiers::NONE))
        );
    }

    #[test]
    fn named_key_navigation() {
        assert_eq!(
            translate_key(GodotKey::HOME, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Home, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::END, 0, false, false, false, false),
            Some(KeyEvent::new(Key::End, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::PAGEUP, 0, false, false, false, false),
            Some(KeyEvent::new(Key::PageUp, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::PAGEDOWN, 0, false, false, false, false),
            Some(KeyEvent::new(Key::PageDown, Modifiers::NONE))
        );
    }

    #[test]
    fn named_key_function_keys() {
        let godot_f_keys = [
            GodotKey::F1,
            GodotKey::F2,
            GodotKey::F3,
            GodotKey::F4,
            GodotKey::F5,
            GodotKey::F6,
            GodotKey::F7,
            GodotKey::F8,
            GodotKey::F9,
            GodotKey::F10,
            GodotKey::F11,
            GodotKey::F12,
        ];
        for (i, gk) in godot_f_keys.iter().enumerate() {
            let n = (i + 1) as u8;
            assert_eq!(
                translate_key(*gk, 0, false, false, false, false),
                Some(KeyEvent::new(Key::F(n), Modifiers::NONE)),
                "F{n} mapping failed"
            );
        }
    }

    #[test]
    fn named_key_keypad_digits() {
        let kp_keys = [
            (GodotKey::KP_0, '0'),
            (GodotKey::KP_1, '1'),
            (GodotKey::KP_2, '2'),
            (GodotKey::KP_3, '3'),
            (GodotKey::KP_4, '4'),
            (GodotKey::KP_5, '5'),
            (GodotKey::KP_6, '6'),
            (GodotKey::KP_7, '7'),
            (GodotKey::KP_8, '8'),
            (GodotKey::KP_9, '9'),
        ];
        for (gk, ch) in kp_keys {
            assert_eq!(
                translate_key(gk, 0, false, false, false, false),
                Some(KeyEvent::new(Key::Char(ch), Modifiers::NONE)),
                "KP_{ch} mapping failed"
            );
        }
    }

    #[test]
    fn named_key_keypad_operators() {
        assert_eq!(
            translate_key(GodotKey::KP_MULTIPLY, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Char('*'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::KP_SUBTRACT, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Char('-'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::KP_ADD, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Char('+'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::KP_PERIOD, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Char('.'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::KP_DIVIDE, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Char('/'), Modifiers::NONE))
        );
    }

    // ── Named keys with modifiers ───────────────────────────────────────

    #[test]
    fn named_key_with_shift_preserves_shift() {
        // Shift+Tab should keep the Shift modifier (named keys don't strip it).
        let result = translate_key(GodotKey::TAB, 0, false, false, true, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Tab, Modifiers::SHIFT)));
    }

    #[test]
    fn named_key_with_ctrl() {
        let result = translate_key(GodotKey::LEFT, 0, true, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Left, Modifiers::CTRL)));
    }

    // ── Ctrl+letter ─────────────────────────────────────────────────────

    #[test]
    fn ctrl_a() {
        // Ctrl+A: keycode = GodotKey::A, unicode = 1 (control code, ignored).
        let result = translate_key(GodotKey::A, 1, true, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('a'), Modifiers::CTRL))
        );
    }

    #[test]
    fn ctrl_z() {
        let result = translate_key(GodotKey::Z, 26, true, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('z'), Modifiers::CTRL))
        );
    }

    #[test]
    fn ctrl_letter_is_lowercase() {
        // Even though keycode is uppercase 'A', result char should be 'a'.
        let result = translate_key(GodotKey::A, 1, true, false, false, false);
        let key = result.unwrap().key();
        assert_eq!(key, Key::Char('a'));
    }

    #[test]
    fn ctrl_shift_letter() {
        // Ctrl+Shift+A should produce Key::Char('a') with CTRL|SHIFT.
        let result = translate_key(GodotKey::A, 1, true, false, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('a'),
                Modifiers::CTRL | Modifiers::SHIFT
            ))
        );
    }

    #[test]
    fn ctrl_shift_2_produces_ctrl_at() {
        // Ctrl+Shift+2 on US layout: keycode=KEY_2, unicode='@' (0x40),
        // ctrl=true, shift=true. Should produce Key::Char('@') + CTRL.
        let result = translate_key(GodotKey::KEY_2, 0x40, true, false, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('@'), Modifiers::CTRL)),
            "Ctrl+Shift+2 with unicode '@' should produce Ctrl+@"
        );
    }

    #[test]
    fn ctrl_shift_6_produces_ctrl_caret() {
        // Ctrl+Shift+6 on US layout: keycode=KEY_6, unicode='^' (0x5E),
        // ctrl=true, shift=true. Should produce Key::Char('^') + CTRL.
        let result = translate_key(GodotKey::KEY_6, 0x5E, true, false, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('^'), Modifiers::CTRL)),
            "Ctrl+Shift+6 with unicode '^' should produce Ctrl+^"
        );
    }

    // ── Ctrl+non-letter ─────────────────────────────────────────────────

    #[test]
    fn ctrl_open_bracket() {
        // Ctrl+[: keycode = GodotKey::BRACKETLEFT, unicode = 0x1B (ESC control code).
        let result = translate_key(GodotKey::BRACKETLEFT, 0x1B, true, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('['), Modifiers::CTRL))
        );
    }

    #[test]
    fn ctrl_close_bracket() {
        let result = translate_key(GodotKey::BRACKETRIGHT, 0x1D, true, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char(']'), Modifiers::CTRL))
        );
    }

    // ── Printable chars with shift stripping ────────────────────────────

    #[test]
    fn uppercase_a_strips_shift() {
        // Typing 'A' (Shift+a): unicode='A' (65), shift=true.
        // Shift should be stripped because it's encoded in the character.
        let result = translate_key(GodotKey::A, 'A' as u32, false, false, true, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('A'), Modifiers::NONE)));
    }

    #[test]
    fn at_sign_strips_shift() {
        // '@' = Shift+2 on US layout: unicode='@' (64), shift=true.
        let result = translate_key(GodotKey::KEY_2, '@' as u32, false, false, true, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)));
    }

    #[test]
    fn plain_lowercase_no_modifiers() {
        let result = translate_key(GodotKey::A, 'a' as u32, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('a'), Modifiers::NONE)));
    }

    #[test]
    fn shift_preserved_with_ctrl() {
        // Ctrl+Shift+printable: Shift should NOT be stripped when Ctrl is active.
        // This tests the unicode path — use a key that's not A-Z to avoid
        // hitting the Ctrl+letter branch. But actually Ctrl+Shift+A hits
        // the Ctrl+letter branch first. Let's test with alt+shift instead.
        let result = translate_key(GodotKey::A, 'A' as u32, false, true, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('A'),
                Modifiers::ALT | Modifiers::SHIFT
            ))
        );
    }

    // ── Bare modifier filtering ─────────────────────────────────────────

    #[test]
    fn bare_shift_returns_none() {
        assert_eq!(
            translate_key(GodotKey::SHIFT, 0, false, false, true, false),
            None
        );
    }

    #[test]
    fn bare_ctrl_returns_none() {
        assert_eq!(
            translate_key(GodotKey::CTRL, 0, true, false, false, false),
            None
        );
    }

    #[test]
    fn bare_alt_returns_none() {
        assert_eq!(
            translate_key(GodotKey::ALT, 0, false, true, false, false),
            None
        );
    }

    #[test]
    fn bare_meta_returns_none() {
        assert_eq!(
            translate_key(GodotKey::META, 0, false, false, false, true),
            None
        );
    }

    #[test]
    fn bare_capslock_returns_none() {
        assert_eq!(
            translate_key(GodotKey::CAPSLOCK, 0, false, false, false, false),
            None
        );
    }

    // ── Non-Latin keyboard layout (latin_key population) ────────────────

    #[test]
    fn non_ascii_with_latin_keycode_populates_latin_key() {
        // Russian 'о' (U+043E) on the physical J key
        let result = translate_key(
            GodotKey::J,          // keycode (Latin equivalent)
            0x043E,               // unicode (Cyrillic о)
            false, false, false, false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('\u{043E}'));
        assert_eq!(ke.latin_key(), Some(Key::Char('j')));
    }

    #[test]
    fn non_ascii_shifted_latin_key_uppercase() {
        // Russian 'О' (U+041E) with Shift on the physical J key
        let result = translate_key(
            GodotKey::J,          // keycode
            0x041E,               // unicode (Cyrillic О)
            false, false, true, false,  // shift=true
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('\u{041E}'));
        assert_eq!(ke.latin_key(), Some(Key::Char('J')));
    }

    #[test]
    fn ascii_char_no_latin_key() {
        let result = translate_key(
            GodotKey::J, 0x006A, // 'j'
            false, false, false, false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('j'));
        assert_eq!(ke.latin_key(), None);
    }

    #[test]
    fn ctrl_path_no_latin_key() {
        // Ctrl+J goes through the Ctrl path, not the unicode path
        let result = translate_key(
            GodotKey::J, 0x000A, // Ctrl+J unicode is control code
            true, false, false, false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('j'));
        assert!(ke.modifiers().contains(Modifiers::CTRL));
        assert_eq!(ke.latin_key(), None);
    }

    #[test]
    fn non_letter_keycode_no_latin_key() {
        // Non-ASCII char but keycode is not in A-Z range (e.g. a symbol key)
        let result = translate_key(
            GodotKey::KEY_4,    // keycode for '4' key
            0x003B,             // ';' (ASCII, would not trigger latin_key anyway)
            false, false, false, false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.latin_key(), None);
    }

    #[test]
    fn named_key_no_latin_key() {
        // Named keys go through get_named_key, not the unicode path
        let result = translate_key(
            GodotKey::UP, 0,
            false, false, false, false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Up);
        assert_eq!(ke.latin_key(), None);
    }

    // ── Zero unicode filtering ──────────────────────────────────────────

    #[test]
    fn zero_unicode_unknown_key_returns_none() {
        // An unrecognized key with no unicode representation.
        assert_eq!(
            translate_key(GodotKey::UNKNOWN, 0, false, false, false, false),
            None
        );
    }

    #[test]
    fn zero_unicode_with_keycode_not_named_returns_none() {
        // A key that isn't in the named table and has zero unicode.
        assert_eq!(
            translate_key(GodotKey::LAUNCHMAIL, 0, false, false, false, false),
            None
        );
    }

    // ── Modifier combination building ───────────────────────────────────

    #[test]
    fn all_modifiers_combined() {
        // Ctrl+Alt+Shift+Meta with a named key should produce all four flags.
        let result = translate_key(GodotKey::UP, 0, true, true, true, true);
        let expected_mods = Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META;
        assert_eq!(result, Some(KeyEvent::new(Key::Up, expected_mods)));
    }

    #[test]
    fn no_modifiers_produces_none_flags() {
        let result = translate_key(GodotKey::ENTER, 0, false, false, false, false);
        assert_eq!(result.unwrap().modifiers(), Modifiers::NONE);
    }
}
