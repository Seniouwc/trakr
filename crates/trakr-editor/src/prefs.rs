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
    /// Custom key bindings: action name (as in `Action`'s Debug form) to shortcut text; empty text unbinds.
    pub keymap: std::collections::BTreeMap<String, String>,
    /// "dark", "light" or "contrast".
    pub theme: String,
    pub ui_scale: f32,
    /// Pattern grid font size in points.
    pub font_size: f32,
    /// Font file for the interface text (empty = built in).
    pub ui_font: String,
    /// Font file for the pattern grid and other monospaced text (empty = built in).
    pub grid_font: String,
    /// Send MIDI timing clock and start/stop to the output port.
    pub midi_clock_out: bool,
    /// Delay applied to MIDI output so it lines up with the audio device's latency.
    pub midi_latency_ms: u32,
    /// Follow the tempo of an incoming MIDI clock.
    pub midi_sync_in: bool,
    /// Window size to restore, in points.
    pub window_size: Option<[f32; 2]>,
    /// UDP port of the OSC server (`None` = off).
    pub osc_port: Option<u16>,
    /// Listen on every network interface instead of this machine only.
    pub osc_any_interface: bool,
    /// `host:port` to send state changes to (`None` = off).
    pub osc_feedback: Option<String>,
    /// Bottom panel to restore ("instrument", "mixer", ...).
    pub dock: String,
    /// Open the most recent song at start-up when none is named.
    pub reopen_last: bool,
    /// Keep the previous file as `<name>.bak` when saving over it.
    pub backups: bool,
    /// CC and key mappings made with MIDI learn.
    pub midi_map: Vec<crate::midi_map::MidiMapping>,
    /// Last audio export settings (format, quality, bit depth, sample rate).
    pub export: trakr_encode::ExportFormat,
    /// Audio host, devices, sample rate and buffer size.
    pub audio: crate::AudioSettings,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            octave: 4,
            edit_step: 1,
            follow: true,
            autosave_seconds: DEFAULT_AUTOSAVE_SECONDS,
            keymap: Default::default(),
            theme: "dark".into(),
            ui_scale: 1.0,
            font_size: 15.0,
            ui_font: String::new(),
            grid_font: String::new(),
            midi_clock_out: false,
            midi_latency_ms: 30,
            midi_sync_in: false,
            window_size: None,
            osc_port: None,
            osc_any_interface: false,
            osc_feedback: None,
            dock: "instrument".into(),
            reopen_last: false,
            backups: true,
            midi_map: Vec::new(),
            export: Default::default(),
            audio: Default::default(),
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

    /// The preferences folder (`~/.config/trakr`), if one is set.
    pub fn config_dir(&self) -> Option<&std::path::Path> {
        self.config_dir.as_deref()
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
                self.rebuild_keymap();
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

// ---- key bindings ------------------------------------------------------------------------

const NAV_KEYS: [KeyCode; 10] = [
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Left,
    KeyCode::Right,
    KeyCode::PageUp,
    KeyCode::PageDown,
    KeyCode::Home,
    KeyCode::End,
    KeyCode::Tab,
    KeyCode::BackTab,
];

impl Editor {
    pub(crate) fn rebuild_keymap(&mut self) {
        let mut map: Vec<(KeyEvent, Action)> = Vec::new();
        for &a in Action::ALL {
            let text = match self.prefs.keymap.get(&format!("{a:?}")) {
                Some(t) => t.as_str(),
                None => a.shortcut(),
            };
            if let Some(k) = KeyEvent::from_text(text) {
                map.push((k, a));
            }
        }
        if !self.prefs.keymap.contains_key("Redo") {
            map.push((
                KeyEvent { code: KeyCode::Char('z'), ctrl: true, alt: false, shift: true },
                Action::Redo,
            ));
        }
        self.keymap = map;
    }

    /// The action bound to a key press, if any.
    pub fn action_for_key(&self, k: KeyEvent) -> Option<Action> {
        let find = |k: KeyEvent| self.keymap.iter().find(|(b, _)| *b == k).map(|(_, a)| *a);
        find(k).or_else(|| {
            // Shift+F5 acts like F5, but Shift+arrows keep extending the selection.
            (k.shift && !NAV_KEYS.contains(&k.code)).then(|| find(KeyEvent { shift: false, ..k })).flatten()
        })
    }

    /// Current shortcut text for an action ("" when unbound).
    pub fn shortcut_for(&self, a: Action) -> String {
        self.keymap.iter().find(|(_, b)| *b == a).map(|(k, _)| k.to_text()).unwrap_or_default()
    }

    /// Bind (or with `None` unbind) a key. Typing keys stay reserved for note and value entry, and a key
    /// already used by another action is refused with a message naming it.
    pub fn set_binding(&mut self, a: Action, key: Option<KeyEvent>) -> Result<(), String> {
        if let Some(k) = key {
            let plain = !k.ctrl && !k.alt;
            if plain && matches!(k.code, KeyCode::Char(c) if c != ' ') {
                return Err("plain letters, digits and symbols are for entering notes and values".into());
            }
            if plain && NAV_KEYS.contains(&k.code) {
                return Err(
                    "plain navigation keys (arrows, Tab, Home, End, PgUp, PgDn) cannot be rebound".into()
                );
            }
            if let Some((_, other)) = self.keymap.iter().find(|(b, o)| *b == k && *o != a) {
                return Err(format!("{} is already used by \"{}\"", k.to_text(), other.label()));
            }
        }
        self.prefs.keymap.insert(format!("{a:?}"), key.map(|k| k.to_text()).unwrap_or_default());
        self.rebuild_keymap();
        self.save_prefs();
        Ok(())
    }

    /// Go back to the built-in shortcut for an action.
    pub fn reset_binding(&mut self, a: Action) {
        self.prefs.keymap.remove(&format!("{a:?}"));
        self.rebuild_keymap();
        self.save_prefs();
    }

    pub fn reset_all_bindings(&mut self) {
        self.prefs.keymap.clear();
        self.rebuild_keymap();
        self.save_prefs();
    }
}
