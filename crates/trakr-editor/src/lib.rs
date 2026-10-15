//! Frontend-neutral editor: cursor, cell entry, selection, undo, transport and song operations.
//! The GUI is a thin renderer over [`Editor`].

mod action;
mod edit_tools;
mod instruments;
mod io;
mod keys;
mod midi;
mod midi_map;
mod mixer_ops;
mod ops;
mod osc_ops;
mod phrase_ops;
mod plugin_ops;
mod prefs;
mod sampling;
mod script_ops;
mod structure_ops;

pub use action::Action;
pub use edit_tools::{CellSpec, Scope};
pub use io::load_song;
pub use keys::{KeyCode, KeyEvent};
pub use midi_map::{MapTarget, MidiMapping};
pub use ops::{Cell, Clipboard};
pub use prefs::{default_config_dir, Prefs};
pub use trakr_encode::{Codec, ExportFormat, MAX_QUALITY};
pub use trakr_engine::InputRecorder;
pub use trakr_midi::{ClockTempo, MidiBackend, MidiEvent, MidiInput, MidiOutMsg, MidiSink};
pub use trakr_script::{API as SCRIPT_API, EXAMPLES as SCRIPT_EXAMPLES};

/// Opens the audio input on demand; supplied by the binary.
pub type RecorderFactory = Box<dyn FnMut() -> anyhow::Result<Box<dyn InputRecorder>>>;

use std::path::PathBuf;
use std::sync::Arc;
use trakr_core::note::note_name;
use trakr_core::{Instrument, Line, Pattern, Song, Track, EMPTY, NOTE_OFF};
use trakr_engine::{Command, EngineHandle};

/// Sub-field counts: note column = note, inst(2), vol(2), pan(2), delay(2); effect column = id(2), amount(2).
pub const NOTE_SUBS: usize = 9;
pub const FX_SUBS: usize = 4;

/// Character ranges of each sub-field inside the text from [`cell_text`], for cursor highlighting.
pub const NOTE_HL: [(usize, usize); NOTE_SUBS] =
    [(0, 3), (4, 5), (5, 6), (7, 8), (8, 9), (10, 11), (11, 12), (13, 14), (14, 15)];
pub const FX_HL: [(usize, usize); FX_SUBS] = [(0, 1), (1, 2), (2, 3), (3, 4)];

const UNDO_LIMIT: usize = 500;

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

/// The sample a convolver slot uses as its impulse response: params 0 and 1 are instrument and sample index.
fn convolver_sample<'a>(
    spec: &trakr_core::DeviceSpec,
    instruments: &'a [trakr_core::Instrument],
) -> Option<&'a trakr_core::Sample> {
    let inst = instruments.get(*spec.params.first()? as usize)?;
    inst.samples.get(*spec.params.get(1)? as usize).filter(|s| !s.data.is_empty())
}

/// What makes a chain slot need rebuilding: its id plus any content the device is built from.
fn chain_key(spec: &trakr_core::DeviceSpec, instruments: &[trakr_core::Instrument]) -> String {
    match spec.id.as_str() {
        "native.formula" => format!("{}\u{1}{}", spec.id, spec.text),
        "native.convolver" => match convolver_sample(spec, instruments) {
            Some(s) => {
                format!("{}\u{1}{}:{}:{}", spec.id, s.data.len(), s.data.as_ptr() as usize, s.sample_rate)
            }
            None => spec.id.clone(),
        },
        _ => spec.id.clone(),
    }
}

/// Build the device for one chain slot: a native device, a hosted plugin, or a pass-through when it
/// cannot be created (the reason is appended to `errors`).
fn make_device(
    host: &mut Option<Box<dyn trakr_engine::PluginHost>>,
    spec: &trakr_core::DeviceSpec,
    instruments: &[trakr_core::Instrument],
    rate: u32,
    errors: &mut Vec<String>,
) -> Box<dyn trakr_engine::Device> {
    if spec.id == "native.formula" {
        let (dev, err) = trakr_engine::create_formula(&spec.text, rate as f32);
        if let Some(e) = err {
            errors.push(format!("formula: {e}"));
        }
        return dev;
    }
    if spec.id == "native.convolver" {
        if let Some(s) = convolver_sample(spec, instruments) {
            return trakr_engine::create_convolver(&s.data, s.channels as usize, s.sample_rate, rate);
        }
    }
    if let Some(p) = &spec.plugin {
        let loaded = match host {
            Some(h) => h.load(&p.path, &p.plugin_id, rate),
            None => Err("this build cannot host plugins".to_string()),
        };
        return match loaded {
            Ok((dev, _)) => dev,
            Err(e) => {
                errors.push(format!("plugin {} not loaded: {e}", p.name));
                trakr_engine::devices::create_or_passthrough("", rate)
            }
        };
    }
    trakr_engine::devices::create_or_passthrough(&spec.id, rate)
}

