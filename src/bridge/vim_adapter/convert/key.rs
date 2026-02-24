use crate::bridge::types::key::{KeyCode, KeyEvent};
use vim_core::inputs::keys::{KeyCode as VimKeyCode, VimKey, VimModifiers};

/// Convert shell `KeyEvent` to vim-core `VimKey`.
#[inline]
#[must_use]
pub fn key_event_to_vim_key(key: &KeyEvent) -> VimKey {
    VimKey::new(
        key_code_to_vim(key.code),
        VimModifiers::from_bits_truncate(key.modifiers.bits()),
    )
}

/// Generates a conversion function from enum A to enum B with identical variant names.
macro_rules! enum_convert {
    (
        $a:ident => $b:ident;
        fwd = $fwd:ident;
        variants: [ $( $variant:ident ),* $(,)? ]
        $( ; wrapped: [ $( $wvar:ident($wb:ident) ),* $(,)? ] )?
    ) => {
        #[inline]
        #[must_use]
        pub fn $fwd(v: $a) -> $b {
            match v {
                $( $a::$variant => $b::$variant, )*
                $( $( $a::$wvar($wb) => $b::$wvar($wb), )* )?
            }
        }
    };
}

enum_convert! {
    KeyCode => VimKeyCode;
    fwd = key_code_to_vim;
    variants: [
        Esc, Enter, Backspace, Delete,
        Left, Right, Up, Down,
        Home, End, PageUp, PageDown,
        Tab, Insert, CapsLock,
        Shift, Control, Alt, Meta, Unknown,
    ];
    wrapped: [Char(c), F(n)]
}
