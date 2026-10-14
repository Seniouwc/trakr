//! Phrase editing: the phrases of the selected instrument, a cursor, and tracker-style cell entry.

use super::*;
use trakr_core::{Phrase, PhraseMode};

impl Editor {
    pub fn current_phrase(&self) -> Option<&Arc<Phrase>> {
        self.current_instrument()?.phrases.get(self.phrase)
    }

    /// (note columns, effect columns) of the selected phrase.
    pub fn phrase_cols(&self) -> (usize, usize) {
        self.current_phrase().map_or((1, 1), |p| (p.note_columns as usize, p.effect_columns as usize))
    }

    fn phrase_subs(&self, col: usize) -> usize {
        if col < self.phrase_cols().0 {
            NOTE_SUBS
        } else {
            FX_SUBS
        }
    }

    /// Edit the selected phrase (undoable; repeated calls with the same `key` share one undo step).
    pub fn edit_phrase<F: FnOnce(&mut Phrase)>(&mut self, key: &'static str, f: F) {
        let (ii, pi) = (self.instrument as usize, self.phrase);
        if self.song.instruments.get(ii).and_then(|i| i.phrases.get(pi)).is_none() {
            return;
        }
        self.checkpoint_key(key);
        let p = Arc::make_mut(&mut self.song_mut().instruments[ii].phrases[pi]);
        f(p);
        p.sanitize();
        self.clamp_phrase_cursor();
        self.sync();
    }

    pub fn phrase_add(&mut self) {
        let ii = self.instrument as usize;
        if ii >= self.song.instruments.len() {
            self.status = "no instrument selected: import a sample first".into();
            return;
        }
        self.checkpoint();
        let inst = &mut self.song_mut().instruments[ii];
        let n = inst.phrases.len();
        inst.phrases.push(Arc::new(Phrase::new(format!("Phrase {n:02X}"), 16)));
        if inst.phrase_mode == PhraseMode::Off {
            inst.phrase_mode = PhraseMode::Keymap;
        }
        self.phrase = n;
        self.phrase_cursor = PhraseCursor::default();
        self.status = "phrase added: notes in its key range now play it (Instrument > Phrase mode)".into();
        self.sync();
    }

    pub fn phrase_remove(&mut self) {
        let ii = self.instrument as usize;
        if self.current_phrase().is_none() {
            return;
        }
        self.checkpoint();
        let pi = self.phrase;
        self.song_mut().instruments[ii].phrases.remove(pi);
        self.phrase = self.phrase.min(self.song.instruments[ii].phrases.len().saturating_sub(1));
        self.clamp_phrase_cursor();
        self.sync();
    }

    pub fn phrase_duplicate(&mut self) {
        let ii = self.instrument as usize;
        let Some(p) = self.current_phrase().cloned() else { return };
        self.checkpoint();
        let mut copy = (*p).clone();
        copy.name.push_str(" copy");
        let phrases = &mut self.song_mut().instruments[ii].phrases;
        phrases.push(Arc::new(copy));
        self.phrase = phrases.len() - 1;
        self.sync();
    }

    pub fn phrase_set_mode(&mut self, mode: PhraseMode) {
        self.edit_instrument("phrase_mode", |i| i.phrase_mode = mode);
    }

    pub fn phrase_select(&mut self, index: usize) {
        if self.current_instrument().is_some_and(|i| index < i.phrases.len()) {
            self.phrase = index;
            self.clamp_phrase_cursor();
        }
    }

    pub fn phrase_preview(&mut self) {
        let Some(p) = self.current_phrase() else { return };
        let note = p.base_note;
        let (track, instrument, phrase) = (self.cursor.track, self.instrument, self.phrase);
        self.send(Command::PlayPhrase { track, instrument, phrase, note });
    }

    pub fn phrase_stop_preview(&mut self) {
        let track = self.cursor.track;
        self.send(Command::NoteOff { track, note: 0 });
    }

    fn clamp_phrase_cursor(&mut self) {
        let (n, f) = self.phrase_cols();
        let lines = self.current_phrase().map_or(1, |p| p.number_of_lines);
        let c = &mut self.phrase_cursor;
        c.line = c.line.min(lines - 1);
        c.col = c.col.min(n + f - 1);
        c.sub = c.sub.min(if c.col < n { NOTE_SUBS } else { FX_SUBS } - 1);
    }

    pub fn phrase_set_cursor(&mut self, col: usize, sub: usize, line: u16) {
        self.phrase_cursor = PhraseCursor { line, col, sub };
        self.clamp_phrase_cursor();
    }

    fn phrase_edit_line<F: FnOnce(&mut Line)>(&mut self, f: F) {
        let l = self.phrase_cursor.line;
        self.edit_phrase("phrase_cell", |p| {
            f(p.line_mut(l));
            if p.line(l).is_some_and(Line::is_empty) {
                p.lines.remove(&l);
            }
        });
        self.last_key = None; // every cell edit is its own undo step
    }

