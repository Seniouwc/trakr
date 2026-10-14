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
use trakr_core::{Song, EMPTY, NOTE_EMPTY, NOTE_OFF};
use voice::Voice;

const MAX_VOICES: usize = 128;
const QUEUE_CAP: usize = 256;

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
            let chunk = if self.playing {
                ((self.frames_to_line.ceil() as usize).max(1)).min(total - done)
            } else {
                total - done
            };
            let buf = &mut out[done * 2..(done + chunk) * 2];
            for v in &mut self.voices {
                let track = self.song.tracks.get(v.track);
                let (vol, pan) = track.map_or((1.0, 0.0), |t| (t.volume, t.panning));
                v.mix(buf, self.sample_rate, vol, pan);
            }
            self.voices.retain(|v| v.alive());
            if self.playing {
                self.frames_to_line -= chunk as f64;
            }
            done += chunk;
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(cmd) = self.rx.pop() {
            match cmd {
                Command::SetSong(new) => {
                    let old = std::mem::replace(&mut self.song, new);
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
                    self.playing = true;
                }
                Command::Stop => {
                    self.playing = false;
                    self.voices.clear();
                }
                Command::NoteOn { track, instrument, note, volume } => {
                    self.note_on(track, usize::MAX, instrument, note, volume, 0.0);
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

    fn note_on(&mut self, track: usize, column: usize, inst: u8, note: u8, vol: f32, pan: f32) {
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
        self.voices.push(Voice::new(track, column, sample.0, sample.1, note, vol, pan));
    }

    fn trigger_line(&mut self) {
        let song = self.song.clone(); // refcount bump only
        let Some(pat) = song.pattern_at(self.seq_pos) else { return };
        for (ti, track) in pat.tracks.iter().enumerate() {
            let Some(line) = track.lines.get(&self.line) else { continue };
            for (ci, col) in line.notes.iter().enumerate() {
                if col.note == NOTE_OFF {
                    for v in self.voices.iter_mut().filter(|v| v.track == ti && v.column == ci) {
                        v.release();
                    }
                } else if col.note != NOTE_EMPTY && col.instrument != EMPTY {
                    let vol = if col.volume <= 0x80 { col.volume as f32 / 128.0 } else { 1.0 };
                    let pan = if col.panning <= 0x80 { col.panning as f32 / 64.0 - 1.0 } else { 0.0 };
                    self.note_on(ti, ci, col.instrument, col.note, vol, pan);
                }
                // TODO: delay column, sample effects, volume/pan column commands.
            }
            for fx in line.effects.iter() {
                match &fx.id {
                    b"ZT" if fx.amount >= 32 => self.bpm = fx.amount as f32,
                    b"ZL" if fx.amount >= 1 => self.lpb = fx.amount,
                    _ => {} // TODO: the rest of the effect command set (see docs/PARITY.md)
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
        if self.line >= lines {
            self.line = 0;
            if !self.loop_pattern {
                self.seq_pos += 1;
                if self.seq_pos >= self.song.sequence.len() {
                    self.seq_pos = 0; // TODO: song loop range / stop at end
                }
            }
        }
    }
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
    fn silent_when_stopped() {
        let (mut engine, _h) = create(44_100);
        let mut buf = vec![1.0f32; 512];
        engine.render(&mut buf);
        assert!(buf.iter().all(|s| *s == 0.0));
    }
}
