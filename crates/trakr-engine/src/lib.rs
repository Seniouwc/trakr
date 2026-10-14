//! Real-time audio engine: sequencer + sample voices + mixer.
//!
//! RT rules for everything reachable from [`Engine::render`]:
//! no allocation, no locks, no IO, no blocking. Talk to the UI thread only
//! through the lock-free queues in [`EngineHandle`] and the atomics in [`SharedPos`].

#[cfg(feature = "audio")]
pub mod audio;
mod voice;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use trakr_core::{EffectCmd, MuteState, Song, EMPTY, NOTE_EMPTY, NOTE_OFF};
use voice::{TriggerOpts, Voice};

const MAX_VOICES: usize = 128;
const QUEUE_CAP: usize = 256;
const MAX_EVENTS: usize = 512;

/// A sub-line event scheduled by the delay column or timing effects.
struct Event {
    /// Frames until it fires.
    at: f64,
    track: usize,
    column: usize,
    kind: EventKind,
}

enum EventKind {
    NoteOn { inst: u8, note: u8, vol: f32, pan: f32, opts: TriggerOpts },
    Release,
}

/// Commands from the UI thread to the audio thread.
pub enum Command {
    /// Swap in a new song snapshot. TODO: replace with incremental edit commands.
    SetSong(Arc<Song>),
    Play {
        seq_pos: usize,
        line: u16,
        loop_pattern: bool,
    },
    Stop,
    /// When true, playback stops (voices keep ringing out) once the sequence ends instead of looping.
    SetStopAtEnd(bool),
    /// Live/preview note, outside the sequencer.
    NoteOn {
        track: usize,
        instrument: u8,
        note: u8,
        volume: f32,
    },
    NoteOff {
        track: usize,
        note: u8,
    },
}

/// A live audio input (microphone, line in) that can be drained from the UI thread.
/// The cpal-backed implementation lives in `audio` (feature `audio`).
pub trait InputRecorder {
    fn sample_rate(&self) -> u32;
    /// 1 or 2.
    fn channels(&self) -> u8;
    /// Append everything captured since the last call (interleaved f32) to `out`.
    fn drain(&mut self, out: &mut Vec<f32>);
}

/// Playback position published by the audio thread.
#[derive(Default)]
pub struct SharedPos {
    pub playing: AtomicBool,
    pub seq_pos: AtomicU32,
    pub line: AtomicU32,
}

impl SharedPos {
    pub fn get(&self) -> (bool, usize, u16) {
        (
            self.playing.load(Ordering::Relaxed),
            self.seq_pos.load(Ordering::Relaxed) as usize,
            self.line.load(Ordering::Relaxed) as u16,
        )
    }
}

/// UI-thread side of the engine.
pub struct EngineHandle {
    tx: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Arc<Song>>,
    pub pos: Arc<SharedPos>,
    pub sample_rate: u32,
}

impl EngineHandle {
    /// Returns false if the queue is full (command dropped).
    pub fn send(&mut self, cmd: Command) -> bool {
        self.tx.push(cmd).is_ok()
    }

    /// Drop song snapshots the audio thread is done with (keeps deallocation off the RT thread).
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }
}

pub fn create(sample_rate: u32) -> (Engine, EngineHandle) {
    let (tx, rx) = rtrb::RingBuffer::new(QUEUE_CAP);
    let (gtx, grx) = rtrb::RingBuffer::new(QUEUE_CAP);
    let pos = Arc::new(SharedPos::default());
    let song = Arc::new(Song::default());
    let engine = Engine {
        sample_rate,
        bpm: song.bpm,
        lpb: song.lpb,
        song,
        rx,
        garbage: gtx,
        pos: pos.clone(),
        playing: false,
        loop_pattern: false,
        seq_pos: 0,
        line: 0,
        frames_to_line: 0.0,
        voices: Vec::with_capacity(MAX_VOICES),
        events: Vec::with_capacity(MAX_EVENTS),
        pending_break: None,
        stop_at_end: false,
    };
    (engine, EngineHandle { tx, garbage: grx, pos, sample_rate })
}

