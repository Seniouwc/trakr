//! Frontend-neutral editor state and input handling, shared by the TUI and GUI.

use std::path::PathBuf;
use std::sync::Arc;
use trakr_core::note::note_name;
use trakr_core::{Line, Song, EMPTY, NOTE_EMPTY, NOTE_OFF};
use trakr_engine::{Command, EngineHandle};

/// Frontend-neutral key codes. Each frontend translates its native events into these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    F(u8),
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Tab,
    BackTab,
    Backspace,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyEvent {
    pub fn new(code: KeyCode) -> Self {
        Self { code, ctrl: false, alt: false }
    }
}

/// Text for one column of a line: a note column ("C-4 01 40 80 00") or an effect column ("0A40").
pub fn cell_text(line: Option<&Line>, c: usize, notes: usize) -> String {
    fn hex2(v: u8, empty: u8) -> String {
        if v == empty {
            "..".into()
        } else {
            format!("{v:02X}")
        }
    }
    if c < notes {
        let n = line.map(|l| l.notes[c]).unwrap_or_default();
        format!(
            "{} {} {} {} {}",
            note_name(n.note),
            hex2(n.instrument, EMPTY),
            hex2(n.volume, EMPTY),
            hex2(n.panning, EMPTY),
            hex2(n.delay, 0)
        )
    } else {
        let e = line.map(|l| l.effects[c - notes]).unwrap_or_default();
        if e.is_empty() {
            "....".into()
        } else {
            format!("{}{}{:02X}", e.id[0] as char, e.id[1] as char, e.amount)
        }
    }
}

/// Sub-field counts: note column = note, inst(2), vol(2), pan(2), delay(2); effect column = id(2), amount(2).
pub const NOTE_SUBS: usize = 9;
pub const FX_SUBS: usize = 4;

/// Character ranges of each sub-field inside the text from [`cell_text`], for cursor highlighting.
pub const NOTE_HL: [(usize, usize); NOTE_SUBS] =
    [(0, 3), (4, 5), (5, 6), (7, 8), (8, 9), (10, 11), (11, 12), (13, 14), (14, 15)];
pub const FX_HL: [(usize, usize); FX_SUBS] = [(0, 1), (1, 2), (2, 3), (3, 4)];

#[derive(Clone, Copy, Debug, Default)]
pub struct Cursor {
    pub track: usize,
    /// Note columns first, then effect columns.
    pub col: usize,
    pub sub: usize,
    pub line: u16,
}

pub struct Editor {
    pub song: Arc<Song>,
    pub cursor: Cursor,
    pub seq_pos: usize,
    pub octave: u8,
    pub edit_step: u16,
    pub instrument: u8,
    pub edit_mode: bool,
    pub engine: Option<EngineHandle>,
    pub status: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub quit: bool,
}

fn piano_key(c: char) -> Option<u8> {
    Some(match c {
        'z' => 0,
        's' => 1,
        'x' => 2,
        'd' => 3,
        'c' => 4,
        'v' => 5,
        'g' => 6,
        'b' => 7,
        'h' => 8,
        'n' => 9,
        'j' => 10,
        'm' => 11,
        ',' => 12,
        'l' => 13,
        '.' => 14,
        ';' => 15,
        '/' => 16,
        'q' => 12,
        '2' => 13,
        'w' => 14,
        '3' => 15,
        'e' => 16,
        'r' => 17,
        '5' => 18,
        't' => 19,
        '6' => 20,
        'y' => 21,
        '7' => 22,
        'u' => 23,
        'i' => 24,
        '9' => 25,
        'o' => 26,
        '0' => 27,
        'p' => 28,
        _ => return None,
    })
}

impl Editor {
    pub fn new(song: Song, path: Option<PathBuf>, engine: Option<EngineHandle>) -> Self {
        let status = if engine.is_some() { "ready".into() } else { "no audio device: editing only".into() };
        let mut app = Self {
            song: Arc::new(song),
            cursor: Cursor::default(),
            seq_pos: 0,
            octave: 4,
            edit_step: 1,
            instrument: 0,
            edit_mode: false,
            engine,
            status,
            path,
            dirty: false,
            quit: false,
        };
        app.sync();
        app.dirty = false;
        app
    }

