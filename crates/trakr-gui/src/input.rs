use eframe::egui::{self, Key};
use trakr_editor::{KeyCode, KeyEvent};

fn nav_key(k: Key) -> Option<KeyCode> {
    Some(match k {
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Delete => KeyCode::Delete,
        Key::Insert => KeyCode::Insert,
        Key::Escape => KeyCode::Esc,
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
        _ => return None,
    })
}

fn letter_key(k: Key) -> Option<char> {
    Some(match k {
        Key::A => 'a',
        Key::N => 'n',
        Key::Q => 'q',
        Key::S => 's',
        Key::Y => 'y',
        Key::Z => 'z',
        Key::P => 'p',
        Key::F => 'f',
        Key::R => 'r',
        _ => return None,
    })
}

/// Translate egui events: navigation/function keys and Ctrl combos from key events, typed characters from text events.
pub fn collect_keys(ctx: &egui::Context) -> Vec<KeyEvent> {
    let mut out = Vec::new();
    ctx.input(|i| {
        for ev in &i.events {
            match ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    let mut k = KeyEvent {
                        code: KeyCode::Tab,
                        ctrl: modifiers.command,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                    };
                    if *key == Key::Comma && modifiers.command {
                        k.code = KeyCode::Char(',');
                        out.push(k);
                    } else if let Some(c) = letter_key(*key).filter(|_| modifiers.command) {
                        k.code = KeyCode::Char(c);
                        out.push(k);
                    } else if let Some(code) = nav_key(*key) {
                        k.code =
                            if code == KeyCode::Tab && modifiers.shift { KeyCode::BackTab } else { code };
                        out.push(k);
                    }
                }
                egui::Event::Copy => out.push(KeyEvent::ctrl('c')),
                egui::Event::Cut => out.push(KeyEvent::ctrl('x')),
                egui::Event::Paste(_) => out.push(KeyEvent::ctrl('v')),
                egui::Event::Text(t) => {
                    out.extend(
                        t.chars().filter(|c| !c.is_control()).map(|c| KeyEvent::new(KeyCode::Char(c))),
                    );
                }
                _ => {}
            }
        }
    });
    out
}
