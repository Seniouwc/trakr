//! Instrument and sample operations.

use super::*;
use std::path::Path;
use trakr_core::Sample;

impl Editor {
    /// Decode an audio file into a new instrument and select it.
    pub fn import_sample(&mut self, path: &Path) {
        match trakr_sample::decode_file(path) {
            Ok(sample) => {
                self.checkpoint();
                let name = sample.name.clone();
                let mut inst = Instrument::new(name.clone());
                inst.samples.push(sample);
                let song = self.song_mut();
                song.instruments.push(inst);
                self.instrument = (self.song.instruments.len() - 1).min(0xFE) as u8;
                self.sample = 0;
                self.status = format!("imported {name}");
                self.sync();
            }
            Err(e) => self.status = format!("import failed: {e}"),
        }
    }

    /// Save the selected instrument, with its samples, as `.trki` (native) or `.xrni` (by the extension).
    pub fn instrument_save_file(&mut self, path: &Path) -> bool {
        let Some(inst) = self.current_instrument().cloned() else {
            self.status = "no instrument selected".into();
            return false;
        };
        let xrni = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xrni"));
        let path = if path.extension().is_none() { path.with_extension("trki") } else { path.to_path_buf() };
        let result = if xrni {
            trakr_xrns::export_instrument_path(&inst, &path)
                .map(|w| {
                    w.first()
                        .map(|f| format!(" ({} things skipped, first: {f})", w.len()))
                        .unwrap_or_default()
                })
                .map_err(|e| e.to_string())
        } else {
            trakr_format::save_instrument_path(&inst, &path).map(|_| String::new()).map_err(|e| e.to_string())
        };
        match result {
            Ok(note) => {
                self.status = format!("saved instrument {}{note}", path.display());
                true
            }
            Err(e) => {
                self.status = format!("saving instrument failed: {e}");
                false
            }
        }
    }

    /// Load a `.trki` or `.xrni` file as a new instrument and select it.
    pub fn instrument_load_file(&mut self, path: &Path) -> bool {
        let xrni = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xrni"));
        let loaded = if xrni {
            trakr_xrns::import_instrument_path(path).map_err(|e| e.to_string()).map(|(i, w)| {
                let note =
                    w.first().map(|f| format!(" ({} warnings, first: {f})", w.len())).unwrap_or_default();
                (i, note)
            })
        } else {
            trakr_format::load_instrument_path(path).map_err(|e| e.to_string()).map(|i| (i, String::new()))
        };
        match loaded {
            Ok((inst, note)) => {
                if self.song.instruments.len() >= 0xFE {
                    self.status = "instrument limit reached".into();
                    return false;
                }
                self.checkpoint();
                let name = inst.name.clone();
                self.song_mut().instruments.push(inst);
                self.instrument = (self.song.instruments.len() - 1).min(0xFE) as u8;
                self.sample = 0;
                self.status = format!("loaded instrument {name}{note}");
                self.sync();
                true
            }
            Err(e) => {
                self.status = format!("loading instrument failed: {e}");
                false
            }
        }
    }

    /// `<config dir>/instruments`: the instrument bank.
    pub fn instrument_bank_dir(&self) -> Option<std::path::PathBuf> {
        self.config_dir.as_ref().map(|d| d.join("instruments"))
    }