    // ---- layout helpers -------------------------------------------------

    /// (note columns, effect columns) shown for a track. Always at least one column.
    pub fn cols(&self, track: usize) -> (usize, usize) {
        let t = &self.song.tracks[track];
        let n = (t.visible_note_columns as usize).min(trakr_core::MAX_NOTE_COLUMNS);
        let mut f = (t.visible_effect_columns as usize).min(trakr_core::MAX_EFFECT_COLUMNS);
        if n == 0 && f == 0 {
            f = 1;
        }
        (n, f)
    }

    fn subs(&self, track: usize, col: usize) -> usize {
        if col < self.cols(track).0 {
            NOTE_SUBS
        } else {
            FX_SUBS
        }
    }

    pub fn pattern_index(&self) -> Option<usize> {
        self.song.sequence.get(self.seq_pos).copied()
    }

    pub fn lines_in_pattern(&self) -> u16 {
        self.pattern_index().and_then(|i| self.song.patterns.get(i)).map_or(1, |p| p.number_of_lines)
    }

    // ---- engine glue ----------------------------------------------------

    fn sync(&mut self) {
        self.dirty = true;
        if let Some(e) = &mut self.engine {
            e.send(Command::SetSong(self.song.clone()));
        }
    }

    pub fn tick(&mut self) {
        if let Some(e) = &mut self.engine {
            e.collect_garbage();
        }
    }

    fn send(&mut self, cmd: Command) {
        if let Some(e) = &mut self.engine {
            e.send(cmd);
        }
    }

    fn edit<F: FnOnce(&mut Line)>(&mut self, f: F) {
        let Some(pi) = self.pattern_index() else { return };
        let (t, l) = (self.cursor.track, self.cursor.line);
        let song = Arc::make_mut(&mut self.song);
        let Some(p) = song.patterns.get_mut(pi) else { return };
        f(p.line_mut(t, l));
        self.sync();
    }

    // ---- movement -------------------------------------------------------

    fn move_lines(&mut self, delta: i32) {
        let mut line = self.cursor.line as i32 + delta;
        let mut seq = self.seq_pos as i32;
        let nseq = self.song.sequence.len().max(1) as i32;
        loop {
            let n = self
                .song
                .sequence
                .get(seq as usize)
                .and_then(|&i| self.song.patterns.get(i))
                .map_or(1, |p| p.number_of_lines) as i32;
            if line >= n {
                line -= n;
                seq = (seq + 1) % nseq;
            } else if line < 0 {
                seq = (seq - 1).rem_euclid(nseq);
                let pn = self
                    .song
                    .sequence
                    .get(seq as usize)
                    .and_then(|&i| self.song.patterns.get(i))
                    .map_or(1, |p| p.number_of_lines) as i32;
                line += pn;
            } else {
                break;
            }
        }
        self.seq_pos = seq as usize;
        self.cursor.line = line as u16;
    }

    fn step_down(&mut self) {
        self.move_lines(self.edit_step as i32);
    }

    fn right(&mut self) {
        let c = self.cursor;
        let (n, f) = self.cols(c.track);
        if c.sub + 1 < self.subs(c.track, c.col) {
            self.cursor.sub += 1;
        } else if c.col + 1 < n + f {
            self.cursor.col += 1;
            self.cursor.sub = 0;
        } else if c.track + 1 < self.song.tracks.len() {
            self.cursor.track += 1;
            self.cursor.col = 0;
            self.cursor.sub = 0;
        }
    }

    fn left(&mut self) {
        let c = self.cursor;
        if c.sub > 0 {
            self.cursor.sub -= 1;
        } else if c.col > 0 {
            self.cursor.col -= 1;
            self.cursor.sub = self.subs(c.track, c.col - 1) - 1;
        } else if c.track > 0 {
            let t = c.track - 1;
            let (n, f) = self.cols(t);
            self.cursor.track = t;
            self.cursor.col = n + f - 1;
            self.cursor.sub = self.subs(t, n + f - 1) - 1;
        }
    }

