//! Loading and saving songs.

use super::*;
use anyhow::{Context, Result};
use std::path::Path;

/// Load a `.trkr` or `.xrns` file. Returns the song and the path later saves should go to
/// (an imported `.xrns` saves to a sibling `.trkr` instead of overwriting the original).
pub fn load_song(path: &Path) -> Result<(Song, PathBuf)> {
    let (song, _) = load_song_report(path)?;
    Ok((song, save_path_for(path)))
}

/// Imported formats save to a sibling `.trkr` instead of overwriting the original.
fn save_path_for(path: &Path) -> PathBuf {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.as_deref() == Some("trkr") {
        path.to_path_buf()
    } else {
        path.with_extension("trkr")
    }
}

/// Like [`load_song`] but also returns what an import (`.xrns`, `.mod`) had to skip.
pub fn load_song_report(path: &Path) -> Result<(Song, Vec<String>)> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("xrns") => {
            trakr_xrns::import_path_with_report(path).with_context(|| format!("importing {}", path.display()))
        }
        Some("mod") => {
            let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            trakr_mod::import(&bytes).with_context(|| format!("importing {}", path.display()))
        }
        Some("trkr") => trakr_format::load_path(path)
            .map(|s| (s, Vec::new()))
            .with_context(|| format!("loading {}", path.display())),
        _ => anyhow::bail!("unknown file type: {} (expected .trkr, .xrns or .mod)", path.display()),
    }
}

impl Editor {
    /// Replace the whole song (new/open). Clears undo history, selection and cursor.
    pub fn replace_song(&mut self, song: Song, path: Option<PathBuf>) {
        self.song = Arc::new(song);
        self.path = path;
        self.cursor = Cursor::default();
        self.seq_pos = 0;
        self.anchor = None;
        self.undo.clear();
        self.redo.clear();
        self.sync();
        self.dirty = false;
    }

    pub fn new_song(&mut self) {
        self.clear_autosave(); // the discarded document's autosave
        self.replace_song(Song::default(), None);
        self.status = "new song".into();
    }

    pub fn open(&mut self, path: &Path) {
        match load_song_report(path) {
            Ok((song, warnings)) => {
                self.clear_autosave(); // the document being replaced
                let save_path = save_path_for(path);
                self.replace_song(song, Some(save_path));
                self.remember_recent(path);
                self.status = if warnings.is_empty() {
                    format!("opened {}", path.display())
                } else {
                    format!(
                        "opened {} with {} warnings, first: {}",
                        path.display(),
                        warnings.len(),
                        warnings[0]
                    )
                };
            }
            Err(e) => self.status = format!("open failed: {e:#}"),
        }
    }

    /// Export the song as an `.xrns` archive for interoperability; reports what was left out.
    pub fn export_xrns(&mut self, path: &Path) {
        match trakr_xrns::export_path_with_report(&self.song, path) {
            Ok(w) if w.is_empty() => self.status = format!("exported {}", path.display()),
            Ok(w) => {
                self.status = format!("exported {} ({} things skipped: {})", path.display(), w.len(), w[0]);
            }
            Err(e) => self.status = format!("export failed: {e}"),
        }
    }

    pub fn save(&mut self) {
        let path = self.path.clone().unwrap_or_else(|| PathBuf::from("untitled.trkr"));
        self.save_as(path);
    }

    pub fn save_as(&mut self, path: PathBuf) {
        let mut song = (*self.song).clone();
        for p in &mut song.patterns {
            Arc::make_mut(p).compact();
        }
        match trakr_format::save_path(&song, &path) {
            Ok(()) => {
                self.status = format!("saved {}", path.display());
                self.path = Some(path.clone());
                self.dirty = false;
                self.clear_autosave();
                self.remember_recent(&path);
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }
}
