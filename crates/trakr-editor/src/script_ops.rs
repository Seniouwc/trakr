//! Running Lua scripts (see the `trakr-script` crate) against the song.

use super::*;
use std::path::{Path, PathBuf};
use trakr_script::{Request, ScriptCtx, Selection};

impl Editor {
    fn script_ctx(&self) -> ScriptCtx {
        let selection = self.selection().map(|b| {
            let (t0, _) = self.unflat(b.col0);
            let (t1, _) = self.unflat(b.col1);
            Selection { track0: t0, track1: t1, line0: b.line0, line1: b.line1 }
        });
        ScriptCtx {
            pattern: self.pattern_index().unwrap_or(0),
            seq_pos: self.seq_pos,
            track: self.cursor.track,
            line: self.cursor.line,
            col: self.cursor.col,
            instrument: self.instrument,
            octave: self.octave,
            selection,
        }
    }

    /// Run a Lua script. On success its edits are one undo step; on error the song is untouched and
    /// the message lands in `script_error` and the status bar. Returns whether it succeeded.
    pub fn run_script(&mut self, source: &str) -> bool {
        self.script_output.clear();
        self.script_error = None;
        match trakr_script::run(&self.song, &self.script_ctx(), source) {
            Ok(out) => {
                self.script_output = out.output;
                if out.changed {
                    self.checkpoint();
                    self.song = Arc::new(out.song);
                    self.song_mut().normalize_slot_meta();
                    self.clamp_cursor();
                    self.sync();
                }
                self.status = out.status.unwrap_or_else(|| {
                    if out.changed {
                        "script finished".into()
                    } else {
                        "script finished (no changes)".into()
                    }
                });
                for r in out.requests {
                    match r {
                        Request::Play => self.do_action(Action::PlaySong),
                        Request::Stop => self.do_action(Action::Stop),
                    }
                }
                true
            }
            Err(e) => {
                self.status = format!("script error: {e}");
                self.script_error = Some(e.to_string());
                false
            }
        }
    }

    /// Run a script file.
    pub fn run_script_file(&mut self, path: &Path) -> bool {
        match std::fs::read_to_string(path) {
            Ok(text) => self.run_script(&text),
            Err(e) => {
                self.status = format!("cannot read {}: {e}", path.display());
                false
            }
        }
    }

    /// `<config dir>/scripts`, where user scripts live.
    pub fn scripts_dir(&self) -> Option<PathBuf> {
        self.config_dir.as_ref().map(|d| d.join("scripts"))
    }

    /// The `.lua` files in the scripts directory as (display name, path), sorted by name.
    pub fn user_scripts(&self) -> Vec<(String, PathBuf)> {
        let Some(dir) = self.scripts_dir() else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut v: Vec<(String, PathBuf)> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "lua"))
            .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
            .collect();
        v.sort();
        v
    }
}
