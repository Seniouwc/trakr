//! Terminal UI for trakr (ratatui + crossterm).

mod ui;

use ratatui::crossterm::event::{self, Event, KeyCode as CK, KeyEvent as CKE, KeyEventKind, KeyModifiers};
use std::time::Duration;
use trakr_editor::{Editor, KeyCode, KeyEvent};

fn translate(k: CKE) -> Option<KeyEvent> {
    let code = match k.code {
        CK::Char(c) => KeyCode::Char(c),
        CK::F(n) => KeyCode::F(n),
        CK::Up => KeyCode::Up,
        CK::Down => KeyCode::Down,
        CK::Left => KeyCode::Left,
        CK::Right => KeyCode::Right,
        CK::PageUp => KeyCode::PageUp,
        CK::PageDown => KeyCode::PageDown,
        CK::Home => KeyCode::Home,
        CK::End => KeyCode::End,
        CK::Tab => KeyCode::Tab,
        CK::BackTab => KeyCode::BackTab,
        CK::Backspace => KeyCode::Backspace,
        CK::Delete => KeyCode::Delete,
        CK::Insert => KeyCode::Insert,
        CK::Esc => KeyCode::Esc,
        _ => return None,
    };
    Some(KeyEvent {
        code,
        ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
        alt: k.modifiers.contains(KeyModifiers::ALT),
        shift: k.modifiers.contains(KeyModifiers::SHIFT),
    })
}

/// Run the TUI until the user quits.
pub fn run(mut app: Editor) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            terminal.draw(|f| ui::draw(f, &app))?;
            if event::poll(Duration::from_millis(33))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        if let Some(k) = translate(k) {
                            app.on_key(k);
                        }
                    }
                }
            }
            app.tick();
        }
        Ok(())
    })();
    ratatui::restore();
    result
}
