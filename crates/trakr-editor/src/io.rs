//! Loading and saving songs.

use super::*;
use anyhow::{Context, Result};
use std::path::Path;

/// Load a `.trkr` or `.xrns` file. Returns the song and the path later saves should go to
/// (an imported `.xrns` saves to a sibling `.trkr` instead of overwriting the original).
pub fn load_song(path: &Path) -> Result<(Song, PathBuf)> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("xrns") => {
            let (song, _) = load_song_report(path)?;
            Ok((song, path.with_extension("trkr")))
        }
        Some("trkr") => {
            let song =
                trakr_format::load_path(path).with_context(|| format!("loading {}", path.display()))?;
            Ok((song, path.to_path_buf()))
        }
        _ => anyhow::bail!("unknown file type: {} (expected .trkr or .xrns)", path.display()),
    }
}

/// Like [`load_song`] but also returns what an `.xrns` import had to skip.
pub fn load_song_report(path: &Path) -> Result<(Song, Vec<String>)> {
    let (song, warnings) =
        trakr_xrns::import_path_with_report(path).with_context(|| format!("importing {}", path.display()))?;
    Ok((song, warnings))
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
        self.replace_song(Song::default(), None);
        self.status = "new song".into();
    }

    pub fn open(&mut self, path: &Path) {
        match load_song(path) {
            Ok((song, save_path)) => {
                let is_xrns = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xrns"));
                let warnings = if is_xrns {
                    load_song_report(path).map(|(_, w)| w).unwrap_or_default()
                } else {
                    Vec::new()
                };
                self.replace_song(song, Some(save_path));
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
                self.path = Some(path);
                self.dirty = false;
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }
}