pub struct Engine {
    sample_rate: u32,
    song: Arc<Song>,
    bpm: f32,
    lpb: u8,
    rx: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Arc<Song>>,
    pos: Arc<SharedPos>,
    playing: bool,
    loop_pattern: bool,
    seq_pos: usize,
    line: u16,
    frames_to_line: f64,
    voices: Vec<Voice>,
    events: Vec<Event>,
    /// Line to jump to in the next pattern slot (ZB command).
    pending_break: Option<u16>,
    stop_at_end: bool,
}

impl Engine {
    /// Render interleaved stereo f32 into `out` (`out.len()` must be even).
    pub fn render(&mut self, out: &mut [f32]) {
        self.drain_commands();
        out.fill(0.0);
        let total = out.len() / 2;
        let mut done = 0;
        while done < total {
            if self.playing && self.frames_to_line <= 0.0 {
                self.trigger_line();
                self.frames_to_line += Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
                self.advance_line();
            }
            self.fire_due_events();
            let mut chunk = total - done;
            if self.playing {
                chunk = chunk.min((self.frames_to_line.ceil() as usize).max(1));
            }
            for e in &self.events {
                chunk = chunk.min((e.at.ceil() as usize).max(1));
            }
            let buf = &mut out[done * 2..(done + chunk) * 2];
            for v in &mut self.voices {
                let track = self.song.tracks.get(v.track);
                let (vol, pan) = track.map_or((1.0, 0.0), |t| {
                    (if t.mute == MuteState::Active { t.volume } else { 0.0 }, t.panning)
                });
                v.mix(buf, self.sample_rate, vol, pan);
            }
            self.voices.retain(|v| v.alive());
            if self.playing {
                self.frames_to_line -= chunk as f64;
            }
            for e in &mut self.events {
                e.at -= chunk as f64;
            }
            done += chunk;
        }
    }

