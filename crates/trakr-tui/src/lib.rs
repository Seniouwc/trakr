//! Terminal UI for trakr (ratatui + crossterm).

mod app;
mod ui;

pub use app::App;

use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::path::PathBuf;
use std::time::Duration;
use trakr_core::Song;
use trakr_engine::EngineHandle;

/// Run the TUI until the user quits. `engine` is `None` when no audio device is available.
pub fn run(song: Song, path: Option<PathBuf>, engine: Option<EngineHandle>) -> anyhow::Result<()> {
    let mut app = App::new(song, path, engine);
    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            terminal.draw(|f| ui::draw(f, &app))?;
            if event::poll(Duration::from_millis(33))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        app.on_key(k);
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