    fn change_track(&mut self, delta: i32) {
        let t = (self.cursor.track as i32 + delta).clamp(0, self.song.tracks.len() as i32 - 1) as usize;
        self.cursor.track = t;
        self.cursor.col = 0;
        self.cursor.sub = 0;
    }

    // ---- input ----------------------------------------------------------

    pub fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.ctrl;
        match k.code {
            KeyCode::Char('q') if ctrl => self.quit = true,
            KeyCode::Char('s') if ctrl => self.save(),
            KeyCode::F(5) => self.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false }),
            KeyCode::F(6) => {
                let seq_pos = self.seq_pos;
                self.send(Command::Play { seq_pos, line: 0, loop_pattern: true });
            }
            KeyCode::F(7) => {
                let (seq_pos, line) = (self.seq_pos, self.cursor.line);
                self.send(Command::Play { seq_pos, line, loop_pattern: false });
            }
            KeyCode::F(8) => self.send(Command::Stop),
            KeyCode::F(1) => self.octave = self.octave.saturating_sub(1),
            KeyCode::F(2) => self.octave = (self.octave + 1).min(8),
            KeyCode::F(3) => self.edit_step = self.edit_step.saturating_sub(1),
            KeyCode::F(4) => self.edit_step = (self.edit_step + 1).min(64),
            KeyCode::F(9) => self.instrument = self.instrument.saturating_sub(1),
            KeyCode::F(10) => self.instrument = self.instrument.saturating_add(1).min(0xFE),
            KeyCode::Up => self.move_lines(-1),
            KeyCode::Down => self.move_lines(1),
            KeyCode::PageUp => self.move_lines(-16),
            KeyCode::PageDown => self.move_lines(16),
            KeyCode::Home => self.cursor.line = 0,
            KeyCode::End => self.cursor.line = self.lines_in_pattern() - 1,
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Tab => self.change_track(1),
            KeyCode::BackTab => self.change_track(-1),
            KeyCode::Backspace | KeyCode::Delete => self.clear_field(),
            KeyCode::Char(' ') => {
                self.edit_mode = !self.edit_mode;
                self.status = if self.edit_mode { "edit mode on".into() } else { "edit mode off".into() };
            }
            KeyCode::Char(c) if !ctrl && !k.alt => self.type_char(c),
            _ => {}
        }
    }

    fn type_char(&mut self, c: char) {
        let c = c.to_ascii_lowercase();
        let Cursor { track, col, sub, .. } = self.cursor;
        let (notes, _) = self.cols(track);
        if col < notes && sub == 0 {
            let note = if c == '`' {
                Some(NOTE_OFF)
            } else {
                piano_key(c).map(|s| (self.octave as u32 * 12 + s as u32).min(119) as u8)
            };
            if let Some(note) = note {
                let inst = self.instrument;
                if note < NOTE_OFF {
                    self.send(Command::NoteOn { track, instrument: inst, note, volume: 1.0 });
                }
                if self.edit_mode {
                    self.edit(|l| {
                        let n = &mut l.notes[col];
                        n.note = note;
                        if note < NOTE_OFF {
                            n.instrument = inst;
                        }
                    });
                    self.step_down();
                }
            }
            return;
        }
        if !self.edit_mode {
            return;
        }
        if col < notes {
            if let Some(d) = c.to_digit(16) {
                self.hex_note_field(col, sub, d as u8);
            }
        } else {
            self.effect_field(col - notes, sub, c);
        }
    }

    fn hex_note_field(&mut self, col: usize, sub: usize, nib: u8) {
        let hi = sub % 2 == 1;
        let set = move |v: u8, empty: u8| {
            let cur = if v == empty && empty == EMPTY { 0 } else { v };
            if hi {
                (cur & 0x0F) | (nib << 4)
            } else {
                (cur & 0xF0) | nib
            }
        };
        self.edit(|l| {
            let n = &mut l.notes[col];
            match sub {
                1 | 2 => n.instrument = set(n.instrument, EMPTY),
                3 | 4 => n.volume = set(n.volume, EMPTY),
                5 | 6 => n.panning = set(n.panning, EMPTY),
                _ => n.delay = set(n.delay, 0),
            }
        });
        if hi {
            self.cursor.sub += 1;
        } else {
            self.cursor.sub -= 1;
            self.step_down();
        }
    }

    fn effect_field(&mut self, fx: usize, sub: usize, c: char) {
        let up = c.to_ascii_uppercase();
        if sub < 2 && !up.is_ascii_alphanumeric() {
            return;
        }
        if sub >= 2 && c.to_digit(16).is_none() {
            return;
        }
        self.edit(|l| {
            let e = &mut l.effects[fx];
            match sub {
                0 => {
                    e.id[0] = up as u8;
                    if e.id[1] == 0 {
                        e.id[1] = b'0';
                    }
                }
                1 => {
                    e.id[1] = up as u8;
                    if e.id[0] == 0 {
                        e.id[0] = b'0';
                    }
                }
                2 => e.amount = (e.amount & 0x0F) | ((c.to_digit(16).unwrap() as u8) << 4),
                _ => e.amount = (e.amount & 0xF0) | c.to_digit(16).unwrap() as u8,
            }
        });
        if sub < 3 {
            self.cursor.sub += 1;
        } else {
            self.cursor.sub = 0;
            self.step_down();
        }
    }

    fn clear_field(&mut self) {
        if !self.edit_mode {
            return;
        }
        let Cursor { track, col, sub, .. } = self.cursor;
        let (notes, _) = self.cols(track);
        self.edit(|l| {
            if col < notes {
                let n = &mut l.notes[col];
                match sub {
                    0 => {
                        n.note = NOTE_EMPTY;
                        n.instrument = EMPTY;
                        n.volume = EMPTY;
                        n.panning = EMPTY;
                        n.delay = 0;
                    }
                    1 | 2 => n.instrument = EMPTY,
                    3 | 4 => n.volume = EMPTY,
                    5 | 6 => n.panning = EMPTY,
                    _ => n.delay = 0,
                }
            } else {
                l.effects[col - notes] = Default::default();
            }
        });
        self.step_down();
    }

    fn save(&mut self) {
        let path = self.path.clone().unwrap_or_else(|| PathBuf::from("untitled.trkr"));
        let mut song = (*self.song).clone();
        song.patterns.iter_mut().for_each(|p| p.compact());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c))
    }

    #[test]
    fn note_entry_writes_and_advances() {
        let mut app = Editor::new(Song::default(), None, None);
        app.on_key(key(' '));
        app.on_key(key('z'));
        let line = app.song.patterns[0].line(0, 0).unwrap();
        assert_eq!(line.notes[0].note, 48);
        assert_eq!(line.notes[0].instrument, 0);
        assert_eq!(app.cursor.line, 1);
    }

    #[test]
    fn nothing_written_when_edit_mode_off() {
        let mut app = Editor::new(Song::default(), None, None);
        app.on_key(key('z'));
        assert!(app.song.patterns[0].line(0, 0).is_none());
    }

    #[test]
    fn hex_entry_in_volume_column() {
        let mut app = Editor::new(Song::default(), None, None);
        app.on_key(key(' '));
        for _ in 0..3 {
            app.on_key(KeyEvent::new(KeyCode::Right));
        }
        app.on_key(key('4'));
        app.on_key(key('0'));
        assert_eq!(app.song.patterns[0].line(0, 0).unwrap().notes[0].volume, 0x40);
    }

    #[test]
    fn down_wraps_through_sequence() {
        let mut app = Editor::new(Song::default(), None, None);
        app.on_key(KeyEvent::new(KeyCode::Up));
        assert_eq!(app.cursor.line, 63);
    }
}
