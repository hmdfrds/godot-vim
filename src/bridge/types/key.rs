//! Key input types for the godot-vim shell.
//!
//! # ADR: Mirroring vim-core's Key Types
//!
//! **Context**: vim-core uses `VimKey { code: KeyCode, modifiers: VimModifiers }`.
//! The shell needs its own equivalent to avoid importing vim-core in input routing,
//! mapping panels, and settings.
//!
//! **Decision**: `KeyEvent` mirrors `VimKey` exactly. Conversions happen in
//! `vim_adapter/convert.rs`. This is intentionally 1:1 so conversion is zero-cost.
//!
//! **Consequence**: Adding a new key to vim-core requires a corresponding addition
//! here and in the converter, but no other shell code changes.

/// Modifier keys held during a key event.
///
/// Uses a raw `u8` bitfield for zero-cost conversion with vim-core's `VimModifiers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0b0000);
    pub const SHIFT: Self = Self(0b0001);
    pub const CTRL: Self = Self(0b0010);
    pub const ALT: Self = Self(0b0100);
    pub const META: Self = Self(0b1000);

    /// Returns the raw bits.
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns `true` if no modifiers are set.
    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns `true` if `other` is a subset of `self`.
    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Union of two modifier sets.
    #[inline]
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for Modifiers {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for Modifiers {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl std::ops::Not for Modifiers {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// Physical or logical key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    Char(char),
    F(u8),
    Esc,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Insert,
    CapsLock,
    Shift,
    Control,
    Alt,
    Meta,
    Unknown,
}

/// A key event with modifiers.
///
/// This is the shell's own key representation. All Godot `InputEventKey` events
/// are converted to this type before entering the input pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
}

impl KeyEvent {
    /// Creates a new key event.
    #[inline]
    #[must_use]
    pub const fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers }
    }
}

impl std::fmt::Display for KeyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Char(c) => write!(f, "{c}"),
            Self::F(n) => write!(f, "<F{n}>"),
            Self::Esc => f.write_str("<Esc>"),
            Self::Enter => f.write_str("<CR>"),
            Self::Backspace => f.write_str("<BS>"),
            Self::Delete => f.write_str("<Del>"),
            Self::Left => f.write_str("<Left>"),
            Self::Right => f.write_str("<Right>"),
            Self::Up => f.write_str("<Up>"),
            Self::Down => f.write_str("<Down>"),
            Self::Home => f.write_str("<Home>"),
            Self::End => f.write_str("<End>"),
            Self::PageUp => f.write_str("<PageUp>"),
            Self::PageDown => f.write_str("<PageDown>"),
            Self::Tab => f.write_str("<Tab>"),
            Self::Insert => f.write_str("<Insert>"),
            Self::CapsLock => f.write_str("<CapsLock>"),
            Self::Shift => f.write_str("<Shift>"),
            Self::Control => f.write_str("<Ctrl>"),
            Self::Alt => f.write_str("<Alt>"),
            Self::Meta => f.write_str("<Meta>"),
            Self::Unknown => f.write_str("<Unknown>"),
        }
    }
}

impl std::fmt::Display for KeyEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.modifiers.is_empty() {
            return write!(f, "{}", self.code);
        }

        f.write_str("<")?;
        if self.modifiers.contains(Modifiers::CTRL) {
            f.write_str("C-")?;
        }
        if self.modifiers.contains(Modifiers::ALT) {
            f.write_str("A-")?;
        }
        if self.modifiers.contains(Modifiers::META) {
            f.write_str("D-")?;
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            let print_shift = match self.code {
                KeyCode::Char(c) => c.is_lowercase() || !c.is_alphabetic(),
                _ => true,
            };
            if print_shift {
                f.write_str("S-")?;
            }
        }
        write!(f, "{}>", self.code)
    }
}