    fn fire_due_events(&mut self) {
        let mut i = 0;
        while i < self.events.len() {
            if self.events[i].at <= 0.0 {
                let e = self.events.remove(i);
                match e.kind {
                    EventKind::NoteOn { inst, note, vol, pan, opts } => {
                        self.note_on(e.track, e.column, inst, note, vol, pan, opts)
                    }
                    EventKind::Release => {
                        for v in self.voices.iter_mut().filter(|v| v.track == e.track && v.column == e.column)
                        {
                            v.release();
                        }
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    fn schedule(&mut self, at: f64, track: usize, column: usize, kind: EventKind) {
        if self.events.len() < MAX_EVENTS {
            self.events.push(Event { at, track, column, kind });
        }
    }

    /// True when nothing is playing, ringing out or scheduled.
    pub fn is_idle(&self) -> bool {
        !self.playing && self.voices.is_empty() && self.events.is_empty()
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rx.pop() {
            match cmd {
                Command::SetSong(new) => {
                    let old = std::mem::replace(&mut self.song, new);
                    // Tempo edits made while playing take effect immediately.
                    if old.bpm != self.song.bpm {
                        self.bpm = self.song.bpm;
                    }
                    if old.lpb != self.song.lpb {
                        self.lpb = self.song.lpb;
                    }
                    // If the garbage queue is full, leak rather than free on the RT thread.
                    if let Err(rtrb::PushError::Full(old)) = self.garbage.push(old) {
                        std::mem::forget(old);
                    }
                    self.clamp_position();
                }
                Command::Play { seq_pos, line, loop_pattern } => {
                    self.bpm = self.song.bpm;
                    self.lpb = self.song.lpb;
                    self.seq_pos = seq_pos;
                    self.line = line;
                    self.clamp_position();
                    self.loop_pattern = loop_pattern;
                    self.frames_to_line = 0.0;
                    self.events.clear();
                    self.pending_break = None;
                    self.playing = true;
                }
                Command::Stop => {
                    self.playing = false;
                    self.voices.clear();
                    self.events.clear();
                    self.pending_break = None;
                }
                Command::SetStopAtEnd(v) => self.stop_at_end = v,
                Command::NoteOn { track, instrument, note, volume } => {
                    self.note_on(track, usize::MAX, instrument, note, volume, 0.0, TriggerOpts::default());
                }
                Command::NoteOff { track, note: _ } => {
                    for v in self.voices.iter_mut().filter(|v| v.track == track) {
                        v.release();
                    }
                }
            }
        }
        self.pos.playing.store(self.playing, Ordering::Relaxed);
    }

    fn clamp_position(&mut self) {
        if self.seq_pos >= self.song.sequence.len() {
            self.seq_pos = 0;
        }
        let lines = self.song.pattern_at(self.seq_pos).map_or(1, |p| p.number_of_lines);
        if self.line >= lines {
            self.line = 0;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn note_on(
        &mut self,
        track: usize,
        column: usize,
        inst: u8,
        note: u8,
        vol: f32,
        pan: f32,
        opts: TriggerOpts,
    ) {
        let Some(sample) =
            self.song.instruments.get(inst as usize).and_then(|i| i.sample_for(note).map(|s| (s, i.volume)))
        else {
            return;
        };
        // Same track+column: cut the previous voice (NNA = cut). TODO: NNA modes.
        for v in self.voices.iter_mut().filter(|v| v.track == track && v.column == column) {
            v.release();
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(Voice::new(track, column, sample.0, sample.1, note, vol, pan, opts));
    }

    fn trigger_line(&mut self) {
        let song = self.song.clone(); // refcount bump only
        let Some(pat) = song.pattern_at(self.seq_pos) else { return };
        let fpl = Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
        let fpt = fpl / song.tpl.max(1) as f64;
        for (ti, track) in pat.tracks.iter().enumerate() {
            let Some(line) = track.lines.get(&self.line) else { continue };

            // Effect commands from the effect columns and the per-note-column slots.
            let mut offset = 0u8;
            let mut backwards = false;
            let mut cut_ticks: Option<u8> = None;
            let mut delay_ticks = 0u8;
            let mut retrig_ticks: Option<u8> = None;
            let mut globals = |fx: &EffectCmd, this: &mut Self| match &fx.id {
                b"ZT" if fx.amount >= 32 => this.bpm = fx.amount as f32,
                b"ZL" if fx.amount >= 1 => this.lpb = fx.amount,
                b"ZB" => this.pending_break = Some(fx.amount as u16),
                b"0S" => offset = fx.amount,
                b"0B" => backwards = fx.amount != 0,
                b"0C" => cut_ticks = Some(fx.amount),
                b"0Q" => delay_ticks = fx.amount,
                b"0R" if fx.amount > 0 => retrig_ticks = Some(fx.amount),
                _ => {} // TODO: the rest of the effect command set (see docs/PARITY.md)
            };
            for fx in line.effects.iter().chain(line.notes.iter().map(|n| &n.effect)) {
                globals(fx, self);
            }

            for (ci, col) in line.notes.iter().enumerate() {
                if col.note == NOTE_OFF {
                    let at = col.delay as f64 / 256.0 * fpl;
                    if at > 0.0 {
                        self.schedule(at, ti, ci, EventKind::Release);
                    } else {
                        for v in self.voices.iter_mut().filter(|v| v.track == ti && v.column == ci) {
                            v.release();
                        }
                    }
                } else if col.note != NOTE_EMPTY && col.instrument != EMPTY {
                    let vol = if col.volume <= 0x80 { col.volume as f32 / 128.0 } else { 1.0 };
                    let pan = if col.panning <= 0x80 { col.panning as f32 / 64.0 - 1.0 } else { 0.0 };
                    let opts = TriggerOpts { offset, backwards };
                    let at = col.delay as f64 / 256.0 * fpl + delay_ticks as f64 * fpt;
                    let kind = EventKind::NoteOn { inst: col.instrument, note: col.note, vol, pan, opts };
                    if at > 0.0 {
                        self.schedule(at, ti, ci, kind);
                    } else {
                        self.note_on(ti, ci, col.instrument, col.note, vol, pan, opts);
                    }
                    if let Some(c) = cut_ticks {
                        self.schedule(at + c as f64 * fpt, ti, ci, EventKind::Release);
                    }
                    if let Some(r) = retrig_ticks {
                        let mut t = at + r as f64 * fpt;
                        while t < fpl {
                            let kind =
                                EventKind::NoteOn { inst: col.instrument, note: col.note, vol, pan, opts };
                            self.schedule(t, ti, ci, kind);
                            t += r as f64 * fpt;
                        }
                    }
                }
            }
        }
        drop(song);
    }

    fn advance_line(&mut self) {
        self.pos.seq_pos.store(self.seq_pos as u32, Ordering::Relaxed);
        self.pos.line.store(self.line as u32, Ordering::Relaxed);
        let lines = self.song.pattern_at(self.seq_pos).map_or(1, |p| p.number_of_lines);
        self.line += 1;
        let brk = self.pending_break.take();
        if self.line >= lines || brk.is_some() {
            self.line = 0;
            if !self.loop_pattern {
                self.seq_pos += 1;
                if self.seq_pos >= self.song.sequence.len() {
                    self.seq_pos = 0; // TODO: song loop range
                    if self.stop_at_end {
                        self.playing = false;
                    }
                }
            }
            if let Some(l) = brk {
                let n = self.song.pattern_at(self.seq_pos).map_or(1, |p| p.number_of_lines);
                self.line = l.min(n - 1);
            }
        }
    }
}

/// Render `song` once through the sequence (plus ring-out) to interleaved stereo f32, offline.
/// Stops at the end of the song, when everything has decayed, or after `max_seconds`.
pub fn render_song(song: Arc<Song>, sample_rate: u32, max_seconds: f32) -> Vec<f32> {
    let (mut engine, mut handle) = create(sample_rate);
    handle.send(Command::SetSong(song));
    handle.send(Command::SetStopAtEnd(true));
    handle.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
    let max_frames = (max_seconds * sample_rate as f32) as usize;
    let mut out: Vec<f32> = Vec::new();
    let mut block = vec![0.0f32; 1024 * 2];
    while out.len() / 2 < max_frames {
        engine.render(&mut block);
        out.extend_from_slice(&block);
        handle.collect_garbage();
        if engine.is_idle() {
            break;
        }
    }
    // Drop the silent tail left by the final block.
    while out.len() >= 2 && out[out.len() - 2..].iter().all(|s| s.abs() < 1e-5) {
        out.truncate(out.len() - 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_song_makes_sound() {
        let (mut engine, mut handle) = create(44_100);
        handle.send(Command::SetSong(Arc::new(Song::demo())));
        handle.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut buf = vec![0.0f32; 2 * 44_100];
        engine.render(&mut buf);
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1, "peak was {peak}");
        assert!(handle.pos.get().0);
    }

    #[test]
    fn render_song_stops_at_the_end() {
        let song = Arc::new(Song::demo()); // 64 lines at 120 BPM / LPB 4 = 8 seconds
        let out = render_song(song, 8_000, 30.0);
        let secs = out.len() as f32 / 2.0 / 8_000.0;
        assert!(secs > 7.0 && secs < 10.0, "rendered {secs}s");
        assert!(out.iter().any(|s| s.abs() > 0.1));
    }

    #[test]
    fn silent_when_stopped() {
        let (mut engine, _h) = create(44_100);
        let mut buf = vec![1.0f32; 512];
        engine.render(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0));
    }
}

#[cfg(test)]
mod sequencer_tests {
    use super::*;
    use trakr_core::{EffectCmd, Instrument, LoopMode, Sample};

    const RATE: u32 = 1000;

    /// Song with one instrument whose sample is `data` at the engine rate (1 frame per frame at C-4),
    /// BPM 60 / LPB 1 so a line lasts exactly one second (1000 frames).
    fn song_with(data: Vec<f32>, loop_mode: LoopMode) -> Song {
        let mut song = Song { bpm: 60.0, lpb: 1, tpl: 4, ..Song::default() };
        let mut inst = Instrument::new("t");
        let mut s = Sample::new("t", RATE, 1, data);
        s.loop_mode = loop_mode;
        inst.samples.push(s);
        song.instruments.push(inst);
        song
    }

    fn put(song: &mut Song, line: u16, f: impl FnOnce(&mut trakr_core::Line)) {
        f(Arc::make_mut(&mut song.patterns[0]).line_mut(0, line));
    }

    fn render(song: Song, frames: usize) -> Vec<f32> {
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        engine.render(&mut out);
        out.chunks_exact(2).map(|f| f[0].max(f[1])).collect()
    }

    fn first_nonzero(v: &[f32]) -> Option<usize> {
        v.iter().position(|s| s.abs() > 1e-6)
    }

    fn note(n: &mut trakr_core::NoteColumn) {
        n.note = 48;
        n.instrument = 0;
    }

    #[test]
    fn delay_column_offsets_the_note() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.notes[0].delay = 128; // half a line = 500 frames
        });
        let out = render(song, 1000);
        assert_eq!(first_nonzero(&out), Some(500));
    }

    #[test]
    fn quantized_delay_effect_uses_ticks() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = EffectCmd { id: *b"0Q", amount: 2 }; // 2 of 4 ticks = 500 frames
        });
        assert_eq!(first_nonzero(&render(song, 1000)), Some(500));
    }

    #[test]
    fn cut_effect_releases_after_ticks() {
        let mut song = song_with(vec![1.0; 900], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.notes[0].effect = EffectCmd { id: *b"0C", amount: 1 }; // 250 frames, then a 5 ms (5 frame) fade
        });
        let out = render(song, 600);
        assert!(out[200] > 0.9);
        assert!(out[400].abs() < 1e-6, "still sounding at 400: {}", out[400]);
    }

    #[test]
    fn sample_offset_skips_into_the_sample() {
        let data: Vec<f32> = (0..256).map(|i| if i < 128 { 0.0 } else { 1.0 }).collect();
        let mut song = song_with(data, LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = EffectCmd { id: *b"0S", amount: 128 }; // half way
        });
        let out = render(song, 10);
        assert!(out[0] > 0.9);
    }

