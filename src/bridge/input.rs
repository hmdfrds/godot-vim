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
        GodotKey::TAB | GodotKey::BACKTAB => Some(Key::Tab),
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
        GodotKey::F13 => Some(Key::F(13)),
        GodotKey::F14 => Some(Key::F(14)),
        GodotKey::F15 => Some(Key::F(15)),
        GodotKey::F16 => Some(Key::F(16)),
        GodotKey::F17 => Some(Key::F(17)),
        GodotKey::F18 => Some(Key::F(18)),
        GodotKey::F19 => Some(Key::F(19)),
        GodotKey::F20 => Some(Key::F(20)),
        GodotKey::F21 => Some(Key::F(21)),
        GodotKey::F22 => Some(Key::F(22)),
        GodotKey::F23 => Some(Key::F(23)),
        GodotKey::F24 => Some(Key::F(24)),
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
        GodotKey::CLEAR => Some(Key::Char('5')),
        _ => None,
    }
}

/// Check if a character is a dead-key artifact that should be filtered.
///
/// Filters the Combining Diacritical Marks block (U+0300..U+036F) which
/// appears as garbage on X11 without XIM. The adjacent Spacing Modifier
/// Letters block (U+02C0..U+02FF) is NOT filtered — it contains legitimate
/// characters used in some languages (e.g., U+02BC ʻokina in Hawaiian).
fn is_dead_key_char(ch: char) -> bool {
    matches!(ch as u32, 0x0300..=0x036F)
}

/// Derive the US-QWERTY character from a physical keycode + shift state.
///
/// Returns `None` for non-printable or non-ASCII physical keys (F-keys,
/// arrow keys, modifier keys, media keys, etc.).
///
/// The shift table is hardcoded to US-QWERTY because Godot's
/// `get_physical_keycode()` is defined as the US-QWERTY scan code.
fn physical_to_ascii(physical: GodotKey, shift: bool) -> Option<char> {
    let code = physical.ord();
    // Letters: physical A-Z → 'a'-'z' or 'A'-'Z'
    let key_a = GodotKey::A.ord();
    let key_z = GodotKey::Z.ord();
    if (key_a..=key_z).contains(&code) {
        let ch = code as u8 as char;
        return Some(if shift {
            ch.to_ascii_uppercase()
        } else {
            ch.to_ascii_lowercase()
        });
    }
    // Digits and symbols: US-QWERTY shift map
    if shift {
        match physical {
            GodotKey::KEY_1 => Some('!'),
            GodotKey::KEY_2 => Some('@'),
            GodotKey::KEY_3 => Some('#'),
            GodotKey::KEY_4 => Some('$'),
            GodotKey::KEY_5 => Some('%'),
            GodotKey::KEY_6 => Some('^'),
            GodotKey::KEY_7 => Some('&'),
            GodotKey::KEY_8 => Some('*'),
            GodotKey::KEY_9 => Some('('),
            GodotKey::KEY_0 => Some(')'),
            GodotKey::MINUS => Some('_'),
            GodotKey::EQUAL => Some('+'),
            GodotKey::BRACKETLEFT => Some('{'),
            GodotKey::BRACKETRIGHT => Some('}'),
            GodotKey::BACKSLASH => Some('|'),
            GodotKey::SEMICOLON => Some(':'),
            GodotKey::APOSTROPHE => Some('"'),
            GodotKey::QUOTELEFT => Some('~'),
            GodotKey::COMMA => Some('<'),
            GodotKey::PERIOD => Some('>'),
            GodotKey::SLASH => Some('?'),
            GodotKey::SPACE => Some(' '),
            _ => None,
        }
    } else {
        // Unshifted: Godot Key enum values ARE ASCII codepoints
        let ch = char::from_u32(u32::try_from(code).ok()?)?;
        if ch.is_ascii_graphic() || ch == ' ' {
            Some(ch)
        } else {
            None
        }
    }
}