    fn phrase_step_down(&mut self) {
        let lines = self.current_phrase().map_or(1, |p| p.number_of_lines);
        let c = &mut self.phrase_cursor;
        c.line = (c.line + self.edit_step.max(1)) % lines;
    }

    fn phrase_move(&mut self, dx: i32) {
        let (n, f) = self.phrase_cols();
        let c = self.phrase_cursor;
        let (mut col, mut sub) = (c.col as i32, c.sub as i32);
        sub += dx;
        if sub < 0 {
            if col > 0 {
                col -= 1;
                sub = self.phrase_subs(col as usize) as i32 - 1;
            } else {
                sub = 0;
            }
        } else if sub >= self.phrase_subs(col as usize) as i32 {
            if (col as usize) + 1 < n + f {
                col += 1;
                sub = 0;
            } else {
                sub -= 1;
            }
        }
        self.phrase_cursor.col = col as usize;
        self.phrase_cursor.sub = sub as usize;
    }

    /// Key handling while the phrase editor has focus. Transport and menu shortcuts still go through
    /// `on_key`; this handles movement and cell entry.
    pub fn phrase_key(&mut self, k: KeyEvent) {
        if matches!(k.code, KeyCode::Delete | KeyCode::Backspace) && !k.ctrl && !k.alt {
            if self.edit_mode {
                self.phrase_clear_cell();
            }
            return;
        }
        if let Some(a) = self.action_for_key(k) {
            self.do_action(a);
            return;
        }
        let lines = self.current_phrase().map_or(1, |p| p.number_of_lines) as i32;
        let line = self.phrase_cursor.line as i32;
        match k.code {
            KeyCode::Up => self.phrase_cursor.line = (line - 1).rem_euclid(lines) as u16,
            KeyCode::Down => self.phrase_cursor.line = (line + 1).rem_euclid(lines) as u16,
            KeyCode::PageUp => self.phrase_cursor.line = (line - 16).max(0) as u16,
            KeyCode::PageDown => self.phrase_cursor.line = (line + 16).min(lines - 1) as u16,
            KeyCode::Home => self.phrase_cursor.line = 0,
            KeyCode::End => self.phrase_cursor.line = (lines - 1) as u16,
            KeyCode::Left => self.phrase_move(-1),
            KeyCode::Right => self.phrase_move(1),
            KeyCode::Tab => {
                let (n, f) = self.phrase_cols();
                self.phrase_cursor.col = (self.phrase_cursor.col + 1) % (n + f);
                self.phrase_cursor.sub = 0;
            }
            KeyCode::BackTab => {
                let (n, f) = self.phrase_cols();
                self.phrase_cursor.col = (self.phrase_cursor.col + n + f - 1) % (n + f);
                self.phrase_cursor.sub = 0;
            }
            KeyCode::Char(c) if !k.ctrl && !k.alt => self.phrase_type_char(c.to_ascii_lowercase()),
            _ => {}
        }
        self.clamp_phrase_cursor();
    }

    fn phrase_clear_cell(&mut self) {
        let (n, _) = self.phrase_cols();
        let col = self.phrase_cursor.col;
        self.phrase_edit_line(|l| {
            if col < n {
                l.notes[col] = Default::default();
            } else {
                l.effects[col - n] = Default::default();
            }
        });
        self.phrase_step_down();
    }

    fn phrase_type_char(&mut self, c: char) {
        let PhraseCursor { col, sub, .. } = self.phrase_cursor;
        let (notes, _) = self.phrase_cols();
        if col < notes && sub == 0 {
            let note = if c == '`' {
                Some(NOTE_OFF)
            } else {
                piano_key(c).map(|s| (self.octave as u32 * 12 + s as u32).min(119) as u8)
            };
            if let Some(note) = note {
                if note < NOTE_OFF {
                    let (track, instrument) = (self.cursor.track, self.instrument);
                    self.send(Command::NoteOn { track, instrument, note, volume: 1.0 });
                }
                if self.edit_mode {
                    self.phrase_edit_line(|l| l.notes[col].note = note);
                    self.phrase_step_down();
                }
            }
            return;
        }
        if !self.edit_mode {
            if col < notes || !c.is_ascii() {
                self.status = "edit mode is off (Space toggles it)".into();
            }
            return;
        }
        if col < notes {
            if let Some(d) = c.to_digit(16) {
                self.phrase_edit_line(|l| set_hex_nibble(l, col, sub, d as u8));
                if sub % 2 == 1 {
                    self.phrase_cursor.sub += 1;
                } else {
                    self.phrase_cursor.sub -= 1;
                    self.phrase_step_down();
                }
            }
        } else if effect_char_ok(sub, c) {
            let fx = col - notes;
            self.phrase_edit_line(|l| set_effect_char(l, fx, sub, c));
            if sub < 3 {
                self.phrase_cursor.sub += 1;
            } else {
                self.phrase_cursor.sub = 0;
                self.phrase_step_down();
            }
        }
    }
}
