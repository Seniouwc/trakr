//! Frontend-neutral key types. Each frontend translates its native events into these.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    F(u8),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Insert,
    Esc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyEvent {
    pub fn new(code: KeyCode) -> Self {
        Self { code, ctrl: false, alt: false, shift: false }
    }
    pub fn ctrl(c: char) -> Self {
        Self { code: KeyCode::Char(c), ctrl: true, alt: false, shift: false }
    }
    pub fn shift(code: KeyCode) -> Self {
        Self { code, ctrl: false, alt: false, shift: true }
    }
    pub fn alt(code: KeyCode) -> Self {
        Self { code, ctrl: false, alt: true, shift: false }
    }
}