/// Cursor inside the phrase editor. `col` counts note columns first, then effect columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhraseCursor {
    pub line: u16,
    pub col: usize,
    pub sub: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub track: usize,
    /// Note columns first, then effect columns.
    pub col: usize,
    pub sub: usize,
    pub line: u16,
}

/// Everything an undoable edit can change. Patterns are `Arc`s, so a snapshot is cheap.
#[derive(Clone)]
struct Snapshot {
    patterns: Vec<Arc<Pattern>>,
    sequence: Vec<usize>,
    tracks: Vec<Track>,
    instruments: Vec<Instrument>,
    loop_range: Option<(usize, usize)>,
    slot_meta: Vec<trakr_core::SlotMeta>,
    groove: Vec<f32>,
    bpm: f32,
    lpb: u8,
    tpl: u8,
}

/// Rectangular block in flattened-column space (all visible columns of all tracks, left to right).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub col0: usize,
    pub col1: usize,
    pub line0: u16,
    pub line1: u16,
}

pub struct Editor {
    pub song: Arc<Song>,
    pub cursor: Cursor,
    pub seq_pos: usize,
    pub octave: u8,
    pub edit_step: u16,
    pub instrument: u8,
    pub edit_mode: bool,
    /// When true, the cursor tracks the playhead while the song plays.
    pub follow: bool,
    pub engine: Option<EngineHandle>,
    pub status: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub quit: bool,
    pub clipboard: Option<Clipboard>,
    /// Selected sample within the selected instrument.
    pub sample: usize,
    /// Selected phrase within the selected instrument, and the cursor inside it.
    pub phrase: usize,
    pub phrase_cursor: PhraseCursor,
    sample_clip: Option<(Vec<f32>, u8)>,
    recorder: Option<Box<dyn InputRecorder>>,
    recorder_factory: Option<RecorderFactory>,
    rec_data: Vec<f32>,
    /// Recent input peak (0..1), for a level meter.
    pub rec_peak: f32,
    /// Selection anchor as (flattened column, line); the other corner is the cursor.
    anchor: Option<(usize, u16)>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    needs_sync: bool,
    /// Key of the last coalescable edit (e.g. dragging a tempo field); cleared by any other checkpoint.
    last_key: Option<&'static str>,
    /// Device ids last sent to the engine per track, so only changed chains are rebuilt.
    sent_chains: Vec<Vec<String>>,
    plugin_host: Option<Box<dyn trakr_engine::PluginHost>>,
    plugin_list: Vec<trakr_core::PluginInfo>,
    plugin_errors: Vec<String>,
    midi_backend: Option<Box<dyn trakr_midi::MidiBackend>>,
    midi_in: Option<Box<dyn trakr_midi::MidiInput>>,
    midi_buf: Vec<trakr_midi::MidiEvent>,
    midi_pump: Option<trakr_midi::pump::Pump>,
    midi_clock: trakr_midi::ClockTempo,
    /// Waiting for the next controller to map to this target.
    pub midi_learn: Option<midi_map::MapTarget>,
    osc: Option<trakr_osc::OscServer>,
    osc_buf: Vec<trakr_osc::Message>,
    /// What the last script printed, and its error if it failed.
    pub script_output: Vec<String>,
    pub script_error: Option<String>,
    /// Set by commands that open a frontend window; the frontend clears them.
    pub palette_requested: bool,
    pub find_requested: bool,
    pub prefs_requested: bool,
    rng: u64,
    config_dir: Option<PathBuf>,
    pub prefs: Prefs,
    keymap: Vec<(KeyEvent, Action)>,
    last_autosave: Option<std::time::Instant>,
    autosave_every: std::time::Duration,
    autosaved: bool,
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
            follow: true,
            engine,
            status,
            path,
            dirty: false,
            quit: false,
            clipboard: None,
            sample: 0,
            phrase: 0,
            phrase_cursor: PhraseCursor::default(),
            sample_clip: None,
            recorder: None,
            recorder_factory: None,
            rec_data: Vec::new(),
            rec_peak: 0.0,
            anchor: None,
            undo: Vec::new(),
            redo: Vec::new(),
            needs_sync: false,
            last_key: None,
            sent_chains: Vec::new(),
            midi_backend: None,
            midi_in: None,
            midi_buf: Vec::new(),
            midi_pump: None,
            midi_clock: Default::default(),
            midi_learn: None,
            osc: None,
            osc_buf: Vec::new(),
            plugin_host: None,
            plugin_list: Vec::new(),
            plugin_errors: Vec::new(),
            script_output: Vec::new(),
            script_error: None,
            palette_requested: false,
            find_requested: false,
            prefs_requested: false,
            rng: 0x9E37_79B9_7F4A_7C15,
            config_dir: None,
            prefs: Prefs::default(),
            keymap: Vec::new(),
            last_autosave: None,
            autosave_every: std::time::Duration::ZERO,
            autosaved: false,
        };
        app.rebuild_keymap();
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

    /// Playhead as (playing, sequence slot, line); `(false, 0, 0)` without an audio engine.
    pub fn playhead(&self) -> (bool, usize, u16) {
        self.engine.as_ref().map_or((false, 0, 0), |e| e.pos.get())
    }

    // ---- engine glue ----------------------------------------------------

    /// Mark the song changed and push a snapshot to the engine (retried in `tick` if the queue is full).
    fn sync(&mut self) {
        self.dirty = true;
        self.needs_sync = true;
        self.flush_sync();
    }

    fn flush_sync(&mut self) {
        if !self.needs_sync {
            return;
        }
        if self.engine.is_none() {
            self.needs_sync = false;
            return;
        }
        let synced = self.sync_chains();
        if let Some(err) = self.plugin_errors.drain(..).next() {
            self.status = err;
        }
        if !synced {
            return; // queue full: retry from tick()
        }
        if let Some(e) = &mut self.engine {
            if e.send(Command::SetSong(self.song.clone())) {
                self.needs_sync = false;
            }
        }
    }

    /// Call regularly (every frame): retries a failed sync, frees engine garbage, follows the playhead.
    pub fn tick(&mut self) {
        self.flush_sync();
        self.record_poll();
        self.midi_poll();
        self.osc_poll();
        self.autosave_poll();
        if let Some(e) = &mut self.engine {
            e.collect_garbage();
        }
        let (playing, seq, line) = self.playhead();
        if playing && self.follow && seq < self.song.sequence.len() {
            self.seq_pos = seq;
            self.cursor.line = line.min(self.lines_in_pattern().saturating_sub(1));
        }
    }

    /// Rebuild the engine's device chain for every track whose device list changed. Returns false if
    /// the command queue was full (nothing is lost: the diff is recomputed on the next call).
    fn sync_chains(&mut self) -> bool {
        let Some(e) = &mut self.engine else { return true };
        let rate = e.sample_rate;
        let tracks = self.song.tracks.len().min(trakr_engine::MAX_METER_TRACKS);
        self.sent_chains.resize(self.sent_chains.len().max(tracks), Vec::new());
        for t in 0..tracks {
            let ids: Vec<String> =
                self.song.tracks[t].devices.iter().map(|d| chain_key(d, &self.song.instruments)).collect();
            if self.sent_chains[t] == ids {
                continue;
            }
            let chain = self.song.tracks[t]
                .devices
                .iter()
                .map(|d| {
                    make_device(
                        &mut self.plugin_host,
                        d,
                        &self.song.instruments,
                        rate,
                        &mut self.plugin_errors,
                    )
                })
                .collect();
            if !e.send(Command::SetChain { track: t, chain }) {
                return false;
            }
            self.sent_chains[t] = ids;
        }
        true
    }

    fn send(&mut self, cmd: Command) {
        if let Some(e) = &mut self.engine {
            e.send(cmd);
        }
    }

    // ---- undo support ---------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        let s = &self.song;
        Snapshot {
            patterns: s.patterns.clone(),
            sequence: s.sequence.clone(),
            tracks: s.tracks.clone(),
            instruments: s.instruments.clone(),
            loop_range: s.loop_range,
            slot_meta: s.slot_meta.clone(),
            groove: s.groove.clone(),
            bpm: s.bpm,
            lpb: s.lpb,
            tpl: s.tpl,
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        let s = Arc::make_mut(&mut self.song);
        s.patterns = snap.patterns;
        s.sequence = snap.sequence;
        s.tracks = snap.tracks;
        s.instruments = snap.instruments;
        s.loop_range = snap.loop_range;
        s.slot_meta = snap.slot_meta;
        s.groove = snap.groove;
        s.bpm = snap.bpm;
        s.lpb = snap.lpb;
        s.tpl = snap.tpl;
        self.clamp_cursor();
        self.sync();
    }

    /// Call before every user-level mutation of the song.
    fn checkpoint(&mut self) {
        self.last_key = None;
        let snap = self.snapshot();
        self.undo.push(snap);
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Like `checkpoint`, but repeated calls with the same key share one undo step.
    fn checkpoint_key(&mut self, key: &'static str) {
        if self.last_key != Some(key) {
            self.checkpoint();
            self.last_key = Some(key);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            let cur = self.snapshot();
            self.redo.push(cur);
            self.last_key = None;
            self.restore(prev);
            self.status = "undo".into();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let cur = self.snapshot();
            self.undo.push(cur);
            self.last_key = None;
            self.restore(next);
            self.status = "redo".into();
        }
    }

    fn song_mut(&mut self) -> &mut Song {
        Arc::make_mut(&mut self.song)
    }

    fn clamp_cursor(&mut self) {
        if self.seq_pos >= self.song.sequence.len() {
            self.seq_pos = self.song.sequence.len().saturating_sub(1);
        }
        self.cursor.track = self.cursor.track.min(self.song.tracks.len() - 1);
        let (n, f) = self.cols(self.cursor.track);
        self.cursor.col = self.cursor.col.min(n + f - 1);
        self.cursor.sub = self.cursor.sub.min(self.subs(self.cursor.track, self.cursor.col) - 1);
        self.cursor.line = self.cursor.line.min(self.lines_in_pattern().saturating_sub(1));
        if let Some((_, l)) = self.anchor {
            if l >= self.lines_in_pattern() {
                self.anchor = None;
            }
        }
    }

    /// Edit the line under the cursor (undoable).
    fn edit<F: FnOnce(&mut Line)>(&mut self, f: F) {
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint();
        let (t, l) = (self.cursor.track, self.cursor.line);
        let song = Arc::make_mut(&mut self.song);
        let Some(p) = song.patterns.get_mut(pi) else { return };
        let p = Arc::make_mut(p);
        f(p.line_mut(t, l));
        if p.line(t, l).is_some_and(Line::is_empty) {
            p.tracks[t].lines.remove(&l);
        }
        self.sync();
    }

    // ---- movement -------------------------------------------------------

    pub fn move_lines(&mut self, delta: i32) {
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

    /// Place the cursor (e.g. from a mouse click). Out-of-range values are clamped.
    pub fn set_cursor(&mut self, track: usize, col: usize, sub: usize, line: u16) {
        self.anchor = None;
        self.cursor.track = track;
        self.cursor.col = col;
        self.cursor.sub = sub;
        self.cursor.line = line;
        self.clamp_cursor();
    }

    // ---- input ----------------------------------------------------------

    pub fn on_key(&mut self, k: KeyEvent) {
        if let Some(a) = self.action_for_key(k) {
            self.do_action(a);
            return;
        }
        let moving = matches!(
            k.code,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Tab
                | KeyCode::BackTab
        );
        if moving {
            if k.shift {
                self.begin_selection();
            } else {
                self.anchor = None;
            }
        }
        match k.code {
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
            KeyCode::Char(c) if !k.ctrl && !k.alt => self.type_char(c),
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
        self.edit(|l| set_hex_nibble(l, col, sub, nib));
        if sub % 2 == 1 {
            self.cursor.sub += 1;
        } else {
            self.cursor.sub -= 1;
            self.step_down();
        }
    }

    fn effect_field(&mut self, fx: usize, sub: usize, c: char) {
        if !effect_char_ok(sub, c) {
            return;
        }
        self.edit(|l| set_effect_char(l, fx, sub, c));
        if sub < 3 {
            self.cursor.sub += 1;
        } else {
            self.cursor.sub = 0;
            self.step_down();
        }
    }
}

/// Set one hex digit of a note column's instrument (sub 1-2), volume (3-4), panning (5-6) or delay (7-8).
fn set_hex_nibble(l: &mut Line, col: usize, sub: usize, nib: u8) {
    let hi = sub % 2 == 1;
    let set = move |v: u8, empty: u8| {
        let cur = if v == empty && empty == EMPTY { 0 } else { v };
        if hi {
            (cur & 0x0F) | (nib << 4)
        } else {
            (cur & 0xF0) | nib
        }
    };
    let n = &mut l.notes[col];
    match sub {
        1 | 2 => n.instrument = set(n.instrument, EMPTY),
        3 | 4 => n.volume = set(n.volume, EMPTY),
        5 | 6 => n.panning = set(n.panning, EMPTY),
        _ => n.delay = set(n.delay, 0),
    }
}

/// Whether `c` is valid for sub-column `sub` of an effect (two id characters, then two hex digits).
fn effect_char_ok(sub: usize, c: char) -> bool {
    if sub < 2 {
        c.is_ascii_alphanumeric()
    } else {
        c.is_ascii_hexdigit()
    }
}

fn set_effect_char(l: &mut Line, fx: usize, sub: usize, c: char) {
    let up = c.to_ascii_uppercase();
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
        2 => e.amount = (e.amount & 0x0F) | ((c.to_digit(16).unwrap_or(0) as u8) << 4),
        _ => e.amount = (e.amount & 0xF0) | c.to_digit(16).unwrap_or(0) as u8,
    }
}

#[cfg(test)]
mod tests;
