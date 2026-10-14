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
                self.status = format!("imported {name}");
                self.sync();
            }
            Err(e) => self.status = format!("import failed: {e}"),
        }
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

    pub fn current_instrument(&self) -> Option<&Instrument> {
        self.song.instruments.get(self.instrument as usize)
    }

    /// Play the selected instrument at `note` without writing to the pattern.
    pub fn preview_note(&mut self, note: u8) {
        let (track, instrument) = (self.cursor.track, self.instrument);
        self.send(Command::NoteOn { track, instrument, note, volume: 1.0 });
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
