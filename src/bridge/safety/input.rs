use crate::bridge::types::key::{KeyCode, KeyEvent, Modifiers};
use godot::classes::{InputEvent, InputEventKey};
use godot::global::Key;
use godot::prelude::*;

pub fn parse_godot_event(event: &Gd<InputEvent>) -> Option<KeyEvent> {
    let key_event = event.clone().try_cast::<InputEventKey>().ok()?;

    // Process only key press events. Echo events (is_echo() == true) are key repeats
    // and are intentionally allowed so that holding a key moves the cursor repeatedly.
    if !key_event.is_pressed() {
        return None;
    }

    let mut modifiers = get_modifiers(&key_event);
    let keycode = get_keycode(&key_event);

    // For uppercase letters the case is already encoded in the character value,
    // so the SHIFT modifier is stripped to match how mappings are parsed.
    // This ensures `<Space>Z` in mappings matches Shift+Z key presses.
    if let KeyCode::Char(c) = keycode {
        if c.is_ascii_uppercase() {
            modifiers &= !Modifiers::SHIFT;
        }
    }

    let key = KeyEvent::new(keycode, modifiers);
    Some(key)
}

fn get_modifiers(event: &Gd<InputEventKey>) -> Modifiers {
    let mut mods = Modifiers::NONE;
    if event.is_shift_pressed() {
        mods |= Modifiers::SHIFT;
    }
    if event.is_ctrl_pressed() {
        mods |= Modifiers::CTRL;
    }
    if event.is_alt_pressed() {
        mods |= Modifiers::ALT;
    }
    if event.is_meta_pressed() {
        mods |= Modifiers::META;
    }
    mods
}

fn get_keycode(event: &Gd<InputEventKey>) -> KeyCode {
    let raw_code = event.get_keycode();

    match raw_code {
        Key::ESCAPE => KeyCode::Esc,
        Key::SPACE => KeyCode::Char(' '),
        Key::ENTER | Key::KP_ENTER => KeyCode::Enter,
        Key::BACKSPACE => KeyCode::Backspace,
        Key::DELETE => KeyCode::Delete,
        Key::LEFT => KeyCode::Left,
        Key::RIGHT => KeyCode::Right,
        Key::UP => KeyCode::Up,
        Key::DOWN => KeyCode::Down,
        Key::TAB => KeyCode::Tab,
        Key::INSERT => KeyCode::Insert,
        Key::HOME => KeyCode::Home,
        Key::END => KeyCode::End,
        Key::PAGEUP => KeyCode::PageUp,
        Key::PAGEDOWN => KeyCode::PageDown,
        Key::CAPSLOCK => KeyCode::CapsLock,
        Key::SHIFT => KeyCode::Shift,
        Key::CTRL => KeyCode::Control,
        Key::ALT => KeyCode::Alt,
        Key::META => KeyCode::Meta,

        // F-Keys
        Key::F1 => KeyCode::F(1),
        Key::F2 => KeyCode::F(2),
        Key::F3 => KeyCode::F(3),
        Key::F4 => KeyCode::F(4),
        Key::F5 => KeyCode::F(5),
        Key::F6 => KeyCode::F(6),
        Key::F7 => KeyCode::F(7),
        Key::F8 => KeyCode::F(8),
        Key::F9 => KeyCode::F(9),
        Key::F10 => KeyCode::F(10),
        Key::F11 => KeyCode::F(11),
        Key::F12 => KeyCode::F(12),

        _ => {
            // When Ctrl is held, unicode gives control codes (e.g., Ctrl+C = 0x03)
            // Use the raw keycode to get the letter instead
            if event.is_ctrl_pressed() {
                // Extract letter from raw_code (e.g., KEY_C -> 'c')
                // Godot's Key enum maps A-Z to the same ordinals as ASCII (65-90).
                let key_val = raw_code.ord();
                let key_a = Key::A.ord();
                let key_z = Key::Z.ord();
                if (key_a..=key_z).contains(&key_val) {
                    if let Some(ch) = char::from_u32(key_val as u32) {
                        return KeyCode::Char(ch.to_ascii_lowercase());
                    }
                    return KeyCode::Unknown;
                }
            }

            // Handle Chars via Unicode for normal keys
            let unicode = event.get_unicode();
            if let Some(ch) = char::from_u32(unicode) {
                if !ch.is_control() {
                    return KeyCode::Char(ch);
                }
            }
            KeyCode::Unknown
        }
    }
}
