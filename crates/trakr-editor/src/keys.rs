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

impl KeyCode {
    /// Display name used in menus and the saved keymap.
    pub fn name(self) -> String {
        match self {
            KeyCode::Char(' ') => "Space".into(),
            KeyCode::Char(c) => c.to_uppercase().to_string(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Up => "Up".into(),
            KeyCode::Down => "Down".into(),
            KeyCode::Left => "Left".into(),
            KeyCode::Right => "Right".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDn".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::BackTab => "BackTab".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Insert => "Ins".into(),
            KeyCode::Esc => "Esc".into(),
        }
    }

    pub fn from_name(name: &str) -> Option<KeyCode> {
        let lower = name.to_ascii_lowercase();
        Some(match lower.as_str() {
            "space" => KeyCode::Char(' '),
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "pgup" | "pageup" => KeyCode::PageUp,
            "pgdn" | "pagedown" => KeyCode::PageDown,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "backspace" => KeyCode::Backspace,
            "del" | "delete" => KeyCode::Delete,
            "ins" | "insert" => KeyCode::Insert,
            "esc" | "escape" => KeyCode::Esc,
            _ => {
                if let Some(n) = lower
                    .strip_prefix('f')
                    .and_then(|n| n.parse::<u8>().ok())
                    .filter(|n| (1..=24).contains(n))
                {
                    return Some(KeyCode::F(n));
                }
                let mut chars = lower.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => KeyCode::Char(c),
                    _ => return None,
                }
            }
        })
    }
}

impl KeyEvent {
    /// Text form such as `Ctrl-Shift-Z` or `Alt-PgUp`, used in menus and the saved keymap.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl-");
        }
        if self.alt {
            s.push_str("Alt-");
        }
        if self.shift {
            s.push_str("Shift-");
        }
        s.push_str(&self.code.name());
        s
    }

    /// Parse the text form back. Returns `None` for anything unrecognised.
    pub fn from_text(text: &str) -> Option<KeyEvent> {
        let mut rest = text.trim();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        loop {
            let lower = rest.to_ascii_lowercase();
            if let Some(r) = lower.strip_prefix("ctrl-").filter(|r| !r.is_empty()) {
                ctrl = true;
                rest = &rest[rest.len() - r.len()..];
            } else if let Some(r) = lower.strip_prefix("alt-").filter(|r| !r.is_empty()) {
                alt = true;
                rest = &rest[rest.len() - r.len()..];
            } else if let Some(r) = lower.strip_prefix("shift-").filter(|r| !r.is_empty()) {
                shift = true;
                rest = &rest[rest.len() - r.len()..];
            } else {
                break;
            }
        }
        KeyCode::from_name(rest).map(|code| KeyEvent { code, ctrl, alt, shift })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_text_roundtrips() {
        for text in [
            "Ctrl-Z",
            "Ctrl-Shift-Z",
            "Alt-PgUp",
            "F5",
            "Space",
            "Del",
            "Esc",
            "Ctrl-Alt-Shift-F12",
            "Ctrl--",
            "Ctrl-Up",
        ] {
            let k = KeyEvent::from_text(text).unwrap_or_else(|| panic!("cannot parse {text}"));
            assert_eq!(k.to_text(), text, "{text}");
        }
        assert_eq!(KeyEvent::from_text("ctrl-s"), Some(KeyEvent::ctrl('s')));
        assert!(KeyEvent::from_text("").is_none());
        assert!(KeyEvent::from_text("Ctrl-").is_none());
        assert!(KeyEvent::from_text("nonsense").is_none());
        assert!(KeyEvent::from_text("F99").is_none());
    }
}