    /// Instrument files in the bank (`.trki` and `.xrni`) as (display name, path), sorted by name.
    pub fn instrument_bank(&self) -> Vec<(String, std::path::PathBuf)> {
        let Some(dir) = self.instrument_bank_dir() else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut v: Vec<(String, std::path::PathBuf)> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("trki") || e.eq_ignore_ascii_case("xrni"))
            })
            .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
            .collect();
        v.sort();
        v
    }

    /// Save the selected instrument into the bank under its own name.
    pub fn instrument_save_to_bank(&mut self) -> bool {
        let Some(dir) = self.instrument_bank_dir() else {
            self.status = "no settings folder for the instrument bank".into();
            return false;
        };
        let Some(name) = self.current_instrument().map(|i| i.name.clone()) else { return false };
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
            .collect();
        let safe = if safe.trim().is_empty() { "instrument".to_string() } else { safe.trim().to_string() };
        if std::fs::create_dir_all(&dir).is_err() {
            self.status = "cannot create the instrument bank folder".into();
            return false;
        }
        self.instrument_save_file(&dir.join(format!("{safe}.trki")))
    }

    /// Add a native device to the selected instrument's FX chain.
    pub fn instrument_fx_add(&mut self, id: &str) -> bool {
        let idx = self.instrument as usize;
        let Some(spec) = trakr_core::DeviceSpec::new(id) else {
            self.status = format!("unknown device {id}");
            return false;
        };
        if idx >= self.song.instruments.len() {
            self.status = "no instrument selected".into();
            return false;
        }
        if trakr_core::MODULATORS.contains(&id) || id == "native.send" {
            self.status = "modulators and sends only work on tracks".into();
            return false;
        }
        if self.song.instruments[idx].fx.len() >= 16 {
            self.status = "an instrument's FX chain holds at most 16 devices".into();
            return false;
        }
        self.checkpoint();
        self.song_mut().instruments[idx].fx.push(spec);
        self.sync();
        true
    }

    pub fn instrument_fx_remove(&mut self, index: usize) {
        let idx = self.instrument as usize;
        if self.song.instruments.get(idx).is_some_and(|i| index < i.fx.len()) {
            self.checkpoint();
            self.song_mut().instruments[idx].fx.remove(index);
            self.sync();
        }
    }

    pub fn instrument_fx_toggle_bypass(&mut self, index: usize) {
        let idx = self.instrument as usize;
        if self.song.instruments.get(idx).is_some_and(|i| index < i.fx.len()) {
            self.checkpoint();
            let d = &mut self.song_mut().instruments[idx].fx[index];
            d.bypass = !d.bypass;
            self.sync();
        }
    }

    pub fn instrument_fx_move(&mut self, index: usize, delta: i32) {
        let idx = self.instrument as usize;
        let Some(n) = self.song.instruments.get(idx).map(|i| i.fx.len()) else { return };
        let to = index as i32 + delta;
        if index < n && (0..n as i32).contains(&to) {
            self.checkpoint();
            self.song_mut().instruments[idx].fx.swap(index, to as usize);
            self.sync();
        }
    }

    pub fn instrument_fx_set_param(&mut self, index: usize, param: usize, value: f32) {
        let idx = self.instrument as usize;
        let Some(spec) = self.song.instruments.get(idx).and_then(|i| i.fx.get(index)) else { return };
        let Some((_, min, max, _, _, _, _)) = spec.param_meta(param) else { return };
        if !(value.is_finite() && min.is_finite() && max.is_finite()) {
            return;
        }
        let value = value.clamp(min.min(max), min.max(max));
        let defaults: Vec<f32> =
            (0..spec.param_count()).filter_map(|i| spec.param_meta(i).map(|m| m.3)).collect();
        self.checkpoint_key("inst_fx_param");
        let d = &mut self.song_mut().instruments[idx].fx[index];
        if d.params.len() < defaults.len() {
            let have = d.params.len();
            d.params.extend_from_slice(&defaults[have..]);
        }
        d.params[param] = value;
        self.sync();
    }

    pub fn instrument_add_empty(&mut self) {
        self.checkpoint();
        let n = self.song.instruments.len();
        self.song_mut().instruments.push(Instrument::new(format!("Instrument {n:02X}")));
        self.instrument = n.min(0xFE) as u8;
        self.sync();
    }

    /// Remove the selected instrument. Notes using it lose their instrument; higher ones shift down.
    pub fn instrument_remove(&mut self) {
        let idx = self.instrument as usize;
        if idx >= self.song.instruments.len() {
            return;
        }
        self.checkpoint();
        let song = self.song_mut();
        song.instruments.remove(idx);
        for p in &mut song.patterns {
            let used = p
                .tracks
                .iter()
                .flat_map(|t| t.lines.values())
                .any(|l| l.notes.iter().any(|n| n.instrument != EMPTY && n.instrument as usize >= idx));
            if !used {
                continue;
            }
            for t in &mut Arc::make_mut(p).tracks {
                for l in t.lines.values_mut() {
                    for n in &mut l.notes {
                        if n.instrument as usize == idx {
                            n.instrument = EMPTY;
                        } else if n.instrument != EMPTY && n.instrument as usize > idx {
                            n.instrument -= 1;
                        }
                    }
                }
            }
        }
        self.instrument = self.instrument.min(self.song.instruments.len().saturating_sub(1) as u8);
        self.sync();
    }

    pub fn instrument_rename(&mut self, name: String) {
        let idx = self.instrument as usize;
        if idx < self.song.instruments.len() {
            self.checkpoint();
            self.song_mut().instruments[idx].name = name;
            self.sync();
        }
    }

    pub fn instrument_set_volume(&mut self, volume: f32) {
        let idx = self.instrument as usize;
        if idx < self.song.instruments.len() {
            self.checkpoint_key("inst_volume");
            self.song_mut().instruments[idx].volume = volume.clamp(0.0, 4.0);
            self.sync();
        }
    }

    /// Edit one sample of the selected instrument (undoable).
    pub fn edit_sample<F: FnOnce(&mut Sample)>(&mut self, sample: usize, f: F) {
        let idx = self.instrument as usize;
        if self.song.instruments.get(idx).and_then(|i| i.samples.get(sample)).is_none() {
            return;
        }
        self.checkpoint();
        f(&mut self.song_mut().instruments[idx].samples[sample]);
        self.sync();
    }

    /// Route the selected instrument's notes to MIDI output (`None` plays samples only).
    pub fn instrument_set_midi(&mut self, midi: Option<trakr_core::MidiOut>) {
        self.edit_instrument("inst_midi", |i| i.midi_out = midi);
    }

    pub fn current_instrument(&self) -> Option<&Instrument> {
        self.song.instruments.get(self.instrument as usize)
    }

    /// Play the selected instrument at `note` without writing to the pattern.
    pub fn preview_note(&mut self, note: u8) {
        let (track, instrument) = (self.cursor.track, self.instrument);
        self.send(Command::NoteOn { track, instrument, note, volume: 1.0, micro: 0 });
    }
}

impl Editor {
    /// Like `edit_sample`, but drags of the same `key` collapse into a single undo step.
    pub fn edit_sample_coalesced<F: FnOnce(&mut Sample)>(&mut self, key: &'static str, sample: usize, f: F) {
        let idx = self.instrument as usize;
        if self.song.instruments.get(idx).and_then(|i| i.samples.get(sample)).is_none() {
            return;
        }
        self.checkpoint_key(key);
        f(&mut self.song_mut().instruments[idx].samples[sample]);
        self.sync();
    }
}

impl Editor {
    /// Edit the selected instrument; repeated calls with the same `key` (e.g. dragging an envelope
    /// point) share one undo step.
    pub fn edit_instrument<F: FnOnce(&mut Instrument)>(&mut self, key: &'static str, f: F) {
        let idx = self.instrument as usize;
        if idx >= self.song.instruments.len() {
            return;
        }
        self.checkpoint_key(key);
        f(&mut self.song_mut().instruments[idx]);
        self.sync();
    }
}
