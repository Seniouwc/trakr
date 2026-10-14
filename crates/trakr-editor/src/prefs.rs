//! Preferences, recent files and autosave recovery.

use super::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};

const MAX_RECENT: usize = 10;
pub const DEFAULT_AUTOSAVE_SECONDS: u64 = 60;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub recent: Vec<PathBuf>,
    pub octave: u8,
    pub edit_step: u16,
    pub follow: bool,
    /// Seconds between autosaves of unsaved work; 0 turns autosave off.
    pub autosave_seconds: u64,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            octave: 4,
            edit_step: 1,
            follow: true,
            autosave_seconds: DEFAULT_AUTOSAVE_SECONDS,
        }
    }
}

/// `$XDG_CONFIG_HOME/trakr` or `~/.config/trakr`.
pub fn default_config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("trakr"))
}

impl Editor {
    /// Where preferences and untitled autosaves live. Frontends leave this unset in tests.
    pub fn set_config_dir(&mut self, dir: Option<PathBuf>) {
        self.config_dir = dir;
    }

    /// Load saved preferences (missing or unreadable files are ignored) and apply them.
    pub fn load_prefs(&mut self) {
        let Some(dir) = &self.config_dir else { return };
        if let Ok(text) = std::fs::read_to_string(dir.join("prefs.json")) {
            if let Ok(p) = serde_json::from_str::<Prefs>(&text) {
                self.octave = p.octave.min(8);
                self.edit_step = p.edit_step.min(64);
                self.follow = p.follow;
                self.prefs = p;
            }
        }
    }

    /// Write preferences (including the current octave, step and follow settings).
    pub fn save_prefs(&mut self) {
        self.prefs.octave = self.octave;
        self.prefs.edit_step = self.edit_step;
        self.prefs.follow = self.follow;
        let Some(dir) = &self.config_dir else { return };
        if std::fs::create_dir_all(dir).is_ok() {
            if let Ok(text) = serde_json::to_string_pretty(&self.prefs) {
                let _ = std::fs::write(dir.join("prefs.json"), text);
            }
        }
    }

    pub fn recent_files(&self) -> &[PathBuf] {
        &self.prefs.recent
    }

    pub(crate) fn remember_recent(&mut self, path: &Path) {
        let path = path.to_path_buf();
        self.prefs.recent.retain(|p| p != &path);
        self.prefs.recent.insert(0, path);
        self.prefs.recent.truncate(MAX_RECENT);
        self.save_prefs();
    }

    // ---- autosave --------------------------------------------------------------------

    /// Autosave file for the current song: beside the project, or in the config dir when untitled.
    pub fn autosave_path(&self) -> Option<PathBuf> {
        match &self.path {
            Some(p) => {
                let mut name = p.file_name()?.to_os_string();
                name.push(".autosave");
                Some(p.with_file_name(name))
            }
            None => self.config_dir.as_ref().map(|d| d.join("untitled.trkr.autosave")),
        }
    }

    pub fn set_autosave_interval(&mut self, interval: Duration) {
        self.autosave_every = interval;
    }

    /// Called from `tick`: write an autosave when there is unsaved work and the interval has passed.
    pub(crate) fn autosave_poll(&mut self) {
        if !self.dirty || self.autosave_every.is_zero() && self.prefs.autosave_seconds == 0 {
            return;
        }
        let every = if self.autosave_every.is_zero() {
            Duration::from_secs(self.prefs.autosave_seconds)
        } else {
            self.autosave_every
        };
        if self.last_autosave.is_some_and(|t| t.elapsed() < every) {
            return;
        }
        self.last_autosave = Some(Instant::now());
        self.autosave_now();
    }

    pub fn autosave_now(&mut self) {
        let Some(path) = self.autosave_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut song = (*self.song).clone();
        for p in &mut song.patterns {
            Arc::make_mut(p).compact();
        }
        if trakr_format::save_path(&song, &path).is_ok() {
            self.autosaved = true;
        }
    }

    pub(crate) fn clear_autosave(&mut self) {
        if let Some(p) = self.autosave_path() {
            let _ = std::fs::remove_file(p);
        }
        self.autosaved = false;
    }

    /// Delete the autosave without loading it.
    pub fn discard_autosave(&mut self) {
        self.clear_autosave();
    }

    /// True when an autosave exists that is newer than the saved project (or the project is gone).
    pub fn recovery_available(&self) -> bool {
        let Some(auto) = self.autosave_path() else { return false };
        let Ok(a) = std::fs::metadata(&auto).and_then(|m| m.modified()) else { return false };
        match self.path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok()) {
            Some(orig) => a > orig,
            None => true,
        }
    }

    /// Replace the song with the autosave. The result counts as unsaved so it gets saved properly.
    pub fn recover_autosave(&mut self) {
        let Some(auto) = self.autosave_path() else { return };
        match trakr_format::load_path(&auto) {
            Ok(song) => {
                let path = self.path.clone();
                self.replace_song(song, path);
                self.dirty = true;
                self.status = "recovered unsaved work from the autosave: save to keep it".into();
            }
            Err(e) => self.status = format!("cannot read the autosave: {e}"),
        }
    }
}