    #[test]
    fn backwards_effect_plays_reversed() {
        let data: Vec<f32> = (0..100).map(|i| i as f32 / 100.0).collect();
        let mut song = song_with(data, LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = EffectCmd { id: *b"0B", amount: 1 };
        });
        let out = render(song, 50);
        assert!(out[0] > 0.9 && out[40] < out[0]);
    }

    #[test]
    fn pattern_break_jumps_to_next_slot_line() {
        let mut song = song_with(vec![1.0; 2], LoopMode::Off);
        song.patterns.push(Arc::new(trakr_core::Pattern::new(song.tracks.len(), 64)));
        song.sequence.push(1);
        put(&mut song, 1, |l| l.effects[0] = EffectCmd { id: *b"ZB", amount: 5 });
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2 * 2500];
        engine.render(&mut out);
        // lines 0,1 play (2000 frames), break lands on slot 1 line 5 at frame 2000
        assert_eq!(h.pos.get().1, 1);
        assert_eq!(h.pos.get().2, 5);
    }

    #[test]
    fn muted_track_is_silent() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        song.tracks[0].mute = MuteState::Muted;
        assert_eq!(first_nonzero(&render(song, 200)), None);
    }

    #[test]
    fn reverse_and_pingpong_loops_keep_sounding() {
        for mode in [LoopMode::Reverse, LoopMode::PingPong, LoopMode::Forward] {
            let mut song = song_with(vec![1.0; 100], mode);
            put(&mut song, 0, |l| note(&mut l.notes[0]));
            let out = render(song, 900);
            assert!(out[800] > 0.9, "{mode:?} stopped early");
        }
    }

    #[test]
    fn tempo_change_applies_while_playing() {
        let song = song_with(vec![1.0; 2], LoopMode::Off);
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song.clone())));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2 * 100];
        engine.render(&mut out);
        let mut faster = song;
        faster.bpm = 120.0;
        h.send(Command::SetSong(Arc::new(faster)));
        let mut out = vec![0.0; 2 * 1500];
        engine.render(&mut out);
        // line 1 at frame 900 (old pace already scheduled), then 500 frames per line: line 2 at 1400
        assert!(h.pos.get().2 >= 2);
    }
}