/// Resolve a Ctrl+key combination to a [`KeyEvent`].
///
/// Consolidates all Ctrl key resolution into a single function with a clear
/// 6-step priority chain. The physical keycode fallback (step 6) ensures
/// that Ctrl+symbol keys (Ctrl+[, Ctrl+], Ctrl+^, Ctrl+\) work correctly
/// on non-Latin keyboard layouts.
///
/// Priority:
/// 1. Ctrl+Space → '@' (terminal NUL convention)
/// 2. Logical letter A-Z → lowercase letter
/// 3. Physical letter A-Z → lowercase letter (non-Latin fallback)
/// 4. Ctrl+Shift + printable non-alpha unicode → unicode char (Shift stripped)
/// 5. Logical keycode is ASCII graphic → shifted symbol or lowercase (Shift stripped for symbols)
/// 6. Physical keycode via `physical_to_ascii` → US-QWERTY character
fn resolve_ctrl_key(
    keycode: GodotKey,
    physical_keycode: GodotKey,
    unicode: u32,
    modifiers: Modifiers,
) -> Option<KeyEvent> {
    // Step 1: Ctrl+Space → Ctrl+@ (terminal NUL / <C-@>).
    if keycode == GodotKey::SPACE {
        let mods = modifiers & !Modifiers::SHIFT;
        return Some(KeyEvent::new(Key::Char('@'), mods));
    }

    let key_a = GodotKey::A.ord();
    let key_z = GodotKey::Z.ord();

    // Step 2: Logical letter A-Z (works on Latin layouts).
    let key_val = keycode.ord();
    if (key_a..=key_z).contains(&key_val) {
        if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
            return Some(KeyEvent::new(Key::Char(ch.to_ascii_lowercase()), modifiers));
        }
        log::warn!(
            "resolve_ctrl_key: Ctrl+letter keycode={} char conversion failed",
            key_val
        );
        return None;
    }

    // Step 3: Physical letter A-Z (non-Latin layout fallback).
    let phys_val = physical_keycode.ord();
    if (key_a..=key_z).contains(&phys_val) {
        if let Some(ch) = u32::try_from(phys_val).ok().and_then(char::from_u32) {
            return Some(KeyEvent::new(Key::Char(ch.to_ascii_lowercase()), modifiers));
        }
    }

    // Step 4: Ctrl+Shift + printable non-alpha unicode (e.g. Ctrl+Shift+2 → '@').
    if modifiers.contains(Modifiers::SHIFT) && unicode != 0 {
        if let Some(ch) = char::from_u32(unicode) {
            if !ch.is_control() && !ch.is_ascii_alphabetic() {
                let mods = modifiers & !Modifiers::SHIFT;
                return Some(KeyEvent::new(Key::Char(ch), mods));
            }
        }
    }

    // Step 5: Logical keycode is ASCII graphic (e.g. Ctrl+[ on Latin layouts).
    // For symbols with Shift, derive the shifted character from physical keycode
    // and strip Shift — same convention as Step 6.
    if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
        if ch.is_ascii_graphic() {
            let has_shift = modifiers.contains(Modifiers::SHIFT);
            let (resolved_ch, mods) = if has_shift && !ch.is_ascii_alphabetic() {
                // Shifted symbol: derive from physical (e.g., Shift+[ → {).
                // Letters never reach here (Steps 2/3 catch them), but the
                // guard is defense-in-depth.
                let shifted = physical_to_ascii(physical_keycode, true).unwrap_or(ch);
                (shifted, modifiers & !Modifiers::SHIFT)
            } else {
                (ch.to_ascii_lowercase(), modifiers)
            };
            log::trace!(
                "resolve_ctrl_key: logical keycode {:?} shift={} -> Key::Char({:?})",
                keycode,
                has_shift,
                resolved_ch
            );
            return Some(KeyEvent::new(Key::Char(resolved_ch), mods));
        }
    }

    // Step 6: Physical keycode fallback (Ctrl+[ on non-Latin layouts).
    // Pass the actual shift state so Ctrl+Shift+6 → '^' (not '6').
    // Strip Shift from modifiers when it was used to derive the character,
    // since it is now encoded in the character itself ('^' vs '6').
    let has_shift = modifiers.contains(Modifiers::SHIFT);
    if let Some(ch) = physical_to_ascii(physical_keycode, has_shift) {
        if ch.is_ascii_graphic() {
            let mods = if has_shift {
                modifiers & !Modifiers::SHIFT
            } else {
                modifiers
            };
            log::trace!(
                "resolve_ctrl_key: physical fallback {:?} shift={} -> Key::Char({:?})",
                physical_keycode,
                has_shift,
                ch
            );
            return Some(KeyEvent::new(Key::Char(ch), mods));
        }
    }

    None
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
    physical_keycode: GodotKey,
    unicode: u32,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Option<KeyEvent> {
    // AltGr on Windows: Godot reports Ctrl+Alt simultaneously when IME is
    // inactive (Normal mode). If both Ctrl and Alt are set with no Meta,
    // and unicode is a printable non-control character, this is AltGr
    // producing a composed character — strip Ctrl and Alt so it enters the
    // normal printable path.
    let is_altgr = ctrl
        && alt
        && !meta
        && unicode != 0
        && char::from_u32(unicode).is_some_and(|c| c >= ' ' && !c.is_control());
    let (ctrl, alt) = if is_altgr {
        (false, false)
    } else {
        (ctrl, alt)
    };

    // Linux AltGr: reported as Alt-only (not Ctrl+Alt like Windows).
    // Detect by comparing unicode against the physical keycode's base character.
    // If Alt is the only modifier and the unicode differs from the physical key's
    // unshifted output, the Alt key is acting as a level-3 modifier (AltGr), not
    // as a genuine Alt press.
    let is_altgr_linux = !is_altgr
        && alt
        && !ctrl
        && !meta
        && unicode != 0
        && char::from_u32(unicode).is_some_and(|c| c >= ' ' && !c.is_control())
        && {
            let uc = char::from_u32(unicode).unwrap();
            let unshifted_match =
                physical_to_ascii(physical_keycode, false).is_some_and(|base| base == uc);
            let shifted_match =
                physical_to_ascii(physical_keycode, true).is_some_and(|base| base == uc);
            // Logical keycode cross-check: if the unicode matches the logical
            // keycode's character (case-insensitive), this is a genuine Alt
            // press, not AltGr. Fixes false positives on QWERTZ (Y↔Z) and
            // AZERTY (A↔Q, W↔Z) where the physical position differs from the
            // logical key.
            let logical_match = u32::try_from(keycode.ord())
                .ok()
                .and_then(char::from_u32)
                .is_some_and(|lc| lc.eq_ignore_ascii_case(&uc));
            !unshifted_match && !shifted_match && !logical_match
        };
    let is_altgr = is_altgr || is_altgr_linux;
    let alt = if is_altgr_linux { false } else { alt };

    // Build modifiers from (possibly AltGr-corrected) bool parameters.
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
            | GodotKey::NUMLOCK
            | GodotKey::SCROLLLOCK
    ) {
        log::trace!("parse_godot_key: filtered bare modifier {:?}", keycode);
        return None;
    }

    if let Some(key) = get_named_key(keycode) {
        // BACKTAB implies Shift — force it on even if the platform doesn't
        // report is_shift_pressed() for this keycode.
        if keycode == GodotKey::BACKTAB {
            modifiers |= Modifiers::SHIFT;
        }
        log::trace!("parse_godot_key: named key {} mods={}", key, modifiers);
        return Some(KeyEvent::new(key, modifiers));
    }

    // Ctrl+key resolution: unified function with physical keycode fallback.
    if modifiers.contains(Modifiers::CTRL) {
        if let Some(event) = resolve_ctrl_key(keycode, physical_keycode, unicode, modifiers) {
            return Some(event);
        }
    }

    if unicode == 0 {
        // macOS: Godot's TextEdit unconditionally re-enables im_active on every
        // redraw (text_edit.cpp _update_ime_window_position). With im_active=true,
        // the macOS keyDown handler sets unicode=0 and relies on interpretKeyEvents
        // → insertText for the character. On key repeat, the Press-and-Hold accent
        // system (ApplePressAndHoldEnabled, default since OS X Lion) can suppress
        // insertText, losing the unicode value entirely.
        //
        // Fall back to the physical keycode's US-QWERTY character. This is the same
        // fallback used for non-Latin layouts at line ~397 below.
        // See: https://github.com/hmdfrds/godot-vim/issues/33
        if let Some(ch) = physical_to_ascii(physical_keycode, shift) {
            log::trace!(
                "parse_godot_key: zero unicode for {:?}, physical fallback '{}'",
                keycode,
                ch
            );
            let mut mods = modifiers;
            if !is_altgr
                && !mods.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META)
            {
                mods &= !Modifiers::SHIFT;
            }
            return Some(KeyEvent::new(Key::Char(ch), mods));
        }
        log::trace!("parse_godot_key: zero unicode for {:?}", keycode);
        return None;
    }
    let ch = char::from_u32(unicode)?;
    if ch.is_control() {
        log::trace!("parse_godot_key: control char U+{:04X} filtered", unicode);
        return None;
    }
    if is_dead_key_char(ch) {
        log::trace!("parse_godot_key: dead key char U+{:04X} filtered", unicode);
        return None;
    }

    // For plain printable characters, Shift is already encoded in the unicode
    // value ('A' = Shift+a, '@' = Shift+2). Reporting Shift as a separate
    // modifier would cause the engine to see <S-A> instead of just 'A'.
    // Keep Shift only when combined with Ctrl/Alt/Meta (e.g. <C-S-f>);
    // named keys (<S-Tab>, <S-Left>) were already handled above.
    if !is_altgr && !modifiers.intersects(Modifiers::CTRL | Modifiers::ALT | Modifiers::META) {
        modifiers &= !Modifiers::SHIFT;
    }

    // When unicode produced a non-ASCII character, derive the Latin command
    // equivalent from the physical keycode's US-QWERTY position. This covers
    // ALL printable ASCII: letters, digits, and symbols.
    let event = KeyEvent::new(Key::Char(ch), modifiers);
    let event = if !ch.is_ascii() && !modifiers.contains(Modifiers::CTRL) {
        if let Some(latin_ch) = physical_to_ascii(physical_keycode, shift) {
            log::trace!(
                "parse_godot_key: non-Latin '{}' with Latin equivalent '{}'",
                ch,
                latin_ch
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
        event.get_physical_keycode(),
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
        let result = translate_key(
            GodotKey::ESCAPE,
            GodotKey::ESCAPE,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Escape, Modifiers::NONE)));
    }

    #[test]
    fn named_key_enter() {
        let result = translate_key(
            GodotKey::ENTER,
            GodotKey::ENTER,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Enter, Modifiers::NONE)));
    }

    #[test]
    fn named_key_kp_enter() {
        let result = translate_key(
            GodotKey::KP_ENTER,
            GodotKey::KP_ENTER,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Enter, Modifiers::NONE)));
    }

    #[test]
    fn named_key_backspace() {
        let result = translate_key(
            GodotKey::BACKSPACE,
            GodotKey::BACKSPACE,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Backspace, Modifiers::NONE)));
    }

    #[test]
    fn named_key_tab() {
        let result = translate_key(GodotKey::TAB, GodotKey::TAB, 0, false, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Tab, Modifiers::NONE)));
    }

    #[test]
    fn named_key_delete() {
        let result = translate_key(
            GodotKey::DELETE,
            GodotKey::DELETE,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Delete, Modifiers::NONE)));
    }

    #[test]
    fn named_key_insert() {
        let result = translate_key(
            GodotKey::INSERT,
            GodotKey::INSERT,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Insert, Modifiers::NONE)));
    }

    #[test]
    fn named_key_arrows() {
        assert_eq!(
            translate_key(GodotKey::UP, GodotKey::UP, 0, false, false, false, false),
            Some(KeyEvent::new(Key::Up, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::DOWN,
                GodotKey::DOWN,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Down, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::LEFT,
                GodotKey::LEFT,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Left, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::RIGHT,
                GodotKey::RIGHT,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Right, Modifiers::NONE))
        );
    }

    #[test]
    fn named_key_navigation() {
        assert_eq!(
            translate_key(
                GodotKey::HOME,
                GodotKey::HOME,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Home, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(GodotKey::END, GodotKey::END, 0, false, false, false, false),
            Some(KeyEvent::new(Key::End, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::PAGEUP,
                GodotKey::PAGEUP,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::PageUp, Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::PAGEDOWN,
                GodotKey::PAGEDOWN,
                0,
                false,
                false,
                false,
                false
            ),
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
                translate_key(*gk, *gk, 0, false, false, false, false),
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
                translate_key(gk, gk, 0, false, false, false, false),
                Some(KeyEvent::new(Key::Char(ch), Modifiers::NONE)),
                "KP_{ch} mapping failed"
            );
        }
    }

    #[test]
    fn named_key_keypad_operators() {
        assert_eq!(
            translate_key(
                GodotKey::KP_MULTIPLY,
                GodotKey::KP_MULTIPLY,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Char('*'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::KP_SUBTRACT,
                GodotKey::KP_SUBTRACT,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Char('-'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::KP_ADD,
                GodotKey::KP_ADD,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Char('+'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::KP_PERIOD,
                GodotKey::KP_PERIOD,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Char('.'), Modifiers::NONE))
        );
        assert_eq!(
            translate_key(
                GodotKey::KP_DIVIDE,
                GodotKey::KP_DIVIDE,
                0,
                false,
                false,
                false,
                false
            ),
            Some(KeyEvent::new(Key::Char('/'), Modifiers::NONE))
        );
    }

    // ── Named keys with modifiers ───────────────────────────────────────

    #[test]
    fn named_key_with_shift_preserves_shift() {
        // Shift+Tab should keep the Shift modifier (named keys don't strip it).
        let result = translate_key(GodotKey::TAB, GodotKey::TAB, 0, false, false, true, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Tab, Modifiers::SHIFT)));
    }

    #[test]
    fn backtab_maps_to_shift_tab() {
        // BACKTAB with shift=true (most platforms report Shift for Shift+Tab)
        let result = translate_key(
            GodotKey::BACKTAB,
            GodotKey::BACKTAB,
            0,
            false,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Tab, Modifiers::SHIFT)),
            "BACKTAB with shift=true should produce <S-Tab>"
        );
    }

    #[test]
    fn backtab_forces_shift_even_when_not_reported() {
        // BACKTAB without shift flag (some platforms don't report Shift for BACKTAB)
        let result = translate_key(
            GodotKey::BACKTAB,
            GodotKey::BACKTAB,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Tab, Modifiers::SHIFT)),
            "BACKTAB should force Shift even when is_shift_pressed() is false"
        );
    }

    #[test]
    fn backtab_with_ctrl() {
        let result = translate_key(
            GodotKey::BACKTAB,
            GodotKey::BACKTAB,
            0,
            true,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT)),
            "Ctrl+BACKTAB should produce <C-S-Tab>"
        );
    }

    #[test]
    fn named_key_with_ctrl() {
        let result = translate_key(GodotKey::LEFT, GodotKey::LEFT, 0, true, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Left, Modifiers::CTRL)));
    }

    // ── Ctrl+letter ─────────────────────────────────────────────────────

    #[test]
    fn ctrl_a() {
        // Ctrl+A: keycode = GodotKey::A, unicode = 1 (control code, ignored).
        let result = translate_key(GodotKey::A, GodotKey::A, 1, true, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('a'), Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_z() {
        let result = translate_key(GodotKey::Z, GodotKey::Z, 26, true, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('z'), Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_letter_is_lowercase() {
        // Even though keycode is uppercase 'A', result char should be 'a'.
        let result = translate_key(GodotKey::A, GodotKey::A, 1, true, false, false, false);
        let key = result.unwrap().key();
        assert_eq!(key, Key::Char('a'));
    }

    #[test]
    fn ctrl_shift_letter() {
        // Ctrl+Shift+A should produce Key::Char('a') with CTRL|SHIFT.
        let result = translate_key(GodotKey::A, GodotKey::A, 1, true, false, true, false);
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
        let result = translate_key(
            GodotKey::KEY_2,
            GodotKey::KEY_2,
            0x40,
            true,
            false,
            true,
            false,
        );
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
        let result = translate_key(
            GodotKey::KEY_6,
            GodotKey::KEY_6,
            0x5E,
            true,
            false,
            true,
            false,
        );
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
        let result = translate_key(
            GodotKey::BRACKETLEFT,
            GodotKey::BRACKETLEFT,
            0x1B,
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('['), Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_close_bracket() {
        let result = translate_key(
            GodotKey::BRACKETRIGHT,
            GodotKey::BRACKETRIGHT,
            0x1D,
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char(']'), Modifiers::CTRL)));
    }

    // ── Printable chars with shift stripping ────────────────────────────

    #[test]
    fn uppercase_a_strips_shift() {
        // Typing 'A' (Shift+a): unicode='A' (65), shift=true.
        // Shift should be stripped because it's encoded in the character.
        let result = translate_key(
            GodotKey::A,
            GodotKey::A,
            'A' as u32,
            false,
            false,
            true,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('A'), Modifiers::NONE)));
    }

    #[test]
    fn at_sign_strips_shift() {
        // '@' = Shift+2 on US layout: unicode='@' (64), shift=true.
        let result = translate_key(
            GodotKey::KEY_2,
            GodotKey::KEY_2,
            '@' as u32,
            false,
            false,
            true,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)));
    }

    #[test]
    fn plain_lowercase_no_modifiers() {
        let result = translate_key(
            GodotKey::A,
            GodotKey::A,
            'a' as u32,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('a'), Modifiers::NONE)));
    }

    #[test]
    fn shift_preserved_with_ctrl() {
        // Ctrl+Shift+printable: Shift should NOT be stripped when Ctrl is active.
        // This tests the unicode path — use a key that's not A-Z to avoid
        // hitting the Ctrl+letter branch. But actually Ctrl+Shift+A hits
        // the Ctrl+letter branch first. Let's test with alt+shift instead.
        let result = translate_key(
            GodotKey::A,
            GodotKey::A,
            'A' as u32,
            false,
            true,
            true,
            false,
        );
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
            translate_key(
                GodotKey::SHIFT,
                GodotKey::SHIFT,
                0,
                false,
                false,
                true,
                false
            ),
            None
        );
    }

    #[test]
    fn bare_ctrl_returns_none() {
        assert_eq!(
            translate_key(GodotKey::CTRL, GodotKey::CTRL, 0, true, false, false, false),
            None
        );
    }

    #[test]
    fn bare_alt_returns_none() {
        assert_eq!(
            translate_key(GodotKey::ALT, GodotKey::ALT, 0, false, true, false, false),
            None
        );
    }

    #[test]
    fn bare_meta_returns_none() {
        assert_eq!(
            translate_key(GodotKey::META, GodotKey::META, 0, false, false, false, true),
            None
        );
    }

    #[test]
    fn bare_capslock_returns_none() {
        assert_eq!(
            translate_key(
                GodotKey::CAPSLOCK,
                GodotKey::CAPSLOCK,
                0,
                false,
                false,
                false,
                false
            ),
            None
        );
    }

    // ── Non-Latin keyboard layout (latin_key population) ────────────────

    #[test]
    fn non_ascii_with_latin_keycode_populates_latin_key() {
        // Russian 'о' (U+043E) on the physical J key
        let result = translate_key(
            GodotKey::J, // keycode (Latin equivalent)
            GodotKey::J, // physical keycode
            0x043E,      // unicode (Cyrillic о)
            false,
            false,
            false,
            false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('\u{043E}'));
        assert_eq!(ke.latin_key(), Some(Key::Char('j')));
    }

    #[test]
    fn non_ascii_shifted_latin_key_uppercase() {
        // Russian 'О' (U+041E) with Shift on the physical J key
        let result = translate_key(
            GodotKey::J, // keycode
            GodotKey::J, // physical keycode
            0x041E,      // unicode (Cyrillic О)
            false,
            false,
            true,
            false, // shift=true
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('\u{041E}'));
        assert_eq!(ke.latin_key(), Some(Key::Char('J')));
    }

    #[test]
    fn ascii_char_no_latin_key() {
        let result = translate_key(
            GodotKey::J,
            GodotKey::J,
            0x006A, // 'j'
            false,
            false,
            false,
            false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Char('j'));
        assert_eq!(ke.latin_key(), None);
    }

    #[test]
    fn ctrl_path_no_latin_key() {
        // Ctrl+J goes through the Ctrl path, not the unicode path
        let result = translate_key(
            GodotKey::J,
            GodotKey::J,
            0x000A, // Ctrl+J unicode is control code
            true,
            false,
            false,
            false,
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
            GodotKey::KEY_4,
            GodotKey::KEY_4, // keycode + physical
            0x003B,          // ';' (ASCII, would not trigger latin_key anyway)
            false,
            false,
            false,
            false,
        );
        let ke = result.unwrap();
        assert_eq!(ke.latin_key(), None);
    }

    #[test]
    fn named_key_no_latin_key() {
        // Named keys go through get_named_key, not the unicode path
        let result = translate_key(GodotKey::UP, GodotKey::UP, 0, false, false, false, false);
        let ke = result.unwrap();
        assert_eq!(ke.key(), Key::Up);
        assert_eq!(ke.latin_key(), None);
    }

    // ── Zero unicode filtering ──────────────────────────────────────────

    #[test]
    fn zero_unicode_unknown_key_returns_none() {
        // An unrecognized key with no unicode representation.
        assert_eq!(
            translate_key(
                GodotKey::UNKNOWN,
                GodotKey::UNKNOWN,
                0,
                false,
                false,
                false,
                false
            ),
            None
        );
    }

    #[test]
    fn zero_unicode_with_keycode_not_named_returns_none() {
        // A key that isn't in the named table and has zero unicode.
        assert_eq!(
            translate_key(
                GodotKey::LAUNCHMAIL,
                GodotKey::LAUNCHMAIL,
                0,
                false,
                false,
                false,
                false
            ),
            None
        );
    }

    #[test]
    fn zero_unicode_printable_key_falls_back_to_physical() {
        // macOS key repeat with im_active=true: unicode=0 but physical keycode
        // is a valid printable key. Should fall back to physical_to_ascii.
        // https://github.com/hmdfrds/godot-vim/issues/33
        let result = translate_key(GodotKey::J, GodotKey::J, 0, false, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('j'), Modifiers::NONE)),
            "zero unicode with printable physical key should use physical fallback"
        );
    }

    #[test]
    fn zero_unicode_physical_fallback_all_motion_keys() {
        // Verify h, j, k, l all recover via physical fallback.
        for (key, ch) in [
            (GodotKey::H, 'h'),
            (GodotKey::J, 'j'),
            (GodotKey::K, 'k'),
            (GodotKey::L, 'l'),
        ] {
            let result = translate_key(key, key, 0, false, false, false, false);
            assert_eq!(
                result,
                Some(KeyEvent::new(Key::Char(ch), Modifiers::NONE)),
                "zero unicode for {:?} should produce '{}'",
                key,
                ch
            );
        }
    }

    #[test]
    fn zero_unicode_physical_fallback_shifted() {
        // Shift+J with unicode=0: physical fallback produces 'J'.
        let result = translate_key(GodotKey::J, GodotKey::J, 0, false, false, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('J'), Modifiers::NONE)),
            "zero unicode shifted J should produce 'J' without Shift modifier (shift is in the char)"
        );
    }

    #[test]
    fn zero_unicode_physical_fallback_digit() {
        // Digit keys also recover.
        let result = translate_key(
            GodotKey::KEY_5,
            GodotKey::KEY_5,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('5'), Modifiers::NONE)),
            "zero unicode for digit key should use physical fallback"
        );
    }

    #[test]
    fn f13_through_f24_translated() {
        let cases = [
            (GodotKey::F13, 13),
            (GodotKey::F14, 14),
            (GodotKey::F15, 15),
            (GodotKey::F16, 16),
            (GodotKey::F17, 17),
            (GodotKey::F18, 18),
            (GodotKey::F19, 19),
            (GodotKey::F20, 20),
            (GodotKey::F21, 21),
            (GodotKey::F22, 22),
            (GodotKey::F23, 23),
            (GodotKey::F24, 24),
        ];
        for (godot_key, num) in cases {
            let result = translate_key(godot_key, godot_key, 0, false, false, false, false);
            assert_eq!(
                result,
                Some(KeyEvent::new(Key::F(num), Modifiers::NONE)),
                "F{num} should translate correctly"
            );
        }
    }

    // ── Dead key filtering ──────────────────────────────────────────────

    #[test]
    fn combining_acute_accent_filtered() {
        let result = translate_key(
            GodotKey::NONE,
            GodotKey::NONE,
            0x0301,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, None, "combining acute accent should be filtered");
    }

    #[test]
    fn combining_diaeresis_filtered() {
        let result = translate_key(
            GodotKey::NONE,
            GodotKey::NONE,
            0x0308,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, None, "combining diaeresis should be filtered");
    }

    #[test]
    fn spacing_modifier_circumflex_not_filtered() {
        // U+02C6 is in Spacing Modifier Letters block (no longer filtered).
        let result = translate_key(
            GodotKey::NONE,
            GodotKey::NONE,
            0x02C6,
            false,
            false,
            false,
            false,
        );
        assert!(
            result.is_some(),
            "modifier circumflex should NOT be filtered"
        );
        assert_eq!(result.unwrap().key(), Key::Char('\u{02C6}'));
    }

    #[test]
    fn normal_e_acute_not_filtered() {
        let result = translate_key(
            GodotKey::NONE,
            GodotKey::NONE,
            0x00E9,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_some(), "composed é should NOT be filtered");
        assert_eq!(result.unwrap().key(), Key::Char('é'));
    }

    #[test]
    fn backtick_not_filtered() {
        let result = translate_key(
            GodotKey::QUOTELEFT,
            GodotKey::QUOTELEFT,
            0x60,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_some(), "backtick should NOT be filtered");
        assert_eq!(result.unwrap().key(), Key::Char('`'));
    }

    #[test]
    fn tilde_char_not_filtered() {
        let result = translate_key(
            GodotKey::ASCIITILDE,
            GodotKey::ASCIITILDE,
            0x7E,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_some(), "tilde should NOT be filtered");
    }

    // ── Modifier combination building ───────────────────────────────────

    #[test]
    fn all_modifiers_combined() {
        // Ctrl+Alt+Shift+Meta with a named key should produce all four flags.
        let result = translate_key(GodotKey::UP, GodotKey::UP, 0, true, true, true, true);
        let expected_mods = Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META;
        assert_eq!(result, Some(KeyEvent::new(Key::Up, expected_mods)));
    }

    #[test]
    fn no_modifiers_produces_none_flags() {
        let result = translate_key(
            GodotKey::ENTER,
            GodotKey::ENTER,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result.unwrap().modifiers(), Modifiers::NONE);
    }

    // ── AltGr detection ────────────────────────────────────────────────

    #[test]
    fn altgr_with_printable_unicode_strips_ctrl_alt() {
        // AltGr+Q on German keyboard: Godot reports ctrl=true, alt=true,
        // unicode='@'. Should strip both Ctrl and Alt so '@' enters normally.
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            true,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)),
            "AltGr with printable unicode should strip Ctrl+Alt"
        );
    }

    #[test]
    fn altgr_with_shift_preserves_shift() {
        // AltGr+Shift+key: should strip Ctrl+Alt but preserve Shift.
        // The is_altgr flag prevents Shift from being stripped in the
        // printable path, so the engine sees <S-@> for mapping purposes.
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            true,
            true,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('@'), Modifiers::SHIFT)),
            "AltGr+Shift should preserve Shift modifier"
        );
    }

    #[test]
    fn real_ctrl_alt_with_zero_unicode_preserved() {
        // Real Ctrl+Alt+Q (not AltGr): unicode=0, ctrl=true, alt=true.
        // Should NOT trigger AltGr detection.
        let result = translate_key(GodotKey::Q, GodotKey::Q, 0, true, true, false, false);
        // Falls through to Ctrl+letter path → Key::Char('q') with CTRL|ALT
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('q'),
                Modifiers::CTRL | Modifiers::ALT
            )),
            "Real Ctrl+Alt with zero unicode should preserve both flags"
        );
    }

    #[test]
    fn ctrl_alt_meta_not_altgr() {
        // Ctrl+Alt+Meta is never AltGr — Meta disqualifies.
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            true,
            true,
            false,
            true,
        );
        // Has Meta → not AltGr. Named-key path doesn't match Q.
        // Ctrl path fires: Ctrl+letter → Key::Char('q') with CTRL|ALT|META
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('q'),
                Modifiers::CTRL | Modifiers::ALT | Modifiers::META
            )),
            "Ctrl+Alt+Meta should not be treated as AltGr"
        );
    }

    #[test]
    fn ctrl_only_with_printable_not_altgr() {
        // Only Ctrl (no Alt) with printable unicode — not AltGr.
        let result = translate_key(GodotKey::A, GodotKey::A, 1, true, false, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('a'), Modifiers::CTRL)),
            "Ctrl-only should not trigger AltGr detection"
        );
    }

    #[test]
    fn alt_only_with_printable_not_altgr() {
        // Only Alt (no Ctrl) with printable unicode — not AltGr.
        let result = translate_key(
            GodotKey::A,
            GodotKey::A,
            'a' as u32,
            false,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('a'), Modifiers::ALT)),
            "Alt-only should not trigger AltGr detection"
        );
    }

    #[test]
    fn altgr_with_euro_sign() {
        // AltGr+E on German keyboard → '€' (U+20AC)
        let result = translate_key(GodotKey::E, GodotKey::E, 0x20AC, true, true, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('€'), Modifiers::NONE)),
            "AltGr+E producing euro sign should work"
        );
    }

    #[test]
    fn altgr_with_control_char_not_stripped() {
        // Ctrl+Alt with a control character unicode — not AltGr
        // (AltGr always produces printable characters).
        let result = translate_key(GodotKey::A, GodotKey::A, 1, true, true, false, false);
        // Unicode 1 is a control char → AltGr check fails.
        // Falls to Ctrl+letter path.
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('a'),
                Modifiers::CTRL | Modifiers::ALT
            )),
            "Ctrl+Alt with control char should preserve flags"
        );
    }

    // ── Linux AltGr (Alt-only) ────────────────────────────────────────────

    #[test]
    fn linux_altgr_strips_alt_when_unicode_differs_from_physical() {
        // German QWERTZ: AltGr+Q → '@', physical Q → 'q'
        // Linux reports alt=true, ctrl=false
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            false,
            true,
            false,
            false, // alt only
        );
        // Alt should be stripped — '@' is the AltGr character, not Alt+@
        assert_eq!(result, Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)));
    }

    #[test]
    fn linux_altgr_preserves_alt_when_unicode_matches_physical() {
        // Genuine Alt+Q on US keyboard: unicode='q', physical Q → 'q'
        // The unicode matches the physical base, so this is real Alt, not AltGr
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            'q' as u32,
            false,
            true,
            false,
            false, // alt only
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('q'), Modifiers::ALT)));
    }

    #[test]
    fn linux_altgr_preserves_alt_when_unicode_is_zero() {
        // Alt+key with no unicode: not AltGr. Physical fallback recovers 'q'.
        let result = translate_key(GodotKey::Q, GodotKey::Q, 0, false, true, false, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('q'), Modifiers::ALT)),
            "Alt + zero unicode should use physical fallback and preserve Alt"
        );
    }

    #[test]
    fn linux_altgr_with_shift_preserves_shift() {
        // AltGr+Shift+key on Linux: produces different char, Shift should be kept
        let result = translate_key(
            GodotKey::KEY_7,
            GodotKey::KEY_7,
            '{' as u32,
            false,
            true,
            true,
            false, // alt + shift
        );
        // Alt stripped (AltGr detected), Shift preserved by is_altgr guard
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('{'), Modifiers::SHIFT))
        );
    }

    #[test]
    fn linux_altgr_curly_brace() {
        // German QWERTZ: AltGr+7 → '{', physical 7 → '7'
        let result = translate_key(
            GodotKey::KEY_7,
            GodotKey::KEY_7,
            '{' as u32,
            false,
            true,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('{'), Modifiers::NONE)));
    }

    #[test]
    fn linux_altgr_backslash() {
        // German QWERTZ: AltGr+ß → '\', physical MINUS → '-'
        let result = translate_key(
            GodotKey::MINUS,
            GodotKey::MINUS,
            '\\' as u32,
            false,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('\\'), Modifiers::NONE))
        );
    }

    #[test]
    fn linux_altgr_not_triggered_without_alt() {
        // No alt pressed at all — plain character
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            'q' as u32,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('q'), Modifiers::NONE)));
    }

    #[test]
    fn linux_altgr_not_triggered_when_ctrl_also_present() {
        // Ctrl+Alt → Windows AltGr path, not Linux AltGr
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            true,
            true,
            false,
            false, // ctrl+alt
        );
        // Windows AltGr detection handles this — both Ctrl and Alt stripped
        assert_eq!(result, Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)));
    }

    // ── Physical keycode to ASCII ──────────────────────────────────────

    #[test]
    fn physical_to_ascii_unshifted_letters() {
        for (gk, expected) in [
            (GodotKey::A, 'a'),
            (GodotKey::B, 'b'),
            (GodotKey::Z, 'z'),
            (GodotKey::M, 'm'),
        ] {
            assert_eq!(
                physical_to_ascii(gk, false),
                Some(expected),
                "unshifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_shifted_letters() {
        for (gk, expected) in [(GodotKey::A, 'A'), (GodotKey::Z, 'Z'), (GodotKey::M, 'M')] {
            assert_eq!(
                physical_to_ascii(gk, true),
                Some(expected),
                "shifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_unshifted_digits() {
        for (gk, expected) in [
            (GodotKey::KEY_0, '0'),
            (GodotKey::KEY_1, '1'),
            (GodotKey::KEY_5, '5'),
            (GodotKey::KEY_9, '9'),
        ] {
            assert_eq!(
                physical_to_ascii(gk, false),
                Some(expected),
                "unshifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_shifted_digits() {
        for (gk, expected) in [
            (GodotKey::KEY_1, '!'),
            (GodotKey::KEY_2, '@'),
            (GodotKey::KEY_3, '#'),
            (GodotKey::KEY_4, '$'),
            (GodotKey::KEY_5, '%'),
            (GodotKey::KEY_6, '^'),
            (GodotKey::KEY_7, '&'),
            (GodotKey::KEY_8, '*'),
            (GodotKey::KEY_9, '('),
            (GodotKey::KEY_0, ')'),
        ] {
            assert_eq!(
                physical_to_ascii(gk, true),
                Some(expected),
                "shifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_unshifted_symbols() {
        for (gk, expected) in [
            (GodotKey::MINUS, '-'),
            (GodotKey::EQUAL, '='),
            (GodotKey::BRACKETLEFT, '['),
            (GodotKey::BRACKETRIGHT, ']'),
            (GodotKey::BACKSLASH, '\\'),
            (GodotKey::SEMICOLON, ';'),
            (GodotKey::APOSTROPHE, '\''),
            (GodotKey::QUOTELEFT, '`'),
            (GodotKey::COMMA, ','),
            (GodotKey::PERIOD, '.'),
            (GodotKey::SLASH, '/'),
        ] {
            assert_eq!(
                physical_to_ascii(gk, false),
                Some(expected),
                "unshifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_shifted_symbols() {
        for (gk, expected) in [
            (GodotKey::MINUS, '_'),
            (GodotKey::EQUAL, '+'),
            (GodotKey::BRACKETLEFT, '{'),
            (GodotKey::BRACKETRIGHT, '}'),
            (GodotKey::BACKSLASH, '|'),
            (GodotKey::SEMICOLON, ':'),
            (GodotKey::APOSTROPHE, '"'),
            (GodotKey::QUOTELEFT, '~'),
            (GodotKey::COMMA, '<'),
            (GodotKey::PERIOD, '>'),
            (GodotKey::SLASH, '?'),
        ] {
            assert_eq!(
                physical_to_ascii(gk, true),
                Some(expected),
                "shifted {:?} should produce '{}'",
                gk,
                expected
            );
        }
    }

    #[test]
    fn physical_to_ascii_space() {
        assert_eq!(physical_to_ascii(GodotKey::SPACE, false), Some(' '));
        assert_eq!(physical_to_ascii(GodotKey::SPACE, true), Some(' '));
    }

    #[test]
    fn physical_to_ascii_non_printable_returns_none() {
        assert_eq!(physical_to_ascii(GodotKey::ESCAPE, false), None);
        assert_eq!(physical_to_ascii(GodotKey::F1, false), None);
        assert_eq!(physical_to_ascii(GodotKey::UP, false), None);
        assert_eq!(physical_to_ascii(GodotKey::SHIFT, false), None);
        assert_eq!(physical_to_ascii(GodotKey::CTRL, false), None);
        assert_eq!(physical_to_ascii(GodotKey::TAB, false), None);
        assert_eq!(physical_to_ascii(GodotKey::ENTER, false), None);
    }

    // ── Physical keycode latin_key for symbols ─────────────────────────

    #[test]
    fn non_latin_period_gets_latin_key_from_physical() {
        // Russian layout: physical '.' key produces 'ю' (U+044E)
        let result = translate_key(
            GodotKey::PERIOD,
            GodotKey::PERIOD,
            0x044E,
            false,
            false,
            false,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('ю'));
        assert_eq!(event.latin_key(), Some(Key::Char('.')));
    }

    #[test]
    fn non_latin_semicolon_gets_latin_key_from_physical() {
        // Russian layout: physical ';' key produces 'ж' (U+0436)
        let result = translate_key(
            GodotKey::SEMICOLON,
            GodotKey::SEMICOLON,
            0x0436,
            false,
            false,
            false,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('ж'));
        assert_eq!(event.latin_key(), Some(Key::Char(';')));
    }

    #[test]
    fn non_latin_shifted_gets_shifted_latin_key() {
        // Russian: Shift+physical ';' → 'Ж', shifted US-QWERTY = ':'
        let result = translate_key(
            GodotKey::SEMICOLON,
            GodotKey::SEMICOLON,
            0x0416,
            false,
            false,
            true,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('Ж'));
        assert_eq!(event.latin_key(), Some(Key::Char(':')));
    }

    #[test]
    fn non_latin_slash_gets_latin_key() {
        // Greek layout: physical '/' produces 'ς' (U+03C2)
        let result = translate_key(
            GodotKey::SLASH,
            GodotKey::SLASH,
            0x03C2,
            false,
            false,
            false,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('ς'));
        assert_eq!(event.latin_key(), Some(Key::Char('/')));
    }

    #[test]
    fn ascii_unicode_does_not_get_latin_key() {
        // English layout: '.' produces '.' — already ASCII
        let result = translate_key(
            GodotKey::PERIOD,
            GodotKey::PERIOD,
            0x002E,
            false,
            false,
            false,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('.'));
        assert_eq!(event.latin_key(), None);
    }

    #[test]
    fn non_latin_letter_still_gets_latin_key() {
        // Russian: physical 'j' → 'о' (U+043E)
        let result = translate_key(GodotKey::J, GodotKey::J, 0x043E, false, false, false, false);
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('о'));
        assert_eq!(event.latin_key(), Some(Key::Char('j')));
    }

    #[test]
    fn ctrl_path_unaffected_by_physical_keycode() {
        let result = translate_key(GodotKey::J, GodotKey::J, 0x000A, true, false, false, false);
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('j'));
        assert_eq!(event.modifiers(), Modifiers::CTRL);
        assert_eq!(event.latin_key(), None);
    }

    // ── AltGr Shift preservation ───────────────────────────────────────

    #[test]
    fn altgr_shift_preserves_shift_modifier() {
        // AltGr+Shift: ctrl=true, alt=true, shift=true, unicode='@'
        let result = translate_key(
            GodotKey::KEY_2,
            GodotKey::KEY_2,
            '@' as u32,
            true,
            true,
            true,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('@'));
        assert!(
            event.modifiers().contains(Modifiers::SHIFT),
            "AltGr+Shift should preserve Shift"
        );
    }

    #[test]
    fn non_altgr_shift_still_stripped_for_printable() {
        // Regular Shift+'a' = 'A', Shift should be stripped
        let result = translate_key(
            GodotKey::A,
            GodotKey::A,
            'A' as u32,
            false,
            false,
            true,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event.key(), Key::Char('A'));
        assert!(
            !event.modifiers().contains(Modifiers::SHIFT),
            "Regular Shift should be stripped for printable"
        );
    }

    // ── Narrowed dead key filter ───────────────────────────────────────

    #[test]
    fn okina_not_filtered() {
        // U+02BC (modifier letter apostrophe / ʻokina) should NOT be filtered
        let result = translate_key(
            GodotKey::APOSTROPHE,
            GodotKey::APOSTROPHE,
            0x02BC,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_some(), "U+02BC should not be filtered");
        assert_eq!(result.unwrap().key(), Key::Char('\u{02BC}'));
    }

    #[test]
    fn combining_grave_still_filtered() {
        // U+0300 (combining grave accent) should still be filtered
        let result = translate_key(
            GodotKey::QUOTELEFT,
            GodotKey::QUOTELEFT,
            0x0300,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_none(), "U+0300 should be filtered");
    }

    #[test]
    fn combining_end_still_filtered() {
        // U+036F should still be filtered
        let result = translate_key(
            GodotKey::QUOTELEFT,
            GodotKey::QUOTELEFT,
            0x036F,
            false,
            false,
            false,
            false,
        );
        assert!(result.is_none(), "U+036F should be filtered");
    }

    // ── Ctrl+Space → Ctrl+@ ───────────────────────────────────────────

    #[test]
    fn ctrl_space_maps_to_ctrl_at() {
        let result = translate_key(
            GodotKey::SPACE,
            GodotKey::SPACE,
            0,
            true,
            false,
            false,
            false,
        );
        let event = result.unwrap();
        assert_eq!(event, KeyEvent::ctrl('@'));
    }

    // ── Ctrl+Space modifier preservation ──────────────────────────────────

    #[test]
    fn ctrl_alt_space_preserves_alt() {
        let result = translate_key(
            GodotKey::SPACE,
            GodotKey::SPACE,
            0,
            true,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('@'),
                Modifiers::CTRL | Modifiers::ALT
            ))
        );
    }

    #[test]
    fn ctrl_meta_space_preserves_meta() {
        let result = translate_key(
            GodotKey::SPACE,
            GodotKey::SPACE,
            0,
            true,
            false,
            false,
            true,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(
                Key::Char('@'),
                Modifiers::CTRL | Modifiers::META
            ))
        );
    }

    #[test]
    fn ctrl_shift_space_strips_shift() {
        let result = translate_key(
            GodotKey::SPACE,
            GodotKey::SPACE,
            0,
            true,
            false,
            true,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('@'), Modifiers::CTRL)));
    }

    /// Drift guard: the completion trigger in `controller::completion` checks
    /// for Ctrl+@ (Key::Char('@') with CTRL). This test verifies that the
    /// bridge's Ctrl+Space translation produces exactly that KeyEvent.
    /// If someone changes resolve_ctrl_key's Step 1, this test will break
    /// and signal that completion.rs must be updated to match.
    #[test]
    fn ctrl_space_bridge_output_matches_completion_trigger() {
        let event = translate_key(
            GodotKey::SPACE,
            GodotKey::SPACE,
            0,
            true,
            false,
            false,
            false,
        )
        .unwrap();
        // This is the exact condition from try_handle_completion.
        // If this assertion fails, update completion.rs to match.
        assert!(
            event.modifiers().contains(Modifiers::CTRL) && event.key() == Key::Char('@'),
            "Bridge Ctrl+Space output changed! Was {:?} — update completion.rs \
             try_handle_completion to match the new post-translation form",
            event,
        );
    }

    // ── CLEAR key ─────────────────────────────────────────────────────────

    #[test]
    fn clear_key_maps_to_five() {
        let result = translate_key(
            GodotKey::CLEAR,
            GodotKey::CLEAR,
            0,
            false,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('5'), Modifiers::NONE)));
    }

    // ── Ctrl+letter physical keycode fallback ─────────────────────────────

    #[test]
    fn ctrl_letter_physical_fallback_when_logical_is_non_latin() {
        // Simulate Russian layout: logical keycode is outside A-Z range,
        // but physical keycode is GodotKey::A (the physical key position)
        let non_latin_keycode = GodotKey::from_ord(0x0444); // Cyrillic ф
        let result = translate_key(
            non_latin_keycode,
            GodotKey::A,
            0x01, // control code for Ctrl+A
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('a'), Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_letter_logical_still_works_when_latin() {
        let result = translate_key(GodotKey::A, GodotKey::A, 0x01, true, false, false, false);
        assert_eq!(result, Some(KeyEvent::new(Key::Char('a'), Modifiers::CTRL)));
    }

    #[test]
    fn ctrl_letter_physical_fallback_z() {
        let non_latin_keycode = GodotKey::from_ord(0x044F); // Cyrillic я
        let result = translate_key(
            non_latin_keycode,
            GodotKey::Z,
            0x1A, // control code for Ctrl+Z
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('z'), Modifiers::CTRL)));
    }

    // ── Ctrl+symbol on non-Latin layouts ───────────────────────────────────

    #[test]
    fn ctrl_bracket_left_non_latin_physical_fallback() {
        // Russian layout: logical keycode is Cyrillic х (U+0445),
        // physical keycode is BRACKETLEFT. Should resolve to Ctrl+[.
        let cyrillic_kc = GodotKey::from_ord(0x0445);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::BRACKETLEFT,
            0x1B, // Ctrl+[ produces ESC control code
            true,
            false,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('['), Modifiers::CTRL)),
            "Ctrl+[ on non-Latin should resolve via physical keycode"
        );
    }

    #[test]
    fn ctrl_bracket_right_non_latin_physical_fallback() {
        let cyrillic_kc = GodotKey::from_ord(0x044A);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::BRACKETRIGHT,
            0x1D,
            true,
            false,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char(']'), Modifiers::CTRL)),
            "Ctrl+] on non-Latin should resolve via physical keycode"
        );
    }

    #[test]
    fn ctrl_caret_non_latin_physical_fallback() {
        // Ctrl+6 (without Shift) on non-Latin: produces Ctrl+'6', not Ctrl+'^'.
        // To get <C-^> (alternate file), the user must press Ctrl+Shift+6.
        let cyrillic_kc = GodotKey::from_ord(0x0447);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::KEY_6,
            0x1E,
            true,
            false,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('6'), Modifiers::CTRL)),
            "Ctrl+6 (no Shift) on non-Latin should produce Ctrl+'6'"
        );
    }

    #[test]
    fn ctrl_shift_6_non_latin_produces_caret() {
        let cyrillic_kc = GodotKey::from_ord(0x0447);
        let result = translate_key(cyrillic_kc, GodotKey::KEY_6, 0x1E, true, false, true, false);
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('^'), Modifiers::CTRL)),
            "Ctrl+Shift+6 on non-Latin should produce Ctrl+'^' with Shift stripped"
        );
    }

    #[test]
    fn ctrl_shift_bracket_non_latin_produces_brace() {
        let cyrillic_kc = GodotKey::from_ord(0x0445);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::BRACKETLEFT,
            0x1B,
            true,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('{'), Modifiers::CTRL)),
            "Ctrl+Shift+[ on non-Latin should produce Ctrl+'{{' with Shift stripped"
        );
    }

    #[test]
    fn ctrl_bracket_left_latin_still_works() {
        let result = translate_key(
            GodotKey::BRACKETLEFT,
            GodotKey::BRACKETLEFT,
            0x1B,
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('['), Modifiers::CTRL)),);
    }

    #[test]
    fn ctrl_shift_bracket_left_latin_produces_brace() {
        let result = translate_key(
            GodotKey::BRACKETLEFT,
            GodotKey::BRACKETLEFT,
            0x1B,
            true,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('{'), Modifiers::CTRL)),
            "Ctrl+Shift+[ on Latin should produce Ctrl+'{{' with Shift stripped"
        );
    }

    #[test]
    fn ctrl_shift_bracket_right_latin_produces_brace() {
        let result = translate_key(
            GodotKey::BRACKETRIGHT,
            GodotKey::BRACKETRIGHT,
            0x1D,
            true,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('}'), Modifiers::CTRL)),
            "Ctrl+Shift+] on Latin should produce Ctrl+'}}' with Shift stripped"
        );
    }

    #[test]
    fn ctrl_bracket_left_latin_no_shift_unchanged() {
        let result = translate_key(
            GodotKey::BRACKETLEFT,
            GodotKey::BRACKETLEFT,
            0x1B,
            true,
            false,
            false,
            false,
        );
        assert_eq!(result, Some(KeyEvent::new(Key::Char('['), Modifiers::CTRL)),);
    }

    // ── Linux AltGr: QWERTZ/AZERTY false positive fixes ───────────────────

    #[test]
    fn linux_altgr_qwertz_alt_z_preserves_alt() {
        // QWERTZ: Alt+Z. Physical=Y (swapped), Logical=Z, Unicode='z'.
        // Should NOT trigger AltGr — this is a genuine Alt press.
        let result = translate_key(
            GodotKey::Z, // logical keycode (layout says Z)
            GodotKey::Y, // physical keycode (Y position on QWERTZ)
            'z' as u32,  // unicode matches logical key
            false,       // ctrl
            true,        // alt
            false,       // shift
            false,       // meta
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('z'), Modifiers::ALT)),
            "QWERTZ Alt+Z should preserve Alt (not false-positive AltGr)"
        );
    }

    #[test]
    fn linux_altgr_qwertz_alt_y_preserves_alt() {
        // QWERTZ: Alt+Y. Physical=Z (swapped), Logical=Y, Unicode='y'.
        let result = translate_key(
            GodotKey::Y, // logical keycode
            GodotKey::Z, // physical keycode (Z position on QWERTZ)
            'y' as u32,  // unicode matches logical key
            false,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('y'), Modifiers::ALT)),
            "QWERTZ Alt+Y should preserve Alt (not false-positive AltGr)"
        );
    }

    #[test]
    fn linux_altgr_azerty_alt_q_preserves_alt() {
        // AZERTY: Alt+Q. Physical=A (swapped), Logical=Q, Unicode='q'.
        let result = translate_key(
            GodotKey::Q, // logical keycode
            GodotKey::A, // physical keycode (A position on AZERTY)
            'q' as u32,  // unicode matches logical key
            false,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('q'), Modifiers::ALT)),
            "AZERTY Alt+Q should preserve Alt (not false-positive AltGr)"
        );
    }

    #[test]
    fn linux_altgr_german_still_detected() {
        // German AltGr+Q → '@'. Physical=Q, Logical=Q, Unicode='@'.
        // Unicode doesn't match logical OR physical — AltGr should trigger.
        let result = translate_key(
            GodotKey::Q,
            GodotKey::Q,
            '@' as u32,
            false,
            true,
            false,
            false,
        );
        // AltGr strips Alt, so we get plain '@'
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)),
            "German AltGr+Q should still detect AltGr and produce plain '@'"
        );
    }

    #[test]
    fn linux_altgr_russian_still_detected() {
        // Russian AltGr: Physical=Q, Logical=Cyrillic, Unicode='@'.
        // Neither physical 'q' nor Cyrillic matches '@' — AltGr triggers.
        let cyrillic_kc = GodotKey::from_ord(0x0439);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::Q,
            '@' as u32,
            false,
            true,
            false,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('@'), Modifiers::NONE)),
            "Russian AltGr should still detect AltGr"
        );
    }

    // ── Alt latin_key for non-Latin layouts ──────────────────────────────

    #[test]
    fn alt_non_latin_gets_latin_key() {
        let cyrillic_kc = GodotKey::from_ord(0x043E);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::J,
            0x043E,
            false,
            true,
            false,
            false, // ctrl=false, alt=true, shift=false, meta=false
        );
        let event = result.expect("should produce a KeyEvent");
        assert_eq!(event.modifiers(), Modifiers::ALT);
        assert_eq!(event.key(), Key::Char('\u{043E}'));
        assert_eq!(
            event.latin_key(),
            Some(Key::Char('j')),
            "Alt + non-Latin should carry latin_key for layout normalization"
        );
    }

    #[test]
    fn meta_non_latin_gets_latin_key() {
        let cyrillic_kc = GodotKey::from_ord(0x043E);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::J,
            0x043E,
            false,
            false,
            false,
            true, // meta=true
        );
        let event = result.expect("should produce a KeyEvent");
        assert_eq!(event.modifiers(), Modifiers::META);
        assert_eq!(
            event.latin_key(),
            Some(Key::Char('j')),
            "Meta + non-Latin should carry latin_key"
        );
    }

    #[test]
    fn alt_latin_no_latin_key() {
        let result = translate_key(
            GodotKey::J,
            GodotKey::J,
            'j' as u32,
            false,
            true,
            false,
            false,
        );
        let event = result.expect("should produce a KeyEvent");
        assert_eq!(
            event.latin_key(),
            None,
            "Alt + ASCII should not carry latin_key"
        );
    }

    #[test]
    fn ctrl_non_latin_no_latin_key() {
        let cyrillic_kc = GodotKey::from_ord(0x043E);
        let result = translate_key(
            cyrillic_kc,
            GodotKey::J,
            0x0A,
            true,
            false,
            false,
            false, // ctrl=true
        );
        let event = result.expect("should produce a KeyEvent");
        assert_eq!(
            event.latin_key(),
            None,
            "Ctrl + non-Latin should NOT carry latin_key (uses resolve_ctrl_key)"
        );
    }

    // ── Minor test coverage gaps (Round 23 audit) ─────────────────────────

    #[test]
    fn bare_numlock_returns_none() {
        assert_eq!(
            translate_key(
                GodotKey::NUMLOCK,
                GodotKey::NUMLOCK,
                0,
                false,
                false,
                false,
                false
            ),
            None
        );
    }

    #[test]
    fn bare_scrolllock_returns_none() {
        assert_eq!(
            translate_key(
                GodotKey::SCROLLLOCK,
                GodotKey::SCROLLLOCK,
                0,
                false,
                false,
                false,
                false
            ),
            None
        );
    }

    #[test]
    fn physical_to_ascii_none_returns_none() {
        assert_eq!(physical_to_ascii(GodotKey::NONE, false), None);
        assert_eq!(physical_to_ascii(GodotKey::NONE, true), None);
    }

    #[test]
    fn ctrl_unrecognized_key_zero_unicode_returns_none() {
        assert_eq!(
            translate_key(
                GodotKey::LAUNCHMAIL,
                GodotKey::LAUNCHMAIL,
                0,
                true,
                false,
                false,
                false
            ),
            None,
            "Ctrl + unrecognized non-printable key should return None"
        );
    }

    #[test]
    fn ctrl_shift_non_letter_zero_unicode_uses_physical_fallback() {
        // Ctrl+Shift+KEY_6 with unicode=0: Step 4 skipped (unicode=0),
        // Step 5 matches KEY_6 as ASCII graphic '6', shift=true so
        // physical_to_ascii(KEY_6, true) = '^', Shift stripped.
        let result = translate_key(
            GodotKey::KEY_6,
            GodotKey::KEY_6,
            0,
            true,
            false,
            true,
            false,
        );
        assert_eq!(
            result,
            Some(KeyEvent::new(Key::Char('^'), Modifiers::CTRL)),
            "Ctrl+Shift+6 with unicode=0 should produce Ctrl+'^' via Step 5 shift handling"
        );
    }
}
