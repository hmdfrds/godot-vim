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

fn get_modifiers(event: &Gd<InputEventKey>) -> Modifiers {
    let mut mods = Modifiers::NONE;
    if event.is_ctrl_pressed() {
        mods |= Modifiers::CTRL;
    }
    if event.is_alt_pressed() {
        mods |= Modifiers::ALT;
    }
    if event.is_shift_pressed() {
        mods |= Modifiers::SHIFT;
    }
    if event.is_meta_pressed() {
        mods |= Modifiers::META;
    }
    mods
}

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

/// Convert a Godot `InputEventKey` into a vim-core `KeyEvent`.
///
/// Returns `None` for events we don't handle:
/// - Bare modifier keys (Shift/Ctrl/Alt/Meta/CapsLock pressed alone)
/// - Unrecognized keys with no unicode representation
/// - Control characters not produced by Ctrl+letter
pub(crate) fn parse_godot_key(event: &Gd<InputEventKey>) -> Option<KeyEvent> {
    let mut modifiers = get_modifiers(event);
    let raw_code = event.get_keycode();

    if matches!(
        raw_code,
        GodotKey::SHIFT
            | GodotKey::CTRL
            | GodotKey::ALT
            | GodotKey::META
            | GodotKey::CAPSLOCK
    ) {
        log::trace!("parse_godot_key: filtered bare modifier {:?}", raw_code);
        return None;
    }

    if let Some(key) = get_named_key(raw_code) {
        log::trace!("parse_godot_key: named key {} mods={}", key, modifiers);
        return Some(KeyEvent::new(key, modifiers));
    }

    // Ctrl+letter: Godot's unicode is a control code (U+0001 for Ctrl+A, etc.)
    // which is useless. Use the raw keycode instead — Godot's Key enum maps
    // A-Z to ASCII ordinals 65-90, so we can safely convert.
    if modifiers.contains(Modifiers::CTRL) {
        let key_val = raw_code.ord();
        let key_a = GodotKey::A.ord();
        let key_z = GodotKey::Z.ord();
        if (key_a..=key_z).contains(&key_val) {
            if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
                return Some(KeyEvent::new(
                    Key::Char(ch.to_ascii_lowercase()),
                    modifiers,
                ));
            }
            log::warn!("parse_godot_key: Ctrl+letter keycode={} char conversion failed", key_val);
            return None;
        }
    }

    // Ctrl+non-letter (e.g. Ctrl+[, Ctrl+]): Godot's unicode is again a
    // control code (Ctrl+[ = U+001B = ESC). We need Key::Char('[') + Ctrl
    // so the engine can match Vim's <C-[>, <C-]>, <C-^> notation.
    if modifiers.contains(Modifiers::CTRL) {
        let key_val = raw_code.ord();
        if let Some(ch) = u32::try_from(key_val).ok().and_then(char::from_u32) {
            if !ch.is_control() {
                log::trace!("parse_godot_key: Ctrl+non-letter {:?} -> Key::Char({:?})", raw_code, ch);
                return Some(KeyEvent::new(
                    Key::Char(ch.to_ascii_lowercase()),
                    modifiers,
                ));
            }
        }
    }

    let unicode = event.get_unicode();
    if unicode == 0 {
        log::trace!("parse_godot_key: zero unicode for {:?}", raw_code);
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

    log::trace!("parse_godot_key: char='{}' mods={}", ch, modifiers);
    Some(KeyEvent::new(Key::Char(ch), modifiers))
}
