//! Real-time audio engine: sequencer + sample voices + mixer.
//!
//! RT rules for everything reachable from [`Engine::render`]:
//! no allocation, no locks, no IO, no blocking. Talk to the UI thread only
//! through the lock-free queues in [`EngineHandle`] and the atomics in [`SharedPos`].

#[cfg(feature = "audio")]
pub mod audio;
pub mod devices;
mod devices_adv;
mod devices_extra;
mod devices_more;
mod dsp;
pub mod formula;
mod voice;

pub use devices::Device;
pub use devices_adv::{create_convolver, create_formula};

/// Loads hosted plugins for the UI thread (the binary supplies a real one, e.g. `trakr-clap`).
pub trait PluginHost {
    /// Plugins found on this machine.
    fn scan(&mut self) -> Vec<trakr_core::PluginInfo>;

    /// Instantiate a plugin as an effect at `sample_rate`. Returns the device plus a description of the
    /// plugin and its parameters for the song to store.
    fn load(
        &mut self,
        path: &str,
        plugin_id: &str,
        sample_rate: u32,
    ) -> Result<(Box<dyn Device>, trakr_core::PluginRef), String>;

    /// Like `load`, restoring saved `state` (empty = none) and also returning the main-thread handle
    /// for the plugin's window, state and parameter changes. Hosts without those keep the default.
    fn load_with_gui(
        &mut self,
        path: &str,
        plugin_id: &str,
        sample_rate: u32,
        _state: &[u8],
    ) -> Result<LoadedPlugin, String> {
        self.load(path, plugin_id, sample_rate).map(|(device, reference)| (device, reference, None))
    }

    /// Run the plugins' main-thread work (timers, file descriptors, callbacks). Call every UI frame.
    fn poll(&mut self) {}
}

/// A freshly loaded plugin: its audio device, its description and, when it has one, its UI handle.
pub type LoadedPlugin = (Box<dyn Device>, trakr_core::PluginRef, Option<Box<dyn PluginGui>>);

/// Main-thread handle to a hosted plugin: its window, saved state and parameter changes made inside it.
pub trait PluginGui {
    /// Whether the plugin can open a window of its own.
    fn has_window(&self) -> bool;
    /// Open (or raise) the plugin's window.
    fn show(&mut self) -> Result<(), String>;
    fn hide(&mut self);
    fn is_visible(&self) -> bool;
    /// True once after the plugin says its state changed (a knob turned in its window, a preset loaded).
    fn take_state_dirty(&mut self) -> bool;
    /// The plugin's state as opaque bytes (None when it has none or cannot save).
    fn save_state(&mut self) -> Option<Vec<u8>>;
    /// Parameter values the plugin changed itself since the last call, as (parameter index, value).
    fn take_param_changes(&mut self, out: &mut Vec<(usize, f32)>);
}

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use trakr_core::{
    AutoTarget, EffectCmd, MuteState, Phrase, Song, EMPTY, MAX_NOTE_COLUMNS, NOTE_EMPTY, NOTE_OFF,
};
use trakr_midi::MidiOutMsg;
use voice::{TriggerOpts, Voice};

const MAX_VOICES: usize = 512;
const QUEUE_CAP: usize = 256;
/// Room for the voices a stop or a dense block can retire at once.
const GARBAGE_CAP: usize = 2048;
/// Longest plugin latency that is compensated (frames, about 0.19 s at 44.1 kHz).
const PDC_FRAMES: usize = 8192;
const MAX_EVENTS: usize = 512;
/// Samples of audio processed per mixing block (also the granularity of automation).
const BLOCK: usize = 512;
/// Tracks that get their own mix bus and device chain.
const MAX_TRACKS: usize = MAX_METER_TRACKS;
const MAX_OVERRIDES: usize = 256;
/// Chain slots per track whose modulation output is kept (and that modulators can target).
const MAX_MOD_DEVS: usize = 32;
/// Modulators applied per track and block.
const MAX_MODS: usize = 16;
/// Messages waiting for the MIDI output thread.
const MIDI_QUEUE: usize = 4096;
/// MIDI notes sounding at once.
const MAX_MIDI_NOTES: usize = 128;
/// Phrases playing at once.
const MAX_PHRASE_RUNS: usize = 32;
/// Voice column ids at and above this belong to phrase playback (one block of columns per trigger column).
const PHRASE_COL: usize = 1000;
/// Pseudo pattern column for phrases started from outside the sequencer.
const LIVE_COL: usize = MAX_NOTE_COLUMNS;

/// Things handed back to the UI thread so they are freed there, not on the audio thread.
/// (Voices travel by value: boxing one would allocate on the audio thread.)
#[allow(clippy::large_enum_variant)]
pub enum Garbage {
    Song(Arc<Song>),
    Chain(Vec<Box<dyn Device>>),
    Device(Box<dyn Device>),
    Phrase(Arc<Phrase>),
    /// A finished voice: it may hold the last reference to a sample or envelope.
    Voice(RetiredVoice),
}

/// A voice on its way to the UI thread for dropping.
pub struct RetiredVoice(#[allow(dead_code)] Voice);

const MAX_INSTRUMENT_PLUGINS: usize = 255;
const MAX_PLUGIN_NOTES: usize = 256;

/// A note held on an instrument plugin.
#[derive(Clone, Copy)]
struct PluginNote {
    inst: u8,
    track: usize,
    column: usize,
    key: u8,
}

/// A note sent to the MIDI output that has not been released yet.
#[derive(Clone, Copy)]
struct MidiNote {
    track: usize,
    column: usize,
    channel: u8,
    note: u8,
}

/// A parameter value forced by pattern automation for the current block.
struct Override {
    track: usize,
    target: Ov,
    value: f32,
}

enum Ov {
    Volume,
    Pan,
    Param { device: usize, param: usize },
}

/// A sub-line event scheduled by the delay column or timing effects.
struct Event {
    /// Frames until it fires.
    at: f64,
    track: usize,
    column: usize,
    kind: EventKind,
}

/// A phrase that is currently playing.
struct PhraseRun {
    track: usize,
    /// Pattern column that triggered it.
    col: usize,
    inst: u8,
    phrase: Arc<Phrase>,
    /// Semitones between the trigger note and the phrase's base note.
    shift: i32,
    vol: f32,
    pan: f32,
    line: u16,
    frames_to_line: f64,
}

enum EventKind {
    NoteOn { inst: u8, note: u8, vol: f32, pan: f32, opts: TriggerOpts, fx: [EffectCmd; 3] },
    Phrase { inst: u8, phrase: usize, note: u8, vol: f32, pan: f32 },
    Release,
}

/// Commands from the UI thread to the audio thread.
pub enum Command {
    /// Swap in a new song snapshot. TODO: replace with incremental edit commands.
    SetSong(Arc<Song>),
    /// Replace one track's device chain with freshly built devices (see [`devices::create`]).
    SetChain {
        track: usize,
        chain: Vec<Box<dyn Device>>,
    },
    Play {
        seq_pos: usize,
        line: u16,
        loop_pattern: bool,
    },
    /// Loop lines `first..=last` of one slot until stopped.
    PlayBlock {
        seq_pos: usize,
        first: u16,
        last: u16,
    },
    Stop,
    /// Install (or remove) the instrument plugin for one instrument slot. The device receives that
    /// instrument's notes; its audio joins the track the notes were played on.
    SetInstrumentPlugin {
        instrument: usize,
        device: Option<Box<dyn Device>>,
    },
    /// Pitch wheel: offset in semitones applied to every sounding voice (0 = centred).
    PitchBend(f32),
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
    /// Turn MIDI output on or off, and whether to send timing clock plus start/stop while playing.
    SetMidiOut {
        enabled: bool,
        clock: bool,
    },
    /// Audition one phrase of an instrument, whatever the instrument's phrase mode.
    PlayPhrase {
        track: usize,
        instrument: u8,
        phrase: usize,
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

/// Most tracks the level meters cover.
pub const MAX_METER_TRACKS: usize = 128;

/// Playback position and level meters published by the audio thread.
pub struct SharedPos {
    pub playing: AtomicBool,
    pub seq_pos: AtomicU32,
    pub line: AtomicU32,
    /// Frames rendered since the engine started (timestamps MIDI output).
    frames: AtomicU64,
    /// Fraction of real time the last render calls took (1.0 = no headroom), as f32 bits.
    cpu: AtomicU32,
    /// Notes sounding at the end of the last render call.
    voice_count: AtomicU32,
    /// Per-track peak of the most recent render call, as f32 bits.
    meters: Vec<AtomicU32>,
    master: [AtomicU32; 2],
}

impl Default for SharedPos {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            seq_pos: AtomicU32::new(0),
            line: AtomicU32::new(0),
            frames: AtomicU64::new(0),
            cpu: AtomicU32::new(0),
            voice_count: AtomicU32::new(0),
            meters: (0..MAX_METER_TRACKS).map(|_| AtomicU32::new(0)).collect(),
            master: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }
}

impl SharedPos {
    pub fn get(&self) -> (bool, usize, u16) {
        (
            self.playing.load(Ordering::Relaxed),
            self.seq_pos.load(Ordering::Relaxed) as usize,
            self.line.load(Ordering::Relaxed) as u16,
        )
    }

    /// Peak (0..) of a track during the last audio buffer.
    pub fn track_peak(&self, track: usize) -> f32 {
        self.meters.get(track).map_or(0.0, |m| f32::from_bits(m.load(Ordering::Relaxed)))
    }

    /// Audio thread load: 0.0 idle, 1.0 means rendering takes as long as the audio it makes.
    pub fn cpu_load(&self) -> f32 {
        f32::from_bits(self.cpu.load(Ordering::Relaxed))
    }

    /// Notes sounding right now.
    pub fn voices(&self) -> u32 {
        self.voice_count.load(Ordering::Relaxed)
    }

    /// (left, right) peak of the final mix during the last audio buffer.
    pub fn master_peak(&self) -> (f32, f32) {
        (
            f32::from_bits(self.master[0].load(Ordering::Relaxed)),
            f32::from_bits(self.master[1].load(Ordering::Relaxed)),
        )
    }
}

/// UI-thread side of the engine.
pub struct EngineHandle {
    tx: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Garbage>,
    pub pos: Arc<SharedPos>,
    pub sample_rate: u32,
    /// Messages for MIDI output. Hand it to a `trakr_midi::pump` thread; `None` while one has it.
    pub midi_out: Option<MidiOutSource>,
}

/// The consumer side of the engine's MIDI output queue.
pub struct MidiOutSource {
    rx: rtrb::Consumer<(u64, MidiOutMsg)>,
    pos: Arc<SharedPos>,
}

impl trakr_midi::pump::Source for MidiOutSource {
    fn poll(&mut self) -> Option<(u64, MidiOutMsg)> {
        self.rx.pop().ok()
    }

    fn frames_rendered(&self) -> u64 {
        self.pos.frames.load(Ordering::Acquire)
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

impl MidiOutSource {
    /// Recover a source handed back by `trakr_midi::pump::Pump::stop`.
    pub fn from_boxed(src: Box<dyn trakr_midi::pump::Source>) -> Option<Self> {
        src.into_any().downcast::<Self>().ok().map(|b| *b)
    }
}

impl EngineHandle {
    /// Returns false if the queue is full (command dropped).
    pub fn send(&mut self, cmd: Command) -> bool {
        self.tx.push(cmd).is_ok()
    }

    /// Drop song snapshots and device chains the audio thread is done with (keeps deallocation off the RT thread).
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
    }
}

pub fn create(sample_rate: u32) -> (Engine, EngineHandle) {
    voice::init_tables();
    let (tx, rx) = rtrb::RingBuffer::new(QUEUE_CAP);
    let (gtx, grx) = rtrb::RingBuffer::new(GARBAGE_CAP);
    let (mtx, mrx) = rtrb::RingBuffer::new(MIDI_QUEUE);
    let pos = Arc::new(SharedPos::default());
    let song = Arc::new(Song::default());
    let song_tpl = song.tpl;
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
        block: None,
        seq_pos: 0,
        line: 0,
        frames_to_line: 0.0,
        tpl: song_tpl,
        fx_vol: [None; MAX_TRACKS],
        fx_pan: [None; MAX_TRACKS],
        fx_width: [None; MAX_TRACKS],
        chance: 0x2545_F491_4F6C_DD1D,
        voices: Vec::with_capacity(MAX_VOICES),
        events: Vec::with_capacity(MAX_EVENTS),
        phrase_runs: Vec::with_capacity(MAX_PHRASE_RUNS),
        midi_tx: mtx,
        midi_on: false,
        midi_clock: false,
        midi_running: false,
        midi_clock_due: 0.0,
        midi_notes: Vec::with_capacity(MAX_MIDI_NOTES),
        midi_program: [0xFF; 16],
        now_frames: 0,
        pending_break: None,
        stop_at_end: false,
        bend: 0.0,
        inst_plugins: (0..MAX_INSTRUMENT_PLUGINS).map(|_| None).collect(),
        plugin_track: vec![0; MAX_INSTRUMENT_PLUGINS],
        plugin_notes: Vec::with_capacity(MAX_PLUGIN_NOTES),
        plugin_buf: vec![0.0; BLOCK * 2],
        tick: 0,
        frames_to_tick: 0.0,
        chains: (0..MAX_TRACKS).map(|_| Vec::new()).collect(),
        pdc: vec![0.0; MAX_TRACKS * PDC_FRAMES * 2],
        pdc_pos: [0; MAX_TRACKS],
        pdc_max: 0,
        track_bufs: vec![0.0; MAX_TRACKS * BLOCK * 2],
        bus: vec![0.0; BLOCK * 2],
        acc: vec![0.0; MAX_TRACKS * BLOCK * 2],
        order: Vec::with_capacity(MAX_TRACKS),
        track_used: [false; MAX_TRACKS],
        fader_prev: [-1.0; MAX_TRACKS],
        overrides: Vec::with_capacity(MAX_OVERRIDES),
        mod_out: vec![0.0; MAX_TRACKS * MAX_MOD_DEVS],
        sc_buf: vec![0.0; BLOCK * 2],
        track_note: [(48, 1.0); MAX_TRACKS],
        playing_seq: 0,
        playing_line: 0,
        meter_acc: [0.0; MAX_TRACKS],
        master_acc: [0.0; 2],
    };
    let midi_out = Some(MidiOutSource { rx: mrx, pos: pos.clone() });
    (engine, EngineHandle { tx, garbage: grx, pos, sample_rate, midi_out })
}

pub struct Engine {
    sample_rate: u32,
    song: Arc<Song>,
    bpm: f32,
    lpb: u8,
    rx: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Garbage>,
    pos: Arc<SharedPos>,
    playing: bool,
    loop_pattern: bool,
    /// Lines looped by `PlayBlock`.
    block: Option<(u16, u16)>,
    seq_pos: usize,
    line: u16,
    frames_to_line: f64,
    /// Ticks per line, changed by `ZK` while playing.
    tpl: u8,
    /// Track volume / pan set by `0L` / `0P` until the next play or stop.
    fx_vol: [Option<f32>; MAX_TRACKS],
    fx_pan: [Option<f32>; MAX_TRACKS],
    /// Track stereo width set by `0H` (0 = mono, 1 = unchanged, 2 = wide).
    fx_width: [Option<f32>; MAX_TRACKS],
    chance: u64,
    voices: Vec<Voice>,
    events: Vec<Event>,
    phrase_runs: Vec<PhraseRun>,
    midi_tx: rtrb::Producer<(u64, MidiOutMsg)>,
    midi_on: bool,
    midi_clock: bool,
    /// Whether start was sent (transport state as the MIDI output last saw it).
    midi_running: bool,
    midi_clock_due: f64,
    midi_notes: Vec<MidiNote>,
    /// Last program change sent per channel (0xFF = none).
    midi_program: [u8; 16],
    now_frames: u64,
    /// Line to jump to in the next pattern slot (ZB command).
    pending_break: Option<u16>,
    stop_at_end: bool,
    bend: f32,
    /// Instrument plugins by instrument index, the track each last played on, and their held notes.
    inst_plugins: Vec<Option<Box<dyn Device>>>,
    plugin_track: Vec<usize>,
    plugin_notes: Vec<PluginNote>,
    plugin_buf: Vec<f32>,
    /// Tick index within the current line and frames until the next tick.
    tick: u8,
    frames_to_tick: f64,
    chains: Vec<Vec<Box<dyn Device>>>,
    /// Delay-compensation rings, one per track (`PDC_FRAMES` stereo frames each), write positions and the
    /// largest chain latency this block.
    pdc: Vec<f32>,
    pdc_pos: [usize; MAX_TRACKS],
    pdc_max: u32,
    /// One stereo bus per track, `BLOCK` frames each, so devices can process a track's whole signal.
    track_bufs: Vec<f32>,
    bus: Vec<f32>,
    /// What other tracks routed into each group or send track this block.
    acc: Vec<f32>,
    order: Vec<usize>,
    track_used: [bool; MAX_TRACKS],
    /// Last fader gain per track (negative = never used), ramped to the new gain across each block.
    fader_prev: [f32; MAX_TRACKS],
    overrides: Vec<Override>,
    /// Last control value of each chain slot's modulator (applied one block later).
    mod_out: Vec<f32>,
    /// Copy of the key track's signal for a sidechain device.
    sc_buf: Vec<f32>,
    /// Last note started on each track and its velocity, for the tracker modulators.
    track_note: [(u8, f32); MAX_TRACKS],
    playing_seq: usize,
    playing_line: u16,
    meter_acc: [f32; MAX_TRACKS],
    master_acc: [f32; 2],
}

impl Engine {
    fn frames_per_tick(&self) -> f64 {
        Song::frames_per_line(self.bpm, self.lpb, self.sample_rate) / self.tpl.max(1) as f64
    }

    /// Render interleaved stereo f32 into `out` (`out.len()` must be even).
    pub fn render(&mut self, out: &mut [f32]) {
        let started = std::time::Instant::now();
        self.drain_commands();
        self.meter_acc = [0.0; MAX_TRACKS];
        self.master_acc = [0.0; 2];
        let total = out.len() / 2;
        let mut off = 0;
        while off < total {
            let n = (total - off).min(BLOCK);
            self.render_block(&mut out[off * 2..(off + n) * 2]);
            off += n;
        }
        for (slot, p) in self.pos.meters.iter().zip(self.meter_acc) {
            slot.store(p.to_bits(), Ordering::Relaxed);
        }
        self.pos.master[0].store(self.master_acc[0].to_bits(), Ordering::Relaxed);
        self.pos.master[1].store(self.master_acc[1].to_bits(), Ordering::Relaxed);
        self.pos.frames.store(self.now_frames, Ordering::Release);
        self.pos.voice_count.store(self.voices.len() as u32, Ordering::Relaxed);
        // Load = time spent / time the buffer represents, smoothed so the meter is readable.
        let budget = total as f64 / self.sample_rate.max(1) as f64;
        if budget > 0.0 {
            let load = (started.elapsed().as_secs_f64() / budget) as f32;
            let prev = f32::from_bits(self.pos.cpu.load(Ordering::Relaxed));
            let smoothed = if load > prev { load } else { prev * 0.95 + load * 0.05 };
            self.pos.cpu.store(smoothed.to_bits(), Ordering::Relaxed);
        }
    }

    /// Sequence, trigger and mix voices into per-track buses for up to `BLOCK` frames, then mix down.
    fn render_block(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        self.track_used = [false; MAX_TRACKS];
        self.update_automation();
        let mut done = 0;
        while done < frames {
            self.midi_transport();
            if self.playing && self.frames_to_line <= 0.0 {
                self.apply_tempo_lanes(self.seq_pos, self.line as f32);
                self.trigger_line();
                self.playing_seq = self.seq_pos;
                self.playing_line = self.line;
                let straight = Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
                self.frames_to_line += straight * self.song.groove_scale(self.line);
                self.tick = 0;
                self.frames_to_tick = self.frames_per_tick();
                for v in &mut self.voices {
                    v.tick(0);
                }
                self.advance_line();
            }
            self.fire_due_events();
            self.step_phrases();
            let mut chunk = frames - done;
            if self.playing {
                chunk = chunk
                    .min((self.frames_to_line.ceil() as usize).max(1))
                    .min((self.frames_to_tick.ceil() as usize).max(1));
            }
            for e in &self.events {
                chunk = chunk.min((e.at.ceil() as usize).max(1));
            }
            for r in &self.phrase_runs {
                chunk = chunk.min((r.frames_to_line.ceil() as usize).max(1));
            }
            if self.midi_on && self.midi_clock && self.playing {
                chunk = chunk.min((self.midi_clock_due.ceil() as usize).max(1));
            }
            let line_frames = Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
            for v in &mut self.voices {
                v.set_line_frames(line_frames);
                v.bend = self.bend;
                let t = v.track.min(MAX_TRACKS - 1);
                let base = t * BLOCK * 2;
                if !self.track_used[t] {
                    self.track_bufs[base..base + frames * 2].fill(0.0);
                    self.track_used[t] = true;
                }
                v.mix(
                    &mut self.track_bufs[base + done * 2..base + (done + chunk) * 2],
                    self.sample_rate,
                    1.0,
                    0.0,
                );
            }
            self.render_plugins(done, chunk, frames);
            self.retire_dead_voices();
            if self.playing {
                self.frames_to_line -= chunk as f64;
                self.frames_to_tick -= chunk as f64;
                while self.frames_to_tick <= 0.0 && self.frames_to_line > 0.0 {
                    self.tick = self.tick.saturating_add(1);
                    for v in &mut self.voices {
                        v.tick(self.tick);
                    }
                    self.frames_to_tick += self.frames_per_tick();
                }
            }
            for e in &mut self.events {
                e.at -= chunk as f64;
            }
            for r in &mut self.phrase_runs {
                r.frames_to_line -= chunk as f64;
            }
            self.midi_clock_due -= chunk as f64;
            self.now_frames += chunk as u64;
            done += chunk;
        }
        self.mix_down(out, frames);
    }

    /// Set tempo and lines per beat from the pattern's tempo lanes at line position `pos`.
    fn apply_tempo_lanes(&mut self, seq_pos: usize, pos: f32) {
        let song = self.song.clone();
        let Some(pat) = song.pattern_at(seq_pos) else { return };
        for a in &pat.automation {
            let Some(v) = a.value_at(pos) else { continue };
            match a.target {
                AutoTarget::Bpm => self.bpm = AutoTarget::bpm_for(v),
                AutoTarget::Lpb => self.lpb = AutoTarget::lpb_for(v),
                _ => {}
            }
        }
    }

    /// Fill `overrides` with the automated values for the current playback position.
    fn update_automation(&mut self) {
        self.overrides.clear();
        if !self.playing {
            return;
        }
        let song = self.song.clone();
        let Some(pat) = song.pattern_at(self.playing_seq) else { return };
        let fpl = Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
        // Before the first line triggers (frames_to_line <= 0) we are at the line about to play.
        let pos = if self.frames_to_line <= 0.0 {
            self.line as f32
        } else {
            self.playing_line as f32 + (1.0 - self.frames_to_line / fpl).clamp(0.0, 1.0) as f32
        };
        for a in &pat.automation {
            let Some(v) = a.value_at(pos) else { continue };
            match a.target {
                AutoTarget::Bpm => {
                    self.bpm = AutoTarget::bpm_for(v);
                    continue;
                }
                AutoTarget::Lpb => {
                    self.lpb = AutoTarget::lpb_for(v);
                    continue;
                }
                _ => {}
            }
            if self.overrides.len() >= MAX_OVERRIDES || a.track >= MAX_TRACKS {
                continue;
            }
            let (target, value) = match a.target {
                AutoTarget::TrackVolume => (Ov::Volume, v),
                AutoTarget::TrackPan => (Ov::Pan, v),
                AutoTarget::Bpm | AutoTarget::Lpb => continue,
                AutoTarget::Device { device, param } => {
                    let spec = song.tracks.get(a.track).and_then(|t| t.devices.get(device));
                    let value = match spec {
                        Some(d) if d.plugin.is_some() => {
                            d.plugin.as_ref().and_then(|p| p.params.get(param)).map(|i| i.denormalize(v))
                        }
                        Some(d) => trakr_core::device_info(&d.id)
                            .and_then(|i| i.params.get(param))
                            .map(|i| i.denormalize(v)),
                        None => None,
                    };
                    match value {
                        Some(value) => (Ov::Param { device, param }, value),
                        None => continue,
                    }
                }
            };
            self.overrides.push(Override { track: a.track, target, value });
        }
    }

    fn override_for(&self, track: usize, f: impl Fn(&Ov) -> bool) -> Option<f32> {
        self.overrides.iter().find(|o| o.track == track && f(&o.target)).map(|o| o.value)
    }

    /// Fader gains for track `t` this block: (previous, target, left, right), with volume/pan automation,
    /// mute and solo applied. Sequencer tracks are silenced by other tracks' solo; groups and sends are not.
    fn fader(&self, t: usize, tr: &trakr_core::Track, any_solo: bool) -> (f32, f32, f32, f32) {
        let vol = match self.override_for(t, |o| matches!(o, Ov::Volume)) {
            Some(n) if n <= 0.0 => 0.0,
            Some(n) => dsp::db_to_gain(-60.0 + 66.0 * n),
            None => tr.volume,
        };
        let vol = self.fx_vol.get(t).copied().flatten().unwrap_or(vol);
        let pan = self.override_for(t, |o| matches!(o, Ov::Pan)).map_or(tr.panning, |n| n * 2.0 - 1.0);
        let pan = self.fx_pan.get(t).copied().flatten().unwrap_or(pan);
        let solo_gate = tr.kind != trakr_core::TrackKind::Sequencer || !any_solo || tr.solo;
        let target = if tr.mute == MuteState::Active && solo_gate { vol } else { 0.0 };
        let prev = if self.fader_prev[t] < 0.0 { target } else { self.fader_prev[t] }; // first use: no ramp
        (prev, target, (1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
    }

    /// Run device chains, route through faders, groups and sends, and finish on the master track.
    ///
    /// Order: sequencer tracks (feeding their group or the master, plus sends) -> groups, deepest first
    /// -> send tracks -> master chain and volume.
    fn mix_down(&mut self, out: &mut [f32], frames: usize) {
        let song = self.song.clone(); // refcount bump only
        let n = song.tracks.len().min(MAX_TRACKS);
        let any_solo = song.tracks.iter().any(|t| t.solo);
        self.bus[..frames * 2].fill(0.0);
        for t in 0..n {
            if matches!(song.tracks[t].kind, trakr_core::TrackKind::Group | trakr_core::TrackKind::Send) {
                let base = t * BLOCK * 2;
                self.acc[base..base + frames * 2].fill(0.0);
            }
        }
        // Where a track's output goes: its group if it has a valid one, otherwise the master bus.
        let dest = |t: usize| -> Option<usize> {
            song.tracks[t]
                .group
                .filter(|&g| g < n && g != t && song.tracks[g].kind == trakr_core::TrackKind::Group)
        };
        let depth = |mut t: usize| {
            let mut d = 0;
            while let Some(g) = dest(t) {
                t = g;
                d += 1;
                if d >= 8 {
                    break;
                }
            }
            d
        };

        // Phase 1: sequencer tracks, then groups (deepest first), then sends. All are "strips".
        // Reuse a preallocated list: nothing on the audio thread may allocate.
        let mut lat = [0u32; MAX_TRACKS];
        let mut pdc_max = 0u32;
        #[allow(clippy::needless_range_loop)]
        for t in 0..n {
            if song.tracks[t].kind != trakr_core::TrackKind::Sequencer {
                continue;
            }
            let total: u32 = self.chains[t]
                .iter()
                .zip(song.tracks[t].devices.iter())
                .filter(|(_, spec)| !spec.bypass)
                .map(|(d, _)| d.latency())
                .sum();
            lat[t] = total.min(PDC_FRAMES as u32 - 1);
            pdc_max = pdc_max.max(lat[t]);
        }
        self.pdc_max = pdc_max;
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        order.extend((0..n).filter(|&t| song.tracks[t].kind == trakr_core::TrackKind::Sequencer));
        for d in (0..=8).rev() {
            order.extend(
                (0..n).filter(|&t| song.tracks[t].kind == trakr_core::TrackKind::Group && depth(t) == d),
            );
        }
        order.extend((0..n).filter(|&t| song.tracks[t].kind == trakr_core::TrackKind::Send));

        for &t in order.iter() {
            let tr = &song.tracks[t];
            let base = t * BLOCK * 2;
            let is_bus = matches!(tr.kind, trakr_core::TrackKind::Group | trakr_core::TrackKind::Send);
            if !is_bus && !self.track_used[t] && self.chains[t].is_empty() && pdc_max == 0 {
                continue;
            }
            if !self.track_used[t] {
                self.track_bufs[base..base + frames * 2].fill(0.0); // silence in, tails out
            }
            if is_bus {
                // Add everything routed here to whatever this strip's own voices produced.
                let (tb, acc) = (&mut self.track_bufs, &self.acc);
                for (d, a) in tb[base..base + frames * 2].iter_mut().zip(&acc[base..base + frames * 2]) {
                    *d += a;
                }
            }
            self.run_chain(t, tr, base, frames);
            if pdc_max > 0 && tr.kind == trakr_core::TrackKind::Sequencer {
                self.compensate(t, base, frames, (pdc_max - lat[t]) as usize);
            }

            let (prev, target, pl, pr) = self.fader(t, tr, any_solo);
            let audible = target > 0.0 || prev > 0.0;
            let width = self.fx_width.get(t).copied().flatten().unwrap_or(1.0).clamp(0.0, 2.0);

            // Sends tap the signal after the chain, before or after the fader.
            for s in &tr.sends {
                if s.target >= n || s.target == t || song.tracks[s.target].kind != trakr_core::TrackKind::Send
                {
                    continue;
                }
                let sbase = s.target * BLOCK * 2;
                let src = &self.track_bufs[base..base + frames * 2];
                let dst = &mut self.acc[sbase..sbase + frames * 2];
                let gate = if tr.mute == MuteState::Active && (!any_solo || tr.solo) { 1.0 } else { 0.0 };
                for (i, (d, f)) in dst.chunks_exact_mut(2).zip(src.chunks_exact(2)).enumerate() {
                    let g = if s.pre_fader {
                        1.0
                    } else {
                        prev + (target - prev) * (i as f32 + 1.0) / frames as f32
                    };
                    let g = g * s.level * gate;
                    d[0] += f[0] * g * if s.pre_fader { 1.0 } else { pl };
                    d[1] += f[1] * g * if s.pre_fader { 1.0 } else { pr };
                }
            }

            let src = &self.track_bufs[base..base + frames * 2];
            let mut peak = 0.0f32;
            let dst: &mut [f32] = match dest(t) {
                Some(g) => &mut self.acc[g * BLOCK * 2..g * BLOCK * 2 + frames * 2],
                None => &mut self.bus[..frames * 2],
            };
            if audible || prev != target {
                for (i, (d, f)) in dst.chunks_exact_mut(2).zip(src.chunks_exact(2)).enumerate() {
                    let g = prev + (target - prev) * (i as f32 + 1.0) / frames as f32;
                    let (fl, fr) = if width == 1.0 {
                        (f[0], f[1])
                    } else {
                        let (m, s) = ((f[0] + f[1]) * 0.5, (f[0] - f[1]) * 0.5 * width);
                        (m + s, m - s)
                    };
                    let (l, r) = (fl * g * pl, fr * g * pr);
                    d[0] += l;
                    d[1] += r;
                    peak = peak.max(l.abs()).max(r.abs());
                }
            }
            self.fader_prev[t] = target;
            self.meter_acc[t] = self.meter_acc[t].max(peak);
        }

        self.order = order;

        // Master.
        let master = (0..n).find(|&t| song.tracks[t].kind == trakr_core::TrackKind::Master);
        if let Some(m) = master {
            let tr = &song.tracks[m];
            let base = m * BLOCK * 2;
            self.track_bufs[base..base + frames * 2].copy_from_slice(&self.bus[..frames * 2]);
            self.run_chain(m, tr, base, frames);
            self.bus[..frames * 2].copy_from_slice(&self.track_bufs[base..base + frames * 2]);
            let vol = match self.override_for(m, |o| matches!(o, Ov::Volume)) {
                Some(n) if n <= 0.0 => 0.0,
                Some(n) => dsp::db_to_gain(-60.0 + 66.0 * n),
                None => tr.volume,
            };
            let g = if tr.mute == MuteState::Active { vol } else { 0.0 };
            for (o, b) in out.chunks_exact_mut(2).zip(self.bus.chunks_exact(2)) {
                o[0] = b[0] * g;
                o[1] = b[1] * g;
                self.master_acc[0] = self.master_acc[0].max(o[0].abs());
                self.master_acc[1] = self.master_acc[1].max(o[1].abs());
            }
        } else {
            out.copy_from_slice(&self.bus[..frames * 2]);
            for o in out.chunks_exact(2) {
                self.master_acc[0] = self.master_acc[0].max(o[0].abs());
                self.master_acc[1] = self.master_acc[1].max(o[1].abs());
            }
        }
    }

    fn run_chain(&mut self, t: usize, tr: &trakr_core::Track, base: usize, frames: usize) {
        if t >= MAX_TRACKS {
            return;
        }
        let mut params = [0.0f32; trakr_core::MAX_PLUGIN_PARAMS];
        // Modulators publish a control value per block; apply last block's values to their targets.
        let mut mods = [(0usize, 0usize, 0.0f32); MAX_MODS];
        let mut nmods = 0;
        for m in 0..self.chains[t].len().min(MAX_MOD_DEVS) {
            let Some(spec) = tr.devices.get(m) else { break };
            if spec.bypass
                || !trakr_core::MODULATORS.contains(&spec.id.as_str())
                || self.chains[t][m].id() != spec.id
            {
                continue;
            }
            let amount = self.mod_out[t * MAX_MOD_DEVS + m];
            let amount = if amount.is_finite() { amount } else { 0.0 };
            if spec.id == "native.metamixer" {
                // four knobs, each a fixed offset on its own target
                for k in 0..4 {
                    let target = spec.param(3 * k).clamp(0.0, 8.0).round() as usize;
                    if target == 0 || nmods == MAX_MODS {
                        continue;
                    }
                    let param = spec.param(3 * k + 1).clamp(0.0, 15.0).round() as usize;
                    mods[nmods] = (target - 1, param, spec.param(3 * k + 2).clamp(-1.0, 1.0));
                    nmods += 1;
                }
                continue;
            }
            if spec.id == "native.hydra" {
                // three targets sharing one oscillator, each with its own depth
                for k in 0..3 {
                    let target = spec.param(2 + 3 * k).clamp(0.0, 8.0).round() as usize;
                    if target == 0 || nmods == MAX_MODS {
                        continue;
                    }
                    let param = spec.param(3 + 3 * k).clamp(0.0, 15.0).round() as usize;
                    let depth = spec.param(4 + 3 * k).clamp(-1.0, 1.0);
                    mods[nmods] = (target - 1, param, amount * depth);
                    nmods += 1;
                }
                continue;
            }
            let target = spec.param(0).clamp(0.0, 8.0).round() as usize;
            if target == 0 || nmods == MAX_MODS {
                continue;
            }
            let param = spec.param(1).clamp(0.0, 15.0).round() as usize;
            mods[nmods] = (target - 1, param, amount);
            nmods += 1;
        }
        for d in 0..self.chains[t].len() {
            let Some(spec) = tr.devices.get(d) else { break };
            if d < MAX_MOD_DEVS {
                self.mod_out[t * MAX_MOD_DEVS + d] = 0.0;
            }
            if spec.bypass {
                continue;
            }
            // The song snapshot and the device instances are updated by separate commands. Never feed a
            // device another device's parameters (that can blow up its feedback paths for good).
            if spec.id.starts_with(trakr_core::PLUGIN_ID_PREFIX) {
                self.run_plugin(t, d, spec, base, frames, &mut params);
                continue;
            }
            if self.chains[t][d].id() != spec.id {
                continue;
            }
            let Some(info) = trakr_core::device_info(&spec.id) else { continue };
            let n = info.params.len().min(params.len());
            for (i, slot) in params.iter_mut().enumerate().take(n) {
                // Files and automation may carry out-of-range values; keep every device inside its design range.
                *slot = spec.param(i).clamp(info.params[i].min, info.params[i].max);
            }
            for o in &self.overrides {
                if let (true, Ov::Param { device, param }) = (o.track == t, &o.target) {
                    if *device == d && *param < n {
                        params[*param] = o.value.clamp(info.params[*param].min, info.params[*param].max);
                    }
                }
            }
            for &(td, tp, amount) in &mods[..nmods] {
                if td == d && tp < n {
                    let i = &info.params[tp];
                    params[tp] = i.from_norm(i.to_norm(params[tp]) + amount);
                }
            }
            if spec.id == "native.send" {
                self.run_send(t, spec, base, frames, &params);
            }
            let buf = &mut self.track_bufs[base..base + frames * 2];
            self.chains[t][d].set_tempo(self.bpm);
            let (note, vel) = self.track_note[t];
            self.chains[t][d].set_track_note(note, vel);
            if spec.id == "native.sidechain" {
                // another track's raw mix as the key signal (silence if it made no sound this block)
                let src = params[0].round().clamp(0.0, MAX_TRACKS as f32) as usize;
                let use_key = src >= 1 && src - 1 != t;
                if use_key {
                    if self.track_used[src - 1] {
                        let sb = (src - 1) * BLOCK * 2;
                        self.sc_buf[..frames * 2].copy_from_slice(&self.track_bufs[sb..sb + frames * 2]);
                    } else {
                        self.sc_buf[..frames * 2].fill(0.0);
                    }
                }
                let sc = use_key.then(|| &self.sc_buf[..frames * 2]);
                let buf = &mut self.track_bufs[base..base + frames * 2];
                self.chains[t][d].process_sc(&params[..n], buf, sc);
            } else {
                self.chains[t][d].process(&params[..n], buf);
            }
            if d < MAX_MOD_DEVS {
                self.mod_out[t * MAX_MOD_DEVS + d] = self.chains[t][d].modulation().unwrap_or(0.0);
            }
        }
    }

    /// Run a hosted plugin in chain slot `d`: its parameters come straight from the song (clamped to the
    /// ranges the plugin reported), with pattern automation applied on top.
    fn run_plugin(
        &mut self,
        t: usize,
        d: usize,
        spec: &trakr_core::DeviceSpec,
        base: usize,
        frames: usize,
        params: &mut [f32; trakr_core::MAX_PLUGIN_PARAMS],
    ) {
        let Some(plugin) = &spec.plugin else { return };
        if self.chains[t][d].id() != "clap" {
            return;
        }
        let n = plugin.params.len().min(params.len());
        for (i, slot) in params.iter_mut().enumerate().take(n) {
            let p = &plugin.params[i];
            *slot = p.clamp(spec.params.get(i).copied().unwrap_or(p.default));
        }
        for o in &self.overrides {
            if let (true, Ov::Param { device, param }) = (o.track == t, &o.target) {
                if *device == d && *param < n {
                    params[*param] = plugin.params[*param].clamp(o.value);
                }
            }
        }
        let buf = &mut self.track_bufs[base..base + frames * 2];
        self.chains[t][d].process(&params[..n], buf);
    }

    /// `#Send`: copy this strip's signal at this point of its chain into a send track.
    fn run_send(
        &mut self,
        t: usize,
        spec: &trakr_core::DeviceSpec,
        base: usize,
        frames: usize,
        params: &[f32],
    ) {
        let target = params[0].round().clamp(1.0, MAX_TRACKS as f32) as usize - 1;
        let valid = self.song.tracks.get(target).is_some_and(|x| x.kind == trakr_core::TrackKind::Send)
            && target != t
            // sends are mixed in track order, so a send track can only feed a later one
            && (self.song.tracks[t].kind != trakr_core::TrackKind::Send || target > t);
        if !valid || spec.bypass {
            return;
        }
        let level = if params[1] <= -59.9 { 0.0 } else { dsp::db_to_gain(params[1]) };
        let sbase = target * BLOCK * 2;
        let (src, dst) =
            (&self.track_bufs[base..base + frames * 2], &mut self.acc[sbase..sbase + frames * 2]);
        for (d, s) in dst.iter_mut().zip(src) {
            *d += s * level;
        }
        if params[2] >= 0.5 {
            self.track_bufs[base..base + frames * 2].fill(0.0);
        }
    }

    fn fire_due_events(&mut self) {
        let mut i = 0;
        while i < self.events.len() {
            if self.events[i].at <= 0.0 {
                let e = self.events.remove(i);
                match e.kind {
                    EventKind::NoteOn { inst, note, vol, pan, opts, fx } => {
                        self.note_on(e.track, e.column, inst, note, vol, pan, opts);
                        // Effects written next to a delayed note start with it, not at the line start.
                        if let Some(v) =
                            self.voices.last_mut().filter(|v| v.track == e.track && v.column == e.column)
                        {
                            for f in fx.iter().filter(|f| !f.is_empty()) {
                                v.apply_effect(f.id, f.amount);
                            }
                        }
                    }
                    EventKind::Phrase { inst, phrase, note, vol, pan } => {
                        self.start_phrase(e.track, e.column, inst, phrase, note, vol, pan)
                    }
                    EventKind::Release => {
                        self.release_plugin(|n| n.track == e.track && n.column == e.column);
                        self.release_midi(|n| n.track == e.track && n.column == e.column);
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
                    if old.tpl != self.song.tpl {
                        self.tpl = self.song.tpl;
                    }
                    // If the garbage queue is full, leak rather than free on the RT thread.
                    if let Err(rtrb::PushError::Full(old)) = self.garbage.push(Garbage::Song(old)) {
                        std::mem::forget(old);
                    }
                    self.clamp_position();
                }
                Command::SetChain { track, chain } => {
                    if let Some(slot) = self.chains.get_mut(track) {
                        let old = std::mem::replace(slot, chain);
                        if let Err(rtrb::PushError::Full(old)) = self.garbage.push(Garbage::Chain(old)) {
                            std::mem::forget(old);
                        }
                    } else {
                        std::mem::forget(chain);
                    }
                }
                Command::SetInstrumentPlugin { instrument, device } => {
                    if let Some(slot) = self.inst_plugins.get_mut(instrument) {
                        self.plugin_notes.retain(|n| n.inst as usize != instrument);
                        let old = std::mem::replace(slot, device);
                        if let Some(old) = old {
                            if let Err(rtrb::PushError::Full(old)) = self.garbage.push(Garbage::Device(old)) {
                                std::mem::forget(old);
                            }
                        }
                    } else {
                        std::mem::forget(device);
                    }
                }
                Command::Play { seq_pos, line, loop_pattern } => {
                    self.bpm = self.song.bpm;
                    self.lpb = self.song.lpb;
                    self.tpl = self.song.tpl;
                    self.fx_vol = [None; MAX_TRACKS];
                    self.fx_pan = [None; MAX_TRACKS];
                    self.fx_width = [None; MAX_TRACKS];
                    self.seq_pos = seq_pos;
                    self.line = line;
                    self.clamp_position();
                    self.loop_pattern = loop_pattern;
                    self.block = None;
                    self.frames_to_line = 0.0;
                    self.events.clear();
                    self.clear_phrases();
                    self.pending_break = None;
                    self.playing = true;
                }
                Command::PlayBlock { seq_pos, first, last } => {
                    self.bpm = self.song.bpm;
                    self.lpb = self.song.lpb;
                    self.tpl = self.song.tpl;
                    self.fx_vol = [None; MAX_TRACKS];
                    self.fx_pan = [None; MAX_TRACKS];
                    self.fx_width = [None; MAX_TRACKS];
                    self.seq_pos = seq_pos;
                    self.clamp_position();
                    let lines = self.song.pattern_at(self.seq_pos).map_or(1, |p| p.number_of_lines);
                    let (a, b) = (first.min(last).min(lines - 1), first.max(last).min(lines - 1));
                    self.line = a;
                    self.block = Some((a, b));
                    self.loop_pattern = true;
                    self.frames_to_line = 0.0;
                    self.events.clear();
                    self.clear_phrases();
                    self.pending_break = None;
                    self.playing = true;
                }
                Command::Stop => {
                    self.block = None;
                    self.playing = false;
                    self.release_plugin(|_| true);
                    self.retire_all_voices();
                    self.events.clear();
                    self.clear_phrases();
                    self.pending_break = None;
                }
                Command::SetStopAtEnd(v) => self.stop_at_end = v,
                Command::PitchBend(semis) => {
                    self.bend = if semis.is_finite() { semis.clamp(-24.0, 24.0) } else { 0.0 };
                }
                Command::NoteOn { track, instrument, note, volume } => {
                    let phrase =
                        self.song.instruments.get(instrument as usize).and_then(|i| i.phrase_for(note));
                    if let Some(p) = phrase {
                        self.start_phrase(track, LIVE_COL, instrument, p, note, volume, 0.0);
                    } else {
                        self.note_on(
                            track,
                            usize::MAX,
                            instrument,
                            note,
                            volume,
                            0.0,
                            TriggerOpts::default(),
                        );
                    }
                }
                Command::NoteOff { track, note: _ } => {
                    self.release_plugin(|n| n.track == track);
                    for v in self.voices.iter_mut().filter(|v| v.track == track) {
                        v.release();
                    }
                    self.release_midi(|n| n.track == track);
                    self.stop_phrase(track, LIVE_COL);
                }
                Command::SetMidiOut { enabled, clock } => {
                    if self.midi_on && !enabled {
                        self.release_midi(|_| true);
                    }
                    self.midi_on = enabled;
                    self.midi_clock = clock;
                    self.midi_running = self.playing && enabled && self.midi_running;
                    if enabled && self.playing && !self.midi_running {
                        self.midi_running = true;
                        if clock {
                            self.midi_emit(MidiOutMsg::Start);
                        }
                    }
                    self.midi_clock_due = 0.0;
                }
                Command::PlayPhrase { track, instrument, phrase, note } => {
                    self.start_phrase(track, LIVE_COL, instrument, phrase, note, 1.0, 0.0);
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
        let song = self.song.clone(); // refcount bump only
        let Some(instrument) = song.instruments.get(inst as usize) else { return };
        if let Some(t) = self.track_note.get_mut(track) {
            *t = (note.min(119), vol.clamp(0.0, 1.0));
        }
        if let Some(m) = instrument.midi_out {
            if song.tracks.get(track).is_none_or(|t| t.mute == MuteState::Active) {
                self.midi_note_on(track, column, m, note, vol);
            }
        }
        if instrument.plugin.is_some() {
            if song.tracks.get(track).is_some_and(|t| t.mute == MuteState::Off) {
                return;
            }
            self.plugin_note_on(track, column, inst, note, vol);
            return;
        }
        let velocity = (vol.clamp(0.0, 1.0) * 127.0).round() as u8;
        let Some(sample) = instrument.sample_for_velocity(note, velocity) else { return };
        if song.tracks.get(track).is_some_and(|t| t.mute == MuteState::Off) {
            return;
        }
        // The previous note in this column ends according to its instrument's NNA setting.
        for v in self.voices.iter_mut().filter(|v| v.track == track && v.column == column) {
            match v.nna() {
                trakr_core::Nna::Cut => v.cut(),
                trakr_core::Nna::NoteOff => v.release(),
                trakr_core::Nna::Continue => {}
            }
            v.detach();
        }
        if self.voices.len() >= MAX_VOICES {
            let old = self.voices.remove(0);
            self.retire_voice(old);
        }
        self.voices.push(Voice::new(track, column, instrument, sample, note, vol, pan, opts));
    }

    /// Delay track `t`'s block by `delay` frames through its compensation ring.
    fn compensate(&mut self, t: usize, base: usize, frames: usize, delay: usize) {
        let ring = &mut self.pdc[t * PDC_FRAMES * 2..(t + 1) * PDC_FRAMES * 2];
        let mut pos = self.pdc_pos[t];
        for f in self.track_bufs[base..base + frames * 2].chunks_exact_mut(2) {
            ring[pos * 2] = f[0];
            ring[pos * 2 + 1] = f[1];
            let read = (pos + PDC_FRAMES - delay) % PDC_FRAMES;
            f[0] = ring[read * 2];
            f[1] = ring[read * 2 + 1];
            pos = (pos + 1) % PDC_FRAMES;
        }
        self.pdc_pos[t] = pos;
    }

    /// Hand a finished voice to the UI thread; if the queue is full it drops here (rare).
    fn retire_voice(&mut self, v: Voice) {
        let _ = self.garbage.push(Garbage::Voice(RetiredVoice(v)));
    }

    fn retire_dead_voices(&mut self) {
        let mut i = 0;
        while i < self.voices.len() {
            if self.voices[i].alive() {
                i += 1;
            } else {
                let v = self.voices.remove(i);
                self.retire_voice(v);
            }
        }
    }

    fn retire_all_voices(&mut self) {
        while let Some(v) = self.voices.pop() {
            self.retire_voice(v);
        }
    }

    /// Run every installed instrument plugin for this chunk and add its output to the track it plays on.
    fn render_plugins(&mut self, done: usize, chunk: usize, frames: usize) {
        let song = self.song.clone(); // refcount bump only
        for i in 0..self.inst_plugins.len() {
            let Some(dev) = self.inst_plugins[i].as_mut() else { continue };
            let Some(spec) = song.instruments.get(i).and_then(|inst| inst.plugin.as_ref()) else { continue };
            let t = self.plugin_track[i].min(MAX_TRACKS - 1);
            let buf = &mut self.plugin_buf[..chunk * 2];
            buf.fill(0.0);
            dev.process(&spec.params, buf);
            let base = t * BLOCK * 2;
            if !self.track_used[t] {
                self.track_bufs[base..base + frames * 2].fill(0.0);
                self.track_used[t] = true;
            }
            let out = &mut self.track_bufs[base + done * 2..base + (done + chunk) * 2];
            for (o, s) in out.iter_mut().zip(buf.iter()) {
                *o += if s.is_finite() { *s } else { 0.0 };
            }
        }
    }

    /// Start a note on the instrument's plugin; the previous note in the same column ends first.
    fn plugin_note_on(&mut self, track: usize, column: usize, inst: u8, note: u8, vol: f32) {
        let i = inst as usize;
        if self.inst_plugins.get(i).and_then(|d| d.as_ref()).is_none() {
            return;
        }
        self.release_plugin(|n| n.inst == inst && n.track == track && n.column == column);
        if self.plugin_notes.len() >= MAX_PLUGIN_NOTES {
            let old = self.plugin_notes.remove(0);
            if let Some(Some(d)) = self.inst_plugins.get_mut(old.inst as usize) {
                d.note_off(old.key);
            }
        }
        if let Some(Some(d)) = self.inst_plugins.get_mut(i) {
            d.note_on(note, vol.clamp(0.0, 1.0));
        }
        self.plugin_track[i] = track.min(MAX_TRACKS - 1);
        self.plugin_notes.push(PluginNote { inst, track, column, key: note });
    }

    /// End the plugin notes `which` selects.
    fn release_plugin(&mut self, which: impl Fn(&PluginNote) -> bool) {
        let mut i = 0;
        while i < self.plugin_notes.len() {
            if which(&self.plugin_notes[i]) {
                let n = self.plugin_notes.swap_remove(i);
                if let Some(Some(d)) = self.inst_plugins.get_mut(n.inst as usize) {
                    d.note_off(n.key);
                }
            } else {
                i += 1;
            }
        }
    }

    fn midi_emit(&mut self, msg: MidiOutMsg) {
        if self.midi_on {
            let _ = self.midi_tx.push((self.now_frames, msg));
        }
    }

    /// Follow the transport: start/stop messages, timing clock, and no hanging notes after a stop.
    fn midi_transport(&mut self) {
        if !self.midi_on {
            return;
        }
        if self.playing != self.midi_running {
            self.midi_running = self.playing;
            if self.playing {
                self.midi_clock_due = 0.0;
                if self.midi_clock {
                    self.midi_emit(MidiOutMsg::Start);
                }
            } else {
                self.release_midi(|_| true);
                if self.midi_clock {
                    self.midi_emit(MidiOutMsg::Stop);
                }
            }
        }
        if self.playing && self.midi_clock {
            // 24 clocks per beat; a beat lasts 60 / bpm seconds.
            let interval = self.sample_rate as f64 * 60.0 / self.bpm.max(1.0) as f64 / 24.0;
            while self.midi_clock_due <= 0.0 {
                self.midi_emit(MidiOutMsg::Clock);
                self.midi_clock_due += interval.max(1.0);
            }
        }
    }

    /// Send note-off for every sounding MIDI note that `which` selects.
    fn release_midi(&mut self, which: impl Fn(&MidiNote) -> bool) {
        let mut i = 0;
        while i < self.midi_notes.len() {
            if which(&self.midi_notes[i]) {
                let n = self.midi_notes.swap_remove(i);
                self.midi_emit(MidiOutMsg::NoteOff { channel: n.channel, note: n.note });
            } else {
                i += 1;
            }
        }
    }

    fn midi_note_on(&mut self, track: usize, column: usize, m: trakr_core::MidiOut, note: u8, vol: f32) {
        if !self.midi_on {
            return;
        }
        let channel = m.channel & 15;
        self.release_midi(|n| n.track == track && n.column == column);
        if let Some(program) = m.program {
            if self.midi_program[channel as usize] != program {
                self.midi_program[channel as usize] = program;
                self.midi_emit(MidiOutMsg::Program { channel, program });
            }
        }
        if self.midi_notes.len() >= MAX_MIDI_NOTES {
            let n = self.midi_notes.remove(0);
            self.midi_emit(MidiOutMsg::NoteOff { channel: n.channel, note: n.note });
        }
        let velocity = (vol.clamp(0.0, 1.0) * 127.0).round().clamp(1.0, 127.0) as u8;
        let note = note.min(127);
        self.midi_notes.push(MidiNote { track, column, channel, note });
        self.midi_emit(MidiOutMsg::NoteOn { channel, note, velocity });
    }

    /// Random byte for probabilistic effects (xorshift64*, no allocation).
    fn next_chance(&mut self) -> u8 {
        let mut x = self.chance;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.chance = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
    }

    fn retire_run(&mut self, run: PhraseRun) {
        if let Err(rtrb::PushError::Full(Garbage::Phrase(p))) = self.garbage.push(Garbage::Phrase(run.phrase))
        {
            std::mem::forget(p);
        }
    }

    fn clear_phrases(&mut self) {
        while let Some(run) = self.phrase_runs.pop() {
            self.retire_run(run);
        }
    }

    fn phrase_voice_base(col: usize) -> usize {
        PHRASE_COL + col.min(LIVE_COL) * MAX_NOTE_COLUMNS
    }

    /// End the phrase started by `col` of `track` and release what it is still sounding.
    fn stop_phrase(&mut self, track: usize, col: usize) {
        let Some(i) = self.phrase_runs.iter().position(|r| r.track == track && r.col == col.min(LIVE_COL))
        else {
            return;
        };
        let run = self.phrase_runs.remove(i);
        let base = Self::phrase_voice_base(run.col);
        self.release_midi(|n| n.track == track && (base..base + MAX_NOTE_COLUMNS).contains(&n.column));
        for v in self
            .voices
            .iter_mut()
            .filter(|v| v.track == track && (base..base + MAX_NOTE_COLUMNS).contains(&v.column))
        {
            v.release();
        }
        self.retire_run(run);
    }

    #[allow(clippy::too_many_arguments)]
    fn start_phrase(
        &mut self,
        track: usize,
        col: usize,
        inst: u8,
        phrase: usize,
        note: u8,
        vol: f32,
        pan: f32,
    ) {
        let Some(p) = self.song.instruments.get(inst as usize).and_then(|i| i.phrases.get(phrase)).cloned()
        else {
            return;
        };
        self.stop_phrase(track, col);
        if self.phrase_runs.len() >= MAX_PHRASE_RUNS {
            return;
        }
        let shift = note as i32 - p.base_note as i32;
        self.phrase_runs.push(PhraseRun {
            track,
            col: col.min(LIVE_COL),
            inst,
            phrase: p,
            shift,
            vol,
            pan,
            line: 0,
            frames_to_line: 0.0,
        });
    }

    /// Fire every phrase line that is due and move each run on.
    fn step_phrases(&mut self) {
        let mut i = 0;
        while i < self.phrase_runs.len() {
            if self.phrase_runs[i].frames_to_line > 0.0 {
                i += 1;
                continue;
            }
            let phrase = self.phrase_runs[i].phrase.clone();
            self.fire_phrase_line(i, &phrase);
            let fpl = Song::frames_per_line(self.bpm, phrase.lpb.max(1), self.sample_rate).max(1.0);
            let mut next = self.phrase_runs[i].line + 1;
            let mut finished = false;
            if phrase.looping {
                if next >= phrase.effective_loop_end() {
                    next = phrase.effective_loop_start();
                }
            } else if next >= phrase.number_of_lines {
                finished = true;
            }
            if finished {
                let run = self.phrase_runs.remove(i);
                self.retire_run(run);
                continue;
            }
            let r = &mut self.phrase_runs[i];
            r.line = next;
            r.frames_to_line += fpl;
        }
    }

    fn fire_phrase_line(&mut self, i: usize, phrase: &Phrase) {
        let (track, col, inst, shift, run_vol, run_pan, l) = {
            let r = &self.phrase_runs[i];
            (r.track, r.col, r.inst, r.shift, r.vol, r.pan, r.line)
        };
        let Some(line) = phrase.line(l) else { return };
        let fpl = Song::frames_per_line(self.bpm, phrase.lpb.max(1), self.sample_rate);
        let base = Self::phrase_voice_base(col);
        for (ci, c) in line.notes.iter().enumerate().take(phrase.note_columns as usize) {
            let vcol = base + ci;
            if c.note == NOTE_OFF {
                for v in self.voices.iter_mut().filter(|v| v.track == track && v.column == vcol) {
                    v.release();
                }
            } else if c.note < NOTE_OFF {
                let note = (c.note as i32 + shift).clamp(0, 119) as u8;
                let inst = if c.instrument != EMPTY { c.instrument } else { inst };
                let vol = if c.volume <= 0x80 { c.volume as f32 / 128.0 } else { 1.0 } * run_vol;
                let pan = if c.panning <= 0x80 { c.panning as f32 / 64.0 - 1.0 } else { run_pan };
                let at = c.delay as f64 / 256.0 * fpl;
                let kind = EventKind::NoteOn {
                    inst,
                    note,
                    vol,
                    pan,
                    opts: TriggerOpts::default(),
                    fx: [EffectCmd::default(); 3],
                };
                if at > 0.0 {
                    self.schedule(at, track, vcol, kind);
                } else {
                    self.note_on(track, vcol, inst, note, vol, pan, TriggerOpts::default());
                }
            } else {
                for v in self.voices.iter_mut().filter(|v| v.track == track && v.column == vcol) {
                    if c.volume <= 0x80 {
                        v.set_col_volume(c.volume as f32 / 128.0);
                    }
                    if c.panning <= 0x80 {
                        v.set_pan(c.panning as f32 / 64.0 - 1.0);
                    }
                }
            }
            if !c.effect.is_empty() {
                for v in self
                    .voices
                    .iter_mut()
                    .filter(|v| v.track == track && v.column == vcol && !v.is_releasing())
                {
                    v.apply_effect(c.effect.id, c.effect.amount);
                }
            }
        }
        for fx in line.effects.iter().take(phrase.effect_columns as usize).filter(|e| !e.is_empty()) {
            for v in self.voices.iter_mut().filter(|v| {
                v.track == track && (base..base + MAX_NOTE_COLUMNS).contains(&v.column) && !v.is_releasing()
            }) {
                v.apply_effect(fx.id, fx.amount);
            }
        }
    }

    fn trigger_line(&mut self) {
        let song = self.song.clone(); // refcount bump only
        let Some(pat) = song.pattern_at(self.seq_pos) else { return };
        let fpl = Song::frames_per_line(self.bpm, self.lpb, self.sample_rate);
        let fpt = fpl / self.tpl.max(1) as f64;
        for (ti, track) in pat.tracks.iter().enumerate() {
            if song.slot_muted(self.seq_pos, ti) {
                continue;
            }
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
                b"ZK" if fx.amount >= 1 => this.tpl = fx.amount,
                b"0L" if ti < MAX_TRACKS => this.fx_vol[ti] = Some(fx.amount as f32 / 128.0),
                b"0P" if ti < MAX_TRACKS => {
                    this.fx_pan[ti] = Some((fx.amount as f32 / 128.0 - 1.0).clamp(-1.0, 1.0))
                }
                b"0H" if ti < MAX_TRACKS => this.fx_width[ti] = Some(fx.amount as f32 / 128.0),
                b"0X" => {
                    this.release_plugin(|n| n.track == ti);
                    for v in this.voices.iter_mut().filter(|v| v.track == ti) {
                        v.cut();
                    }
                }
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

            let track_glide = line.effects.iter().find(|e| &e.id == b"0G").map(|e| e.amount);
            for (ci, col) in line.notes.iter().enumerate() {
                if col.note == NOTE_OFF {
                    self.stop_phrase(ti, ci);
                    let at = col.delay as f64 / 256.0 * fpl;
                    if at > 0.0 {
                        self.schedule(at, ti, ci, EventKind::Release);
                    } else {
                        self.release_plugin(|n| n.track == ti && n.column == ci);
                        self.release_midi(|n| n.track == ti && n.column == ci);
                        for v in self.voices.iter_mut().filter(|v| v.track == ti && v.column == ci) {
                            v.release();
                        }
                    }
                } else if col.note != NOTE_EMPTY && col.instrument != EMPTY {
                    let vol = if col.volume <= 0x80 { col.volume as f32 / 128.0 } else { 1.0 };
                    let pan = if col.panning <= 0x80 { col.panning as f32 / 64.0 - 1.0 } else { 0.0 };
                    if &col.effect.id == b"0Y" && self.next_chance() > col.effect.amount {
                        continue; // "maybe": the note plays with probability amount/255
                    }
                    let phrase =
                        song.instruments.get(col.instrument as usize).and_then(|i| i.phrase_for(col.note));
                    if let Some(pi) = phrase {
                        let at = col.delay as f64 / 256.0 * fpl;
                        if at > 0.0 {
                            let kind = EventKind::Phrase {
                                inst: col.instrument,
                                phrase: pi,
                                note: col.note,
                                vol,
                                pan,
                            };
                            self.schedule(at, ti, ci, kind);
                        } else {
                            self.start_phrase(ti, ci, col.instrument, pi, col.note, vol, pan);
                        }
                        continue;
                    }
                    self.stop_phrase(ti, ci);
                    // Glide: retarget the sounding note in this column instead of retriggering.
                    let glide = if &col.effect.id == b"0G" { Some(col.effect.amount) } else { track_glide };
                    if let Some(speed) = glide {
                        if let Some(v) = self
                            .voices
                            .iter_mut()
                            .find(|v| v.track == ti && v.column == ci && !v.is_releasing())
                        {
                            let target = v.semis_for_note(col.note);
                            v.glide_to(target, speed);
                            continue;
                        }
                    }
                    let opts = TriggerOpts { offset, backwards };
                    let at = col.delay as f64 / 256.0 * fpl + delay_ticks as f64 * fpt;
                    let mut fx = [EffectCmd::default(); 3];
                    for (d, f) in fx.iter_mut().zip(
                        std::iter::once(&col.effect).chain(line.effects.iter()).filter(|f| !f.is_empty()),
                    ) {
                        *d = *f;
                    }
                    let kind = EventKind::NoteOn { inst: col.instrument, note: col.note, vol, pan, opts, fx };
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
                            let kind = EventKind::NoteOn {
                                inst: col.instrument,
                                note: col.note,
                                vol,
                                pan,
                                opts,
                                fx: [EffectCmd::default(); 3],
                            };
                            self.schedule(t, ti, ci, kind);
                            t += r as f64 * fpt;
                        }
                    }
                } else if col.note == NOTE_EMPTY {
                    // Volume and panning columns on a line without a note change the sounding note.
                    for v in self.voices.iter_mut().filter(|v| v.track == ti && v.column == ci) {
                        if col.volume <= 0x80 {
                            v.set_col_volume(col.volume as f32 / 128.0);
                        }
                        if col.panning <= 0x80 {
                            v.set_pan(col.panning as f32 / 64.0 - 1.0);
                        }
                    }
                }
            }

            // Per-tick effects: track-wide from the effect columns, per-column from the note slots.
            for fx in line.effects.iter().filter(|e| !e.is_empty()) {
                for v in self.voices.iter_mut().filter(|v| v.track == ti && !v.is_releasing()) {
                    v.apply_effect(fx.id, fx.amount);
                }
            }
            for (ci, col) in line.notes.iter().enumerate().filter(|(_, c)| !c.effect.is_empty()) {
                for v in
                    self.voices.iter_mut().filter(|v| v.track == ti && v.column == ci && !v.is_releasing())
                {
                    v.apply_effect(col.effect.id, col.effect.amount);
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
        if let Some((first, last)) = self.block {
            if self.line > last {
                self.line = first;
            }
            return;
        }
        if self.line >= lines || brk.is_some() {
            self.line = 0;
            if !self.loop_pattern {
                match self.song.loop_range {
                    Some((a, b)) if self.seq_pos == b && a <= b => self.seq_pos = a,
                    _ => {
                        self.seq_pos += 1;
                        if self.seq_pos >= self.song.sequence.len() {
                            self.seq_pos = 0;
                            if self.stop_at_end {
                                self.playing = false;
                            }
                        }
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

/// Build every track's device chain from the song's device list (unknown ids become pass-throughs).
pub fn build_chains(song: &Song, sample_rate: u32) -> Vec<Vec<Box<dyn Device>>> {
    song.tracks
        .iter()
        .map(|t| t.devices.iter().map(|d| devices::create_or_passthrough(&d.id, sample_rate)).collect())
        .collect()
}

/// Render `song` once through the sequence (plus ring-out) to interleaved stereo f32, offline.
/// Stops at the end of the song, when everything has decayed, or after `max_seconds`.
pub fn render_song(song: Arc<Song>, sample_rate: u32, max_seconds: f32) -> Vec<f32> {
    let chains = build_chains(&song, sample_rate);
    render_song_with_chains(song, sample_rate, max_seconds, chains)
}

/// Like [`render_song`] with caller-built device chains (so hosted plugins can take part).
pub fn render_song_with_chains(
    song: Arc<Song>,
    sample_rate: u32,
    max_seconds: f32,
    chains: Vec<Vec<Box<dyn Device>>>,
) -> Vec<f32> {
    render_song_with_plugins(song, sample_rate, max_seconds, chains, Vec::new())
}

/// Like [`render_song_with_chains`] with instrument plugin devices too, as (instrument index, device).
pub fn render_song_with_plugins(
    song: Arc<Song>,
    sample_rate: u32,
    max_seconds: f32,
    chains: Vec<Vec<Box<dyn Device>>>,
    instruments: Vec<(usize, Box<dyn Device>)>,
) -> Vec<f32> {
    let song = if song.loop_range.is_some() {
        let mut s = (*song).clone();
        s.loop_range = None; // an offline render plays the whole song once
        Arc::new(s)
    } else {
        song
    };
    let (mut engine, mut handle) = create(sample_rate);
    handle.send(Command::SetSong(song));
    for (track, chain) in chains.into_iter().enumerate() {
        if !chain.is_empty() {
            handle.send(Command::SetChain { track, chain });
        }
    }
    for (instrument, device) in instruments {
        handle.send(Command::SetInstrumentPlugin { instrument, device: Some(device) });
    }
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

    fn with_phrase(mut song: Song, f: impl FnOnce(&mut Phrase)) -> Song {
        let mut p = Phrase::new("p", 4);
        p.lpb = 1;
        f(&mut p);
        song.instruments[0].phrases.push(Arc::new(p));
        song.instruments[0].phrase_mode = trakr_core::PhraseMode::Keymap;
        song
    }

    fn onsets(v: &[f32]) -> Vec<usize> {
        let mut out = Vec::new();
        let mut quiet = true;
        for (i, s) in v.iter().enumerate() {
            let loud = s.abs() > 1e-6;
            if loud && quiet {
                out.push(i);
            }
            quiet = !loud;
        }
        out
    }

    #[test]
    fn phrase_plays_its_lines_at_its_own_speed() {
        let mut song = with_phrase(song_with(vec![1.0; 100], LoopMode::Off), |p| {
            p.looping = false;
            p.line_mut(0).notes[0].note = 48;
            p.line_mut(2).notes[0].note = 48;
        });
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let out = render(song, 4000);
        assert_eq!(onsets(&out), vec![0, 2000]);
    }

    #[test]
    fn phrase_transposes_relative_to_its_base_note() {
        let mut song = with_phrase(song_with(vec![1.0; 100], LoopMode::Off), |p| {
            p.looping = false;
            p.line_mut(0).notes[0].note = 48;
        });
        put(&mut song, 0, |l| {
            l.notes[0].note = 60; // one octave above the base note: twice as fast, half as long
            l.notes[0].instrument = 0;
        });
        let out = render(song, 1000);
        let len = out.iter().filter(|s| s.abs() > 1e-6).count();
        assert!((45..=55).contains(&len), "len {len}");
    }

    #[test]
    fn phrases_loop_and_note_off_stops_them() {
        let mut song = with_phrase(song_with(vec![1.0; 100], LoopMode::Off), |p| {
            p.number_of_lines = 1;
            p.line_mut(0).notes[0].note = 48;
        });
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let out = render(song.clone(), 4000);
        assert_eq!(onsets(&out), vec![0, 1000, 2000, 3000]);
        put(&mut song, 2, |l| l.notes[0].note = NOTE_OFF);
        let out = render(song, 4000);
        assert_eq!(onsets(&out), vec![0, 1000]);
    }

    #[test]
    fn phrase_mode_off_plays_the_sample_directly() {
        let mut song = with_phrase(song_with(vec![1.0; 100], LoopMode::Off), |p| {
            p.line_mut(2).notes[0].note = 48;
        });
        song.instruments[0].phrase_mode = trakr_core::PhraseMode::Off;
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let out = render(song, 4000);
        assert_eq!(onsets(&out), vec![0]);
    }

    #[test]
    fn phrase_key_range_chooses_the_phrase() {
        let mut song = with_phrase(song_with(vec![1.0; 100], LoopMode::Off), |p| {
            p.key_range = (60, 119);
            p.looping = false;
            p.line_mut(1).notes[0].note = 48;
        });
        put(&mut song, 0, |l| note(&mut l.notes[0])); // C-4 is outside the range: plays the sample, no phrase
        put(&mut song, 2, |l| {
            l.notes[0].note = 60;
            l.notes[0].instrument = 0;
        });
        let out = render(song, 4000);
        assert_eq!(onsets(&out), vec![0, 3000]);
    }

    /// Render with MIDI output on and collect what the engine queued.
    fn render_midi(song: Song, frames: usize, clock: bool) -> (Vec<f32>, Vec<(u64, MidiOutMsg)>) {
        use trakr_midi::pump::Source;
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetMidiOut { enabled: true, clock });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        engine.render(&mut out);
        let mut src = h.midi_out.take().unwrap();
        let mut msgs = Vec::new();
        while let Some(m) = src.poll() {
            msgs.push(m);
        }
        assert_eq!(src.frames_rendered(), frames as u64);
        (out.chunks_exact(2).map(|f| f[0].max(f[1])).collect(), msgs)
    }

    #[test]
    fn midi_instrument_sends_notes_with_velocity_and_releases_them() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        song.instruments[0].midi_out = Some(trakr_core::MidiOut { channel: 3, program: Some(12) });
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.notes[0].volume = 0x40;
        });
        put(&mut song, 1, |l| l.notes[0].note = NOTE_OFF);
        let (audio, msgs) = render_midi(song, 3000, false);
        assert_eq!(
            msgs,
            vec![
                (0, MidiOutMsg::Program { channel: 3, program: 12 }),
                (0, MidiOutMsg::NoteOn { channel: 3, note: 48, velocity: 64 }),
                (1000, MidiOutMsg::NoteOff { channel: 3, note: 48 }),
            ]
        );
        assert!(audio.iter().any(|s| s.abs() > 0.0), "the instrument's sample still plays");
    }

    #[test]
    fn a_new_note_in_the_column_ends_the_previous_midi_note_and_stop_releases_all() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        song.instruments[0].midi_out = Some(trakr_core::MidiOut::default());
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| {
            note(&mut l.notes[0]);
            l.notes[0].note = 50;
        });
        let (_, msgs) = render_midi(song, 1500, false);
        let kinds: Vec<_> = msgs.iter().map(|m| (m.0, m.1)).collect();
        assert_eq!(
            kinds,
            vec![
                (0, MidiOutMsg::NoteOn { channel: 0, note: 48, velocity: 127 }),
                (1000, MidiOutMsg::NoteOff { channel: 0, note: 48 }),
                (1000, MidiOutMsg::NoteOn { channel: 0, note: 50, velocity: 127 }),
            ]
        );
    }

    #[test]
    fn midi_clock_runs_24_per_beat_between_start_and_stop() {
        // BPM 60: one beat per second, so 24 clocks per 1000 frames.
        let song = song_with(vec![0.0; 10], LoopMode::Off);
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::SetMidiOut { enabled: true, clock: true });
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2000 * 2];
        engine.render(&mut out);
        h.send(Command::Stop);
        engine.render(&mut out[..200]);
        use trakr_midi::pump::Source;
        let mut src = h.midi_out.take().unwrap();
        let mut msgs = Vec::new();
        while let Some(m) = src.poll() {
            msgs.push(m);
        }
        assert_eq!(msgs.first(), Some(&(0, MidiOutMsg::Start)));
        assert_eq!(msgs.last().map(|m| m.1), Some(MidiOutMsg::Stop));
        let clocks: Vec<u64> = msgs.iter().filter(|m| m.1 == MidiOutMsg::Clock).map(|m| m.0).collect();
        assert_eq!(clocks.len(), 48, "two beats of clock"); // frames 0..2000
        let step = clocks[1] - clocks[0];
        assert!((41..=42).contains(&step), "spacing {step}");
    }

    #[test]
    fn midi_is_silent_unless_enabled_and_muted_tracks_stay_quiet() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        song.instruments[0].midi_out = Some(trakr_core::MidiOut::default());
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song.clone())));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 1000 * 2];
        engine.render(&mut out);
        use trakr_midi::pump::Source;
        assert!(h.midi_out.as_mut().unwrap().poll().is_none(), "disabled output queues nothing");
        song.tracks[0].mute = MuteState::Muted;
        let (_, msgs) = render_midi(song, 1000, true);
        assert!(msgs.iter().all(|m| !matches!(m.1, MidiOutMsg::NoteOn { .. })), "{msgs:?}");
    }

    fn render_stereo(song: Song, frames: usize) -> Vec<(f32, f32)> {
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        engine.render(&mut out);
        out.chunks_exact(2).map(|f| (f[0], f[1])).collect()
    }

    fn fx_at(l: &mut trakr_core::Line, slot: usize, id: &[u8; 2], amount: u8) {
        l.effects[slot] = EffectCmd { id: *id, amount };
    }

    #[test]
    fn track_volume_and_pan_commands() {
        let mut song = song_with(vec![1.0; 3000], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            fx_at(l, 0, b"0L", 0x40);
            fx_at(l, 1, b"0P", 0x00);
        });
        let out = render_stereo(song, 800);
        let (l, r) = out[500];
        assert!((l - 0.5).abs() < 0.02 && r.abs() < 1e-4, "{l} {r}");
    }

    #[test]
    fn note_volume_command_sets_the_playing_note() {
        let mut song = song_with(vec![1.0; 3000], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.notes[0].effect = EffectCmd { id: *b"0M", amount: 0x40 };
        });
        let out = render_stereo(song, 800);
        assert!((out[500].0 - 0.5).abs() < 0.03, "{}", out[500].0);
    }

    #[test]
    fn maybe_command_gates_notes_by_probability() {
        let mut never = song_with(vec![1.0; 100], LoopMode::Off);
        let mut always = never.clone();
        for line in 0..6 {
            put(&mut never, line, |l| {
                note(&mut l.notes[0]);
                l.notes[0].effect = EffectCmd { id: *b"0Y", amount: 0 };
            });
            put(&mut always, line, |l| {
                note(&mut l.notes[0]);
                l.notes[0].effect = EffectCmd { id: *b"0Y", amount: 0xFF };
            });
        }
        assert!(onsets(&render(never, 6000)).len() <= 1, "amount 0 almost never plays");
        assert_eq!(onsets(&render(always, 6000)).len(), 6);
    }

    #[test]
    fn stop_track_command_cuts_everything() {
        let mut song = song_with(vec![1.0; 100_000], LoopMode::Forward);
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| fx_at(l, 0, b"0X", 0));
        let out = render(song, 3000);
        assert!(out[500] > 0.9 && out[1500].abs() < 1e-3 && out[2900].abs() < 1e-3);
    }

    #[test]
    fn ticks_per_line_command_rescales_timing() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        put(&mut song, 0, |l| fx_at(l, 0, b"ZK", 8));
        put(&mut song, 1, |l| {
            note(&mut l.notes[0]);
            fx_at(l, 0, b"0Q", 4); // 4 of 8 ticks: half a line
        });
        assert_eq!(first_nonzero(&render(song, 3000)), Some(1500));
    }

    #[test]
    fn auto_pan_swings_the_note_between_channels() {
        let mut song = song_with(vec![1.0; 100_000], LoopMode::Forward);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.notes[0].effect = EffectCmd { id: *b"0N", amount: 0x4F };
        });
        let out = render_stereo(song, 2000);
        let (lmin, lmax) = out[100..].iter().fold((f32::MAX, 0.0f32), |(a, b), s| (a.min(s.0), b.max(s.0)));
        assert!(lmax > 0.9 && lmin < 0.2, "{lmin} {lmax}");
    }

    #[test]
    fn cpu_load_and_voice_count_are_published() {
        let mut song = song_with(vec![1.0; 100_000], LoopMode::Forward);
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 512 * 2];
        for _ in 0..5 {
            engine.render(&mut out);
        }
        assert_eq!(h.pos.voices(), 1);
        let load = h.pos.cpu_load();
        assert!(load > 0.0 && load < 1.0, "{load}");
    }

    #[test]
    fn beat_sync_stretches_the_sample_over_the_given_lines() {
        let len = |lines: u32, bpm: f32| {
            let mut song = song_with(vec![1.0; 1000], LoopMode::Off);
            song.bpm = bpm;
            song.instruments[0].samples[0].beat_sync_lines = lines;
            put(&mut song, 0, |l| note(&mut l.notes[0]));
            render(song, 8000).iter().filter(|s| s.abs() > 1e-6).count()
        };
        assert!((len(0, 60.0) as i32 - 1000).abs() < 20, "off: native length");
        assert!((len(2, 60.0) as i32 - 2000).abs() < 40, "two lines of 1000 frames: {}", len(2, 60.0));
        assert!((len(2, 120.0) as i32 - 1000).abs() < 40, "twice the tempo, half the length");
    }

    #[test]
    fn slot_mutes_silence_a_track_in_one_slot_only() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        Arc::make_mut(&mut song.patterns[0]).number_of_lines = 2;
        song.sequence = vec![0, 0];
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        song.insert_slot(2, 0, trakr_core::SlotMeta::default());
        song.slot_meta[0].mutes = 1; // track 0 is silent in slot 0
        song.sequence.truncate(2);
        song.slot_meta.truncate(2);
        // slot 0 lasts 2 lines = 2000 frames; the second slot's note is at 2000
        assert_eq!(onsets(&render(song, 5000)), vec![2000]);
    }

    #[test]
    fn groove_delays_every_other_line() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        song.groove = vec![0.0, 0.25];
        put(&mut song, 1, |l| note(&mut l.notes[0]));
        put(&mut song, 2, |l| note(&mut l.notes[0]));
        put(&mut song, 3, |l| note(&mut l.notes[0]));
        // line 1 starts a quarter late; lines 2 and 3 follow the 1250 / 750 pattern
        assert_eq!(onsets(&render(song, 5000)), vec![1250, 2000, 3250]);
    }

    #[test]
    fn play_block_loops_the_chosen_lines() {
        let mut song = song_with(vec![1.0; 100], LoopMode::Off);
        put(&mut song, 1, |l| note(&mut l.notes[0]));
        put(&mut song, 2, |l| note(&mut l.notes[0]));
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::PlayBlock { seq_pos: 0, first: 1, last: 2 });
        let mut out = vec![0.0; 2 * 5500];
        engine.render(&mut out);
        let mono: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        assert_eq!(onsets(&mono), vec![0, 1000, 2000, 3000, 4000, 5000]);
        h.send(Command::Stop);
        let mut more = vec![0.0; 2 * 3000];
        engine.render(&mut more);
        assert!(onsets(&more.chunks_exact(2).map(|f| f[0]).collect::<Vec<_>>()).is_empty());
    }

    #[test]
    fn macros_scale_targets_and_pattern_commands_move_them() {
        let setup = |amount: Option<u8>| {
            let mut song = song_with(vec![1.0; 30_000], LoopMode::Forward);
            song.instruments[0].mods.push(trakr_core::InstMod {
                source: trakr_core::ModSource::Macro(0),
                target: trakr_core::LfoTarget::Volume,
                depth: 1.0,
            });
            put(&mut song, 0, |l| note(&mut l.notes[0]));
            if let Some(a) = amount {
                put(&mut song, 1, |l| l.effects[0] = EffectCmd { id: *b"M1", amount: a });
            }
            render(song, 3000)
        };
        let still = setup(None);
        assert!((still[500] - 1.0).abs() < 0.05, "the middle position changes nothing: {}", still[500]);
        let off = setup(Some(0));
        assert!(
            off[1500] < 0.05 && off[500] > 0.9,
            "macro down silences it after the command: {}",
            off[1500]
        );
        let up = setup(Some(255));
        assert!(up[1500] > 1.9, "macro up doubles it: {}", up[1500]);
    }

    #[test]
    fn fade_out_lets_a_released_note_die_away() {
        let mut song = song_with(vec![1.0; 30_000], LoopMode::Forward);
        song.instruments[0].fade_out = 0.5;
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| l.notes[0].note = NOTE_OFF);
        let out = render(song, 3000);
        assert!(out[1000 + 50] > 0.8, "starts fading from full: {}", out[1050]);
        assert!(out[1250] > 0.3 && out[1250] < 0.7, "half way: {}", out[1250]);
        assert!(out[1700] < 0.02, "gone after the fade: {}", out[1700]);
        // without it the note ends at once
        let mut plain = song_with(vec![1.0; 30_000], LoopMode::Forward);
        put(&mut plain, 0, |l| note(&mut l.notes[0]));
        put(&mut plain, 1, |l| l.notes[0].note = NOTE_OFF);
        assert!(render(plain, 3000)[1100] < 0.05);
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
    fn effects_next_to_a_delayed_note_apply_to_it() {
        // 0M sets the note volume; with a delay column the voice does not exist at the line start.
        let level = |delay: u8| {
            let mut song = song_with(vec![1.0; 1000], LoopMode::Off);
            put(&mut song, 0, |l| {
                note(&mut l.notes[0]);
                l.notes[0].delay = delay;
                l.notes[0].effect = EffectCmd { id: *b"0M", amount: 0x20 }; // quarter volume
            });
            let out = render(song, 900);
            out[700]
        };
        let (direct, delayed) = (level(0), level(128));
        assert!((direct - 0.25).abs() < 0.02, "{direct}");
        assert!((delayed - direct).abs() < 0.02, "delayed note volume {delayed} vs {direct}");
        // a track-level effect column too (0Q delay by ticks plus 0M in another effect column)
        let mut song = song_with(vec![1.0; 1000], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = EffectCmd { id: *b"0Q", amount: 2 };
            l.effects[1] = EffectCmd { id: *b"0M", amount: 0x40 };
        });
        let out = render(song, 900);
        assert!((out[700] - 0.5).abs() < 0.02, "{}", out[700]);
    }

    #[test]
    fn track_width_effect_narrows_and_widens() {
        // a hard-left stereo signal: mono makes both sides equal, width 0x80 keeps it
        let song = song_with(vec![1.0; 1000], LoopMode::Off);
        let sides = |amount: u8| {
            let mut s = song.clone();
            put(&mut s, 0, |l| {
                note(&mut l.notes[0]);
                l.notes[0].panning = 0; // hard left
                l.effects[0] = EffectCmd { id: *b"0H", amount };
            });
            let (mut engine, mut h) = create(RATE);
            h.send(Command::SetSong(Arc::new(s)));
            h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
            let mut out = vec![0.0; 2 * 400];
            engine.render(&mut out);
            (out[2 * 300], out[2 * 300 + 1])
        };
        let (l0, r0) = sides(0);
        assert!((l0 - r0).abs() < 1e-4, "width 0 is mono: {l0} {r0}");
        let (l1, r1) = sides(0x80);
        let (lw, rw) = sides(0xFF);
        assert!((l1 - l0).abs() > 0.0 || (r1 - r0).abs() > 0.0);
        assert!(
            (lw - rw).abs() > (l1 - r1).abs() - 1e-6,
            "wider is at least as wide: {lw} {rw} vs {l1} {r1}"
        );
    }

    #[test]
    fn pan_slide_moves_the_note_across_the_field() {
        let mut song = song_with(vec![1.0; 3000], LoopMode::Off);
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = EffectCmd { id: *b"0K", amount: 0x0F }; // right, 15/64 per tick
        });
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2 * 2500];
        engine.render(&mut out);
        let early = (out[2 * 100], out[2 * 100 + 1]);
        let late = (out[2 * 2400], out[2 * 2400 + 1]);
        assert!(late.0 < early.0 * 0.5, "left side fades as the pan slides right: {early:?} -> {late:?}");
        assert!(late.1 >= early.1 - 1e-4);
    }

    #[test]
    fn tempo_automation_changes_the_line_length() {
        // 4 lines of one second each at 60 BPM / LPB 1; a tempo lane ramps up to 255 BPM
        let line_starts = |lane: Option<trakr_core::Automation>| {
            let mut song = song_with(vec![1.0; 50], LoopMode::Off);
            for l in 0..4 {
                put(&mut song, l, |ln| note(&mut ln.notes[0]));
            }
            if let Some(a) = lane {
                Arc::make_mut(&mut song.patterns[0]).automation.push(a);
            }
            let out = render(song, 6000);
            out.iter()
                .enumerate()
                .filter(|(i, s)| **s > 0.5 && (*i == 0 || out[i - 1] < 0.5))
                .map(|(i, _)| i)
                .collect::<Vec<_>>()
        };
        let plain = line_starts(None);
        assert_eq!(plain.len().min(4), 4);
        let mut a = trakr_core::Automation::new(0, trakr_core::AutoTarget::Bpm);
        a.set_point(0.0, trakr_core::AutoTarget::bpm_value(240.0));
        let fast = line_starts(Some(a));
        assert!(
            fast[1] < plain[1] / 2,
            "240 BPM lines are about a quarter the length: {fast:?} vs {plain:?}"
        );
        // a stepped lane switches tempo at its second point
        let mut a = trakr_core::Automation::new(0, trakr_core::AutoTarget::Bpm);
        a.curve = trakr_core::AutoCurve::Step;
        a.set_point(0.0, trakr_core::AutoTarget::bpm_value(60.0));
        a.set_point(2.0, trakr_core::AutoTarget::bpm_value(240.0));
        let s = line_starts(Some(a));
        assert!((s[1] as i32 - 1000).abs() < 30 && (s[2] as i32 - 2000).abs() < 30, "{s:?}");
        assert!(s[3] - s[2] < 400, "faster after the switch: {s:?}");
    }

    #[test]
    fn finished_and_stopped_voices_are_dropped_on_the_ui_thread() {
        let song = song_with(vec![1.0; 50], LoopMode::Off);
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        for track in 0..8 {
            h.send(Command::NoteOn { track, instrument: 0, note: 48, volume: 1.0 });
        }
        let mut out = vec![0.0; 2 * 400];
        engine.render(&mut out); // the 50-frame samples end inside this block
        let mut retired = 0;
        while let Ok(g) = h.garbage.pop() {
            retired += matches!(g, Garbage::Voice(_)) as usize;
        }
        assert_eq!(retired, 8, "every finished voice went through the garbage queue");
        let long = song_with(vec![1.0; 5000], LoopMode::Off);
        h.send(Command::SetSong(Arc::new(long)));
        for track in 0..8 {
            h.send(Command::NoteOn { track, instrument: 0, note: 48, volume: 1.0 });
        }
        engine.render(&mut out);
        while h.garbage.pop().is_ok() {}
        h.send(Command::Stop);
        engine.render(&mut out);
        let mut stopped = 0;
        while let Ok(g) = h.garbage.pop() {
            stopped += matches!(g, Garbage::Voice(_)) as usize;
        }
        assert_eq!(stopped, 8, "stopping retires the sounding voices");
        assert!(engine.is_idle());
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

    // ---- per-tick effects, envelopes, mixer ----

    /// Song with a long ramp sample (value = frame / 100000) so output slope reveals pitch, or a
    /// constant sample so output level reveals gain. BPM 60 / LPB 1 / TPL 4: line = 1000 frames, tick = 250.
    fn ramp_song() -> Song {
        song_with((0..100_000).map(|i| i as f32 / 100_000.0).collect(), LoopMode::Off)
    }

    fn hold_song() -> Song {
        song_with(vec![1.0; 100_000], LoopMode::Off)
    }

    fn put_t(song: &mut Song, track: usize, line: u16, f: impl FnOnce(&mut trakr_core::Line)) {
        f(Arc::make_mut(&mut song.patterns[0]).line_mut(track, line));
    }

    fn fx(id: &[u8; 2], amount: u8) -> EffectCmd {
        EffectCmd { id: *id, amount }
    }

    /// Output slope (per frame, x 1e5) around frame `at`.
    fn slope(out: &[f32], at: usize) -> f32 {
        (out[at + 10] - out[at - 10]) / 20.0 * 100_000.0
    }

    #[test]
    fn pitch_slide_up_and_down() {
        for (id, up) in [(b"0U", true), (b"0D", false)] {
            let mut song = ramp_song();
            put(&mut song, 0, |l| {
                note(&mut l.notes[0]);
                l.effects[0] = fx(id, 0x10); // one semitone per tick
            });
            let out = render(song, 1000);
            let (a, b) = (slope(&out, 100), slope(&out, 900));
            assert!((a - 1.0).abs() < 0.05, "start slope {a}");
            if up {
                assert!(b > a * 1.15, "slide up: {a} -> {b}");
            } else {
                assert!(b < a * 0.9, "slide down: {a} -> {b}");
            }
        }
    }

    #[test]
    fn glide_reaches_the_new_note_without_retriggering() {
        let mut song = ramp_song();
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| {
            l.notes[0].note = 60; // an octave up
            l.notes[0].instrument = 0;
            l.notes[0].effect = fx(b"0G", 0x40); // 4 semitones per tick
        });
        let out = render(song, 2000);
        // no retrigger: the ramp keeps climbing instead of restarting near zero at frame 1000
        assert!(out[1100] > out[900]);
        let (a, b) = (slope(&out, 900), slope(&out, 1900));
        assert!((b / a - 2.0).abs() < 0.15, "glide ended at {b} vs start {a}");
    }

    #[test]
    fn vibrato_wobbles_the_pitch() {
        let mut song = ramp_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0V", 0x4F); // 2 Hz, ~1.9 semitones
        });
        let out = render(song, 1000);
        let slopes: Vec<f32> = (1..19).map(|i| slope(&out, i * 50)).collect();
        let (lo, hi) = slopes.iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(*s), h.max(*s)));
        assert!(hi / lo > 1.1, "vibrato range {lo}..{hi}");
    }

    #[test]
    fn arpeggio_cycles_pitch_per_tick() {
        let mut song = ramp_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0A", 0x0C); // base, +0, +12 over ticks 0, 1, 2
        });
        let out = render(song, 1000);
        let (base, up) = (slope(&out, 100), slope(&out, 650));
        assert!((up / base - 2.0).abs() < 0.2, "arp {base} -> {up}");
    }

    #[test]
    fn fade_in_and_out() {
        let mut song = hold_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0I", 4);
        });
        let out = render(song, 1100);
        assert!(out[100] < out[500] && out[500] < out[900], "{} {} {}", out[100], out[500], out[900]);
        assert!(out[1050] > 0.9);

        let mut song = hold_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0O", 2);
        });
        let out = render(song, 1000);
        assert!(out[100] > 0.7);
        assert!(out[700].abs() < 0.01, "still audible: {}", out[700]);
    }

    #[test]
    fn volume_slide_and_tremolo() {
        let mut song = hold_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0W", 0x08); // down 8/64 per tick
        });
        let out = render(song, 1000);
        assert!(out[900] < out[100] * 0.8, "{} -> {}", out[100], out[900]);

        let mut song = hold_song();
        put(&mut song, 0, |l| {
            note(&mut l.notes[0]);
            l.effects[0] = fx(b"0T", 0x4F);
        });
        let out = render(song, 1000);
        let (lo, hi) = out[50..900].iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(*s), h.max(*s)));
        assert!(hi - lo > 0.3, "tremolo range {lo}..{hi}");
    }

    #[test]
    fn volume_column_changes_a_sounding_note() {
        let mut song = hold_song();
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| l.notes[0].volume = 0x40); // half volume, no new note
        let out = render(song, 1500);
        assert!(out[500] > 0.9);
        assert!((out[1400] - 0.5).abs() < 0.05, "{}", out[1400]);
    }

    #[test]
    fn volume_envelope_with_sustain_and_release() {
        let mut song = hold_song();
        Arc::make_mut(&mut song.instruments[0].volume_env)
            .clone_from(&trakr_core::Envelope::adsr(0.1, 0.1, 0.5, 0.2));
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        put(&mut song, 1, |l| l.notes[0].note = NOTE_OFF);
        let out = render(song, 1500);
        assert!((out[50] - 0.5).abs() < 0.08, "attack {}", out[50]);
        assert!((out[600] - 0.5).abs() < 0.05, "sustain {}", out[600]);
        assert!(out[1100] > 0.1 && out[1100] < 0.4, "release {}", out[1100]);
        assert!(out[1400].abs() < 0.01, "after end {}", out[1400]);
    }

    #[test]
    fn pitch_envelope_bends_the_note() {
        let mut song = ramp_song();
        // 0 -> +12 semitones over half a second
        let env = trakr_core::Envelope {
            enabled: true,
            points: vec![(0.0, 0.5), (0.5, 1.0)],
            ..Default::default()
        };
        *Arc::make_mut(&mut song.instruments[0].pitch_env) = env;
        put(&mut song, 0, |l| note(&mut l.notes[0]));
        let out = render(song, 600);
        assert!(slope(&out, 450) > slope(&out, 60) * 1.7);
    }

    #[test]
    fn solo_off_and_master_volume() {
        let mut song = hold_song();
        put_t(&mut song, 1, 0, |l| note(&mut l.notes[0]));
        song.tracks[0].solo = true; // track 1 (index 1) is not soloed
        assert_eq!(first_nonzero(&render(song.clone(), 200)), None);
        song.tracks[0].solo = false;
        assert!(first_nonzero(&render(song.clone(), 200)).is_some());
        song.tracks[1].mute = MuteState::Off;
        assert_eq!(first_nonzero(&render(song.clone(), 200)), None);
        song.tracks[1].mute = MuteState::Active;
        let n = song.tracks.len();
        song.tracks[n - 1].volume = 0.5; // master
        let out = render(song, 200);
        assert!((out[100] - 0.5).abs() < 0.01);
    }

    #[test]
    fn meters_report_track_and_master_levels() {
        let mut song = hold_song();
        put_t(&mut song, 1, 0, |l| note(&mut l.notes[0]));
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; 2 * 300];
        engine.render(&mut out);
        assert!(h.pos.track_peak(1) > 0.4);
        assert_eq!(h.pos.track_peak(0), 0.0);
        assert!(h.pos.master_peak().0 > 0.4);
    }

    #[test]
    fn loop_range_repeats_slots() {
        let mut song = hold_song();
        for _ in 0..2 {
            song.patterns.push(Arc::new(trakr_core::Pattern::new(song.tracks.len(), 2)));
        }
        Arc::make_mut(&mut song.patterns[0]).number_of_lines = 2;
        song.sequence = vec![0, 1, 2];
        song.loop_range = Some((1, 2));
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut seen_after_start = Vec::new();
        for i in 0..20 {
            let mut out = vec![0.0; 2 * 1000];
            engine.render(&mut out);
            if i >= 4 {
                seen_after_start.push(h.pos.get().1);
            }
        }
        assert!(seen_after_start.iter().all(|&s| s == 1 || s == 2), "{seen_after_start:?}");
        assert!(seen_after_start.contains(&1) && seen_after_start.contains(&2));
    }
}

#[cfg(test)]
mod device_tests {
    use super::*;
    use trakr_core::{AutoTarget, Automation, DeviceSpec, Instrument, InstrumentFilter, LoopMode, Sample};

    const RATE: u32 = 44_100;

    fn sine(freq: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(RATE as f32 * secs) as usize)
            .map(|i| (i as f32 / RATE as f32 * freq * std::f32::consts::TAU).sin() * amp)
            .collect()
    }

    /// One-track song playing `data` at its native pitch from line 0 (BPM 60, LPB 1: a line is one second).
    fn song_with(data: Vec<f32>) -> Song {
        let mut song = Song { bpm: 60.0, lpb: 1, tpl: 4, ..Song::default() };
        let mut inst = Instrument::new("t");
        let mut s = Sample::new("t", RATE, 1, data);
        s.loop_mode = LoopMode::Off;
        inst.samples.push(s);
        song.instruments.push(inst);
        let n = &mut Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0];
        n.note = 48;
        n.instrument = 0;
        song
    }

    fn device(song: &mut Song, id: &str, params: &[(usize, f32)]) {
        let mut d = DeviceSpec::new(id).unwrap();
        for &(i, v) in params {
            d.params[i] = v;
        }
        song.tracks[0].devices.push(d);
    }

    /// Render `frames` mono-ified frames with device chains installed.
    fn render(song: Song, frames: usize) -> Vec<f32> {
        let chains = build_chains(&song, RATE);
        let (mut engine, mut h) = create(RATE);
        h.send(Command::SetSong(Arc::new(song)));
        for (track, chain) in chains.into_iter().enumerate() {
            h.send(Command::SetChain { track, chain });
        }
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
        let mut out = vec![0.0; frames * 2];
        // small buffers, like a real audio callback
        for chunk in out.chunks_mut(2 * 256) {
            engine.render(chunk);
            h.collect_garbage();
        }
        out.chunks_exact(2).map(|f| f[0]).collect()
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn lfo_modulates_another_devices_parameter() {
        let mut song = song_with(sine(220.0, 2.0, 0.5));
        // square LFO, depth 1, 2 Hz, driving the gainer in chain slot 2
        device(&mut song, "native.lfo", &[(0, 2.0), (1, 0.0), (2, 3.0), (3, 2.0), (4, 1.0)]);
        device(&mut song, "native.gainer", &[]);
        let out = render(song, 40_000);
        let loud = peak(&out[2000..9000]);
        let quiet = peak(&out[13_000..20_000]);
        assert!(loud > 1.5, "loud half {loud}");
        assert!(quiet < loud * 0.01, "quiet half {quiet} vs {loud}");
    }

    #[test]
    fn modulators_with_no_target_or_bypassed_change_nothing() {
        let base = {
            let mut song = song_with(sine(220.0, 1.0, 0.5));
            device(&mut song, "native.gainer", &[]);
            peak(&render(song, 20_000))
        };
        for (target, bypass) in [(0.0, false), (2.0, true), (8.0, false)] {
            let mut song = song_with(sine(220.0, 1.0, 0.5));
            device(&mut song, "native.lfo", &[(0, target), (4, 1.0)]);
            device(&mut song, "native.gainer", &[]);
            song.tracks[0].devices[0].bypass = bypass;
            let out = peak(&render(song, 20_000));
            assert!(
                (out - base).abs() < 0.02 || target == 8.0 && out.is_finite(),
                "{target} {bypass}: {out} vs {base}"
            );
        }
    }

    #[test]
    fn envelope_follower_opens_a_parameter_with_the_signal_level() {
        let plain = {
            let mut song = song_with(sine(220.0, 1.0, 0.5));
            device(&mut song, "native.gainer", &[]);
            peak(&render(song, 30_000)[10_000..30_000])
        };
        let mut song = song_with(sine(220.0, 1.0, 0.5));
        device(&mut song, "native.follower", &[(0, 2.0), (1, 0.0), (4, 16.0), (5, 1.0)]);
        device(&mut song, "native.gainer", &[]);
        let driven = peak(&render(song, 30_000)[10_000..30_000]);
        assert!(driven > plain * 2.0, "{driven} vs {plain}");
    }

    #[test]
    fn gainer_scales_the_signal() {
        let mut song = song_with(vec![1.0; 20_000]);
        device(&mut song, "native.gainer", &[(0, -6.0206)]);
        let out = render(song, 10_000);
        assert!((out[8000] - 0.5).abs() < 0.01, "{}", out[8000]);
    }

    #[test]
    fn filter_passes_and_blocks() {
        for (mode, freq, expect_loud) in
            [(0.0, 100.0, true), (0.0, 10_000.0, false), (1.0, 100.0, false), (1.0, 10_000.0, true)]
        {
            let mut song = song_with(sine(freq, 0.5, 0.8));
            device(&mut song, "native.filter", &[(0, mode), (1, 1000.0), (2, 0.707)]);
            let out = render(song, 20_000);
            let p = peak(&out[5000..20_000]);
            if expect_loud {
                assert!(p > 0.6, "mode {mode} {freq} Hz: {p}");
            } else {
                assert!(p < 0.1, "mode {mode} {freq} Hz: {p}");
            }
        }
    }

    #[test]
    fn eq_boosts_the_mid_band() {
        let mut song = song_with(sine(1000.0, 0.5, 0.2));
        device(&mut song, "native.eq", &[(2, 12.0), (3, 1000.0), (4, 1.0)]);
        let out = render(song, 20_000);
        let p = peak(&out[5000..20_000]);
        assert!((p / 0.2 - 3.98).abs() < 0.3, "boost was {}", p / 0.2);
    }

    #[test]
    fn delay_echoes_after_the_set_time() {
        let mut impulse = vec![0.0; 100];
        impulse[0] = 1.0;
        let mut song = song_with(impulse);
        device(&mut song, "native.delay", &[(0, 100.0), (1, 0.0), (2, 1.0), (3, 0.0)]);
        let out = render(song, 10_000);
        let first = out.iter().position(|s| s.abs() > 0.1).unwrap();
        assert!((first as i32 - 4410).abs() < 40, "echo at {first}");
        assert!(peak(&out[..4000]) < 0.01, "dry signal leaked");
    }

    #[test]
    fn reverb_rings_out() {
        let mut impulse = vec![0.0; 100];
        impulse[0] = 1.0;
        let mut song = song_with(impulse);
        device(&mut song, "native.reverb", &[(0, 0.8), (1, 0.3), (2, 1.0), (3, 1.0)]);
        let out = render(song, 44_100);
        let early: f32 = out[2000..8000].iter().map(|s| s.abs()).sum();
        let late: f32 = out[30_000..44_000].iter().map(|s| s.abs()).sum();
        assert!(early > 0.5 && late > 0.01, "early {early} late {late}");
        assert!(late < early, "tail should decay");
        assert!(peak(&out) < 1.5);
    }

    #[test]
    fn chorus_changes_the_signal_but_stays_bounded() {
        let dry = render(song_with(sine(440.0, 0.5, 0.5)), 20_000);
        let mut song = song_with(sine(440.0, 0.5, 0.5));
        device(&mut song, "native.chorus", &[(0, 1.0), (1, 5.0), (2, 1.0), (3, 0.0)]);
        let wet = render(song, 20_000);
        let diff: f32 = dry[5000..15_000].iter().zip(&wet[5000..15_000]).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 10.0, "chorus did nothing: {diff}");
        assert!(peak(&wet) < 1.0);
    }

    #[test]
    fn distortion_saturates() {
        let mut song = song_with(sine(200.0, 0.5, 0.5));
        device(&mut song, "native.distortion", &[(0, 30.0), (1, 1.0), (2, 0.0)]);
        let out = render(song, 20_000);
        let p = peak(&out[5000..20_000]);
        assert!(p > 0.95 && p <= 1.0001, "peak {p}");
    }

    #[test]
    fn compressor_reduces_loud_signals() {
        let mut song = song_with(vec![1.0; 30_000]);
        device(&mut song, "native.compressor", &[(0, -20.0), (1, 10.0), (2, 1.0), (3, 50.0), (4, 0.0)]);
        let out = render(song, 20_000);
        // 20 dB over threshold at 10:1 removes 18 dB: gain ~0.126
        assert!((out[19_000] - 0.126).abs() < 0.04, "{}", out[19_000]);
    }

    #[test]
    fn bypass_and_unknown_devices_are_harmless() {
        let mut song = song_with(vec![1.0; 20_000]);
        device(&mut song, "native.gainer", &[(0, -60.0)]);
        song.tracks[0].devices[0].bypass = true;
        song.tracks[0].devices.push(DeviceSpec {
            id: "plugin.unknown".into(),
            bypass: false,
            params: vec![],
            plugin: None,
            text: String::new(),
        });
        let out = render(song, 5000);
        assert!((out[4000] - 1.0).abs() < 0.01);
    }

    #[test]
    fn automation_moves_track_volume_and_device_params() {
        // volume ramps from silence (0) to unity (60/66) over the first line
        let mut song = song_with(vec![1.0; 60_000]);
        let mut a = Automation::new(0, AutoTarget::TrackVolume);
        a.set_point(0.0, 0.0);
        a.set_point(1.0, 60.0 / 66.0);
        Arc::make_mut(&mut song.patterns[0]).automation.push(a);
        let out = render(song, 44_000);
        assert!(out[2000] < 0.2, "{}", out[2000]);
        // at 0.907 of the line the volume is -60 + 66 * 0.824 = -5.6 dB
        assert!((out[40_000] - 0.52).abs() < 0.05, "{}", out[40_000]);
        assert!(out[20_000] > out[10_000]);

        // gainer gain automated from -60 dB (normalized 0) to 0 dB (normalized 60/72)
        let mut song = song_with(vec![1.0; 60_000]);
        device(&mut song, "native.gainer", &[]);
        let mut a = Automation::new(0, AutoTarget::Device { device: 0, param: 0 });
        a.set_point(0.0, 0.0);
        a.set_point(1.0, 60.0 / 72.0);
        Arc::make_mut(&mut song.patterns[0]).automation.push(a);
        let out = render(song, 44_000);
        assert!(out[2000] < 0.1, "{}", out[2000]);
        assert!((out[40_000] - 0.52).abs() < 0.05, "{}", out[40_000]);
    }

    #[test]
    fn instrument_filter_removes_highs() {
        let mut song = song_with(sine(10_000.0, 0.5, 0.8));
        song.instruments[0].filter =
            InstrumentFilter { mode: trakr_core::FilterMode::LowPass, cutoff: 500.0, resonance: 0.707 };
        let out = render(song, 20_000);
        assert!(peak(&out[5000..20_000]) < 0.1);
        // filter envelope opening the cutoff by 5 octaves lets the tone through
        let mut song = song_with(sine(10_000.0, 0.5, 0.8));
        song.instruments[0].filter =
            InstrumentFilter { mode: trakr_core::FilterMode::LowPass, cutoff: 500.0, resonance: 0.707 };
        song.instruments[0].cutoff_env = Arc::new(trakr_core::Envelope {
            enabled: true,
            points: vec![(0.0, 1.0), (1.0, 1.0)],
            sustain: None,
            loop_points: None,
        });
        let out = render(song, 20_000);
        assert!(peak(&out[5000..20_000]) > 0.5);
    }

    fn lp_song(cutoff: f32) -> Song {
        let mut song = song_with(sine(10_000.0, 0.5, 0.8));
        song.instruments[0].filter =
            InstrumentFilter { mode: trakr_core::FilterMode::LowPass, cutoff, resonance: 0.707 };
        song
    }

    #[test]
    fn instrument_lfos_modulate_volume_pitch_and_cutoff() {
        // square volume LFO at 2 Hz, full depth: loud and silent halves
        let mut song = song_with(vec![1.0; 30_000]);
        song.instruments[0].lfos.push(trakr_core::InstLfo {
            target: trakr_core::LfoTarget::Volume,
            shape: trakr_core::LfoShape::Square,
            rate: 2.0,
            depth: 1.0,
            fade_in: 0.0,
            ..Default::default()
        });
        song.instruments[0].samples[0].loop_mode = LoopMode::Forward;
        let out = render(song, 22_000);
        let loud = peak(&out[2000..9000]);
        let quiet = peak(&out[13_000..20_000]);
        assert!(loud > 1.5 && quiet < 0.1, "{loud} {quiet}");
        // an LFO with fade-in starts at no depth
        let mut song = song_with(vec![1.0; 30_000]);
        song.instruments[0].samples[0].loop_mode = LoopMode::Forward;
        song.instruments[0].lfos.push(trakr_core::InstLfo {
            target: trakr_core::LfoTarget::Volume,
            shape: trakr_core::LfoShape::Square,
            rate: 2.0,
            depth: 1.0,
            fade_in: 1.0,
            ..Default::default()
        });
        let out = render(song, 22_000);
        assert!((out[2000] - 1.0).abs() < 0.3 && out[12_000] < 0.9 || out[12_000] > 1.1);
        // cutoff LFO with a filter that blocks the tone: opening it lets the tone through
        let mut song = lp_song(500.0);
        song.instruments[0].lfos.push(trakr_core::InstLfo {
            target: trakr_core::LfoTarget::Cutoff,
            shape: trakr_core::LfoShape::Square,
            rate: 1.0,
            depth: 5.0,
            fade_in: 0.0,
            ..Default::default()
        });
        song.instruments[0].samples[0].loop_mode = LoopMode::Forward;
        let out = render(song, 22_000);
        assert!(peak(&out[10_000..20_000]) < 0.2 || peak(&out[1000..9000]) > 0.4);
    }

    #[test]
    fn stepper_lfo_follows_its_steps() {
        let mut song = song_with(vec![1.0; 30_000]);
        song.instruments[0].samples[0].loop_mode = LoopMode::Forward;
        let mut l = trakr_core::InstLfo {
            target: trakr_core::LfoTarget::Volume,
            shape: trakr_core::LfoShape::Stepper,
            rate: 1.0, // one cycle per second
            depth: 0.5,
            ..Default::default()
        };
        l.steps = 4;
        l.step_values[..4].copy_from_slice(&[1.0, 0.0, -1.0, 0.0]);
        song.instruments[0].lfos.push(l);
        let out = render(song, 44_100);
        // steps of a quarter second: gain 1.5, 1.0, 0.5, 1.0
        let at = |s: f32| out[(s * RATE as f32) as usize];
        assert!((at(0.1) - 1.5).abs() < 0.1 && (at(0.35) - 1.0).abs() < 0.1, "{} {}", at(0.1), at(0.35));
        assert!((at(0.6) - 0.5).abs() < 0.1 && (at(0.85) - 1.0).abs() < 0.1, "{} {}", at(0.6), at(0.85));
    }

    #[test]
    fn key_tracking_and_velocity_move_the_cutoff() {
        let tone = |track: f32, note: u8, volume: u8, vel_cut: f32| {
            let mut song = lp_song(2000.0);
            song.instruments[0].key_track = track;
            song.instruments[0].vel_cutoff = vel_cut;
            song.instruments[0].samples[0].base_note = note; // keep the pitch fixed at 10 kHz
            let n = &mut Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0];
            n.note = note;
            n.volume = volume;
            peak(&render(song, 20_000)[5000..20_000])
        };
        let flat = tone(0.0, 48, 0x80, 0.0);
        let tracked_up = tone(1.0, 84, 0x80, 0.0); // three octaves up: cutoff 16 kHz, tone passes
        let tracked_flat = tone(0.0, 84, 0x80, 0.0);
        assert!(tracked_up > tracked_flat * 3.0, "{tracked_up} {tracked_flat}");
        assert!(flat < 0.1);
        let bright = tone(0.0, 48, 0x80, 0.0);
        let soft = tone(0.0, 48, 0x20, 3.0);
        assert!(soft <= bright + 1e-6);
    }

    #[test]
    fn sidechain_compressor_ducks_a_track_with_another_tracks_signal() {
        // track 1: steady tone from line 0; track 2 (muted): a loud burst from line 1 acting as the key
        let mut song = song_with(vec![0.0; 10]);
        let mut tone = Instrument::new("tone");
        let mut ts = Sample::new("tone", RATE, 1, sine(220.0, 3.0, 0.5));
        ts.loop_mode = LoopMode::Forward;
        tone.samples.push(ts);
        let mut kick = Instrument::new("kick");
        kick.samples.push(Sample::new("kick", RATE, 1, vec![0.9; 3000]));
        song.instruments = vec![tone, kick];
        let pat = Arc::make_mut(&mut song.patterns[0]);
        let n = &mut pat.line_mut(0, 0).notes[0];
        n.note = 48;
        n.instrument = 0;
        let n = &mut pat.line_mut(1, 1).notes[0];
        n.note = 48;
        n.instrument = 1;
        song.tracks[1].mute = MuteState::Muted;
        device(&mut song, "native.sidechain", &[(0, 2.0), (1, -30.0), (2, 20.0), (3, 0.1), (4, 5.0)]);
        let out = render(song, 50_000);
        let before = peak(&out[30_000..44_000]);
        let during = peak(&out[45_000..46_800]);
        assert!(before > 0.4, "{before}");
        assert!(during < before * 0.2, "ducked while the key plays: {during} vs {before}");
    }

    #[test]
    fn hydra_drives_several_parameters_from_one_oscillator() {
        let mut song = song_with(sine(220.0, 2.0, 0.5));
        // square hydra: slot 2's gain follows it at full depth, and so does slot 3's (the latter inverted)
        device(
            &mut song,
            "native.hydra",
            &[(0, 3.0), (1, 2.0), (2, 2.0), (3, 0.0), (4, 1.0), (5, 3.0), (6, 0.0), (7, -1.0)],
        );
        device(&mut song, "native.gainer", &[]);
        device(&mut song, "native.gainer", &[]);
        let out = render(song, 40_000);
        let a = peak(&out[2000..9000]);
        let b = peak(&out[13_000..20_000]);
        assert!(a > 3.0 * b || b > 3.0 * a, "the two halves differ: {a} {b}");
    }

    #[test]
    fn offline_render_includes_devices() {
        let mut impulse = vec![0.0; 100];
        impulse[0] = 1.0;
        let mut song = song_with(impulse);
        device(&mut song, "native.delay", &[(0, 500.0), (1, 0.0), (2, 1.0), (3, 0.0)]);
        // 64-line pattern is far longer than the echo, so the echo must be in the render
        let out = render_song(Arc::new(song), RATE, 5.0);
        assert!(out.iter().skip(2 * 20_000).any(|s| s.abs() > 0.1), "echo missing from the bounce");
    }

    // ---- groups and sends ----

    fn insert_track(song: &mut Song, index: usize, kind: trakr_core::TrackKind, name: &str) -> usize {
        song.tracks.insert(index, trakr_core::Track::new(name, kind));
        index
    }

    #[test]
    fn group_tracks_sum_their_members_and_apply_their_own_chain_and_fader() {
        let mut song = hold_song_dev();
        let g = insert_track(&mut song, 8, trakr_core::TrackKind::Group, "Group");
        song.tracks[0].group = Some(g);
        let plain = render(song.clone(), 3000)[2500];
        assert!((plain - 1.0).abs() < 0.01, "a group with no changes is transparent: {plain}");
        song.tracks[g].volume = 0.25;
        assert!((render(song.clone(), 3000)[2500] - 0.25).abs() < 0.01);
        song.tracks[g].volume = 1.0;
        let mut d = DeviceSpec::new("native.gainer").unwrap();
        d.params[0] = -6.0206;
        song.tracks[g].devices.push(d);
        assert!((render(song, 3000)[2500] - 0.5).abs() < 0.01, "the group's own device chain runs");
    }

    #[test]
    fn nested_groups_route_through_each_other() {
        let mut song = hold_song_dev();
        let inner = insert_track(&mut song, 8, trakr_core::TrackKind::Group, "Inner");
        let outer = insert_track(&mut song, 9, trakr_core::TrackKind::Group, "Outer");
        song.tracks[0].group = Some(inner);
        song.tracks[inner].group = Some(outer);
        song.tracks[inner].volume = 0.5;
        song.tracks[outer].volume = 0.5;
        assert!((render(song.clone(), 3000)[2500] - 0.25).abs() < 0.01);
        // a cycle must not hang or blow up: the cyclic link is ignored
        song.tracks[outer].group = Some(inner);
        let out = render(song, 3000);
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn send_tracks_add_a_wet_feed_and_respect_pre_post_fader() {
        let mut song = hold_song_dev();
        let n = song.tracks.len();
        let s = insert_track(&mut song, n, trakr_core::TrackKind::Send, "Send");
        song.tracks[0].sends.push(trakr_core::SendLevel { target: s, level: 0.5, pre_fader: false });
        let out = render(song.clone(), 3000)[2500];
        assert!((out - 1.5).abs() < 0.02, "dry 1.0 + send 0.5, got {out}");
        // the send track's own chain applies to the feed only
        let mut with_fx = song.clone();
        let mut d = DeviceSpec::new("native.gainer").unwrap();
        d.params[0] = -60.0;
        with_fx.tracks[s].devices.push(d);
        assert!((render(with_fx, 3000)[2500] - 1.0).abs() < 0.02, "muted send leaves only the dry signal");
        // fader down: post-fader send follows it, pre-fader send does not
        song.tracks[0].volume = 0.0;
        assert!(render(song.clone(), 3000)[2500].abs() < 0.02);
        song.tracks[0].sends[0].pre_fader = true;
        assert!((render(song.clone(), 3000)[2500] - 0.5).abs() < 0.02);
        // muting the track silences even a pre-fader send
        song.tracks[0].mute = MuteState::Muted;
        assert!(render(song, 3000)[2500].abs() < 0.02);
    }

    fn hold_song_dev() -> Song {
        song_with(vec![1.0; 20_000])
    }

    // ---- NNA and velocity zones ----

    fn nna_song(mode: trakr_core::Nna) -> Song {
        let mut song = song_with(vec![1.0; 100_000]);
        song.instruments[0].nna = mode;
        *Arc::make_mut(&mut song.instruments[0].volume_env) =
            trakr_core::Envelope::adsr(0.01, 0.01, 1.0, 0.2);
        // instrument 1: a 10-frame click, played on line 1 in the same column
        let mut click = Instrument::new("click");
        click.samples.push(Sample::new("c", RATE, 1, vec![0.0; 10]));
        song.instruments.push(click);
        let n = &mut Arc::make_mut(&mut song.patterns[0]).line_mut(0, 1).notes[0];
        n.note = 48;
        n.instrument = 1;
        song
    }

    #[test]
    fn nna_decides_what_happens_to_the_previous_note() {
        // line 1 starts at frame 44100 (one second per line). Probe 100 ms and 500 ms later.
        let (a, b) = (44_100 + 4410, 44_100 + 22_050);
        let cut = render(nna_song(trakr_core::Nna::Cut), b + 100);
        assert!(cut[a].abs() < 0.01, "cut should be silent: {}", cut[a]);
        let off = render(nna_song(trakr_core::Nna::NoteOff), b + 100);
        assert!(off[a] > 0.2 && off[a] < 0.8, "release in progress: {}", off[a]);
        assert!(off[b].abs() < 0.01, "release over: {}", off[b]);
        let cont = render(nna_song(trakr_core::Nna::Continue), b + 100);
        assert!(cont[a] > 0.9 && cont[b] > 0.9, "continue keeps sounding: {} {}", cont[a], cont[b]);
    }

    #[test]
    fn velocity_zones_choose_the_layer() {
        let mut song = song_with(vec![0.2; 5000]);
        song.instruments[0].samples[0].vel_range = (0, 63);
        let mut hard = Sample::new("hard", RATE, 1, vec![0.9; 5000]);
        hard.vel_range = (64, 127);
        song.instruments[0].samples.push(hard);
        let set_vol = |song: &mut Song, v: u8| {
            Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0].volume = v;
        };
        set_vol(&mut song, 0x20); // 0.25 -> velocity 32 -> soft layer
        let soft = render(song.clone(), 500)[400];
        set_vol(&mut song, 0x70); // 0.875 -> velocity 111 -> hard layer
        let hard = render(song, 500)[400];
        assert!((soft - 0.05).abs() < 0.01, "soft layer {soft}");
        assert!((hard - 0.79).abs() < 0.03, "hard layer {hard}");
    }

    #[test]
    fn showcase_song_renders_loud_but_never_clips() {
        let song = Arc::new(Song::showcase());
        let out = render_song(song, 44_100, 120.0);
        let secs = out.len() as f32 / 2.0 / 44_100.0;
        assert!(secs > 25.0 && secs < 40.0, "4 patterns of 64 lines at 124 BPM ~ 31 s, got {secs}");
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.3, "too quiet: {peak}");
        assert!(peak <= 1.0, "limiter should hold the master: {peak}");
        let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt();
        assert!(rms > 0.03, "rms {rms}");
    }

    // ---- randomized robustness ----

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next() % n.max(1)
        }
        fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
            &v[self.below(v.len() as u32) as usize]
        }
    }

    const FX_IDS: [&[u8; 2]; 31] = [
        b"ZT", b"ZL", b"ZB", b"0S", b"0B", b"0C", b"0Q", b"0R", b"0U", b"0D", b"0G", b"0V", b"0T", b"0A",
        b"0W", b"0I", b"0O", b"XX", b"00", b"ZZ", b"0M", b"0N", b"0Y", b"0L", b"0P", b"0X", b"ZK", b"M1",
        b"M2", b"M4", b"M9",
    ];

    /// Stands in for a hosted plugin: scales the signal by the first parameter.
    struct FuzzClap;

    impl Device for FuzzClap {
        fn id(&self) -> &'static str {
            "clap"
        }
        fn process(&mut self, params: &[f32], buf: &mut [f32]) {
            assert!(params.len() <= trakr_core::MAX_PLUGIN_PARAMS);
            assert!(params.iter().all(|p| p.is_finite() || p.is_nan()), "{params:?}");
            let g = params.first().copied().filter(|g| g.is_finite()).unwrap_or(1.0).clamp(-4.0, 4.0);
            for s in buf {
                *s *= g;
            }
        }
    }

    fn random_song(rng: &mut Rng) -> Song {
        let mut song = Song {
            bpm: 40.0 + rng.below(200) as f32,
            lpb: 1 + rng.below(8) as u8,
            tpl: 1 + rng.below(16) as u8,
            ..Song::default()
        };
        // instruments with odd samples: tiny, looped with odd points, stereo, envelopes, filters
        for i in 0..1 + rng.below(4) {
            let mut inst = Instrument::new(format!("i{i}"));
            for _ in 0..1 + rng.below(3) {
                let frames = 1 + rng.below(3000) as usize;
                let ch = 1 + rng.below(2) as u8;
                let data: Vec<f32> = (0..frames * ch as usize).map(|k| ((k as f32) * 0.07).sin()).collect();
                let mut s = Sample::new("s", 4000 + rng.below(80_000), ch, data);
                s.loop_mode =
                    *rng.pick(&[LoopMode::Off, LoopMode::Forward, LoopMode::Reverse, LoopMode::PingPong]);
                s.loop_start = rng.below(frames as u32 + 50) as usize;
                s.loop_end = rng.below(frames as u32 + 50) as usize;
                s.base_note = rng.below(120) as u8;
                s.fine_tune = rng.below(256) as u8 as i8;
                s.panning = rng.below(300) as f32 / 100.0 - 1.5;
                s.volume = rng.below(500) as f32 / 100.0;
                s.beat_sync_lines = if rng.below(3) == 0 { rng.below(40) } else { 0 };
                inst.samples.push(s);
            }
            if rng.below(2) == 0 {
                *Arc::make_mut(&mut inst.volume_env) =
                    trakr_core::Envelope::adsr(rng.below(100) as f32 / 100.0, 0.05, 0.5, 0.1);
            }
            inst.filter = InstrumentFilter {
                mode: *rng.pick(&[
                    trakr_core::FilterMode::Off,
                    trakr_core::FilterMode::LowPass,
                    trakr_core::FilterMode::BandPass,
                ]),
                cutoff: 20.0 + rng.below(20_000) as f32,
                resonance: 0.1 + rng.below(1000) as f32 / 100.0,
            };
            inst.nna =
                *rng.pick(&[trakr_core::Nna::Cut, trakr_core::Nna::NoteOff, trakr_core::Nna::Continue]);
            for _ in 0..rng.below(6) {
                inst.lfos.push(trakr_core::InstLfo {
                    target: *rng.pick(&[
                        trakr_core::LfoTarget::Volume,
                        trakr_core::LfoTarget::Pan,
                        trakr_core::LfoTarget::Pitch,
                        trakr_core::LfoTarget::Cutoff,
                        trakr_core::LfoTarget::Resonance,
                    ]),
                    shape: *rng.pick(&[
                        trakr_core::LfoShape::Sine,
                        trakr_core::LfoShape::Triangle,
                        trakr_core::LfoShape::Saw,
                        trakr_core::LfoShape::Square,
                        trakr_core::LfoShape::Random,
                        trakr_core::LfoShape::SmoothRandom,
                        trakr_core::LfoShape::Stepper,
                    ]),
                    rate: rng.below(100_000) as f32 / 1000.0 - 20.0,
                    depth: rng.below(100_000) as f32 / 1000.0 - 50.0,
                    fade_in: rng.below(3000) as f32 / 100.0 - 5.0,
                    ..Default::default()
                });
            }
            for _ in 0..rng.below(10) {
                inst.mods.push(trakr_core::InstMod {
                    source: {
                        let k = rng.below(9) as u8;
                        *rng.pick(&[
                            trakr_core::ModSource::Key,
                            trakr_core::ModSource::Velocity,
                            trakr_core::ModSource::Macro(k),
                        ])
                    },
                    target: *rng.pick(&[
                        trakr_core::LfoTarget::Volume,
                        trakr_core::LfoTarget::Pan,
                        trakr_core::LfoTarget::Pitch,
                        trakr_core::LfoTarget::Cutoff,
                        trakr_core::LfoTarget::Resonance,
                    ]),
                    depth: rng.below(4000) as f32 / 100.0 - 20.0,
                });
            }
            inst.macros = [rng.below(100) as f32 / 50.0 - 0.5; 4];
            inst.fade_out = rng.below(300) as f32 / 100.0 - 0.5;
            if rng.below(3) == 0 {
                *Arc::make_mut(&mut inst.volume_env) = trakr_core::Envelope {
                    enabled: true,
                    points: vec![(0.0, 0.0), (0.05, 1.0), (0.2, 0.5), (0.4, 0.0)],
                    sustain: if rng.below(2) == 0 { Some(rng.below(6) as usize) } else { None },
                    loop_points: if rng.below(2) == 0 {
                        Some((rng.below(6) as usize, rng.below(6) as usize))
                    } else {
                        None
                    },
                };
            }
            inst.key_track = rng.below(1000) as f32 / 100.0 - 3.0;
            inst.vel_cutoff = rng.below(1000) as f32 / 100.0 - 3.0;
            if rng.below(3) == 0 {
                inst.midi_out = Some(trakr_core::MidiOut {
                    channel: rng.below(40) as u8,
                    program: if rng.below(2) == 0 { Some(rng.below(200) as u8) } else { None },
                });
            }
            if rng.below(2) == 0 {
                inst.phrase_mode = trakr_core::PhraseMode::Keymap;
                for _ in 0..1 + rng.below(3) {
                    let mut p = Phrase::new("p", 1 + rng.below(20) as u16);
                    p.lpb = rng.below(40) as u8;
                    p.looping = rng.below(2) == 0;
                    p.loop_start = rng.below(30) as u16;
                    p.loop_end = rng.below(30) as u16;
                    p.base_note = rng.below(130) as u8;
                    p.key_range = (rng.below(120) as u8, rng.below(120) as u8);
                    p.note_columns = rng.below(15) as u8;
                    p.effect_columns = rng.below(12) as u8;
                    for _ in 0..rng.below(30) {
                        let l = p.line_mut(rng.below(25) as u16);
                        let n = &mut l.notes[rng.below(trakr_core::MAX_NOTE_COLUMNS as u32) as usize];
                        n.note = *rng.pick(&[48u8, 0, 119, 120, 121, 60]);
                        n.instrument = rng.below(6) as u8;
                        n.volume = rng.below(256) as u8;
                        n.delay = rng.below(256) as u8;
                        n.effect = EffectCmd { id: **rng.pick(&FX_IDS), amount: rng.below(256) as u8 };
                    }
                    inst.phrases.push(Arc::new(p));
                }
            }
            song.instruments.push(inst);
        }
        // devices at extreme settings on random tracks
        for t in 0..song.tracks.len() {
            for _ in 0..rng.below(4) {
                let info = rng.pick(trakr_core::DEVICES);
                let mut d = DeviceSpec::new(info.id).unwrap();
                for (i, p) in info.params.iter().enumerate() {
                    d.params[i] = *rng.pick(&[p.min, p.max, p.default, (p.min + p.max) * 0.5]);
                }
                d.bypass = rng.below(8) == 0;
                song.tracks[t].devices.push(d);
            }
            song.tracks[t].volume = rng.below(400) as f32 / 100.0;
            song.tracks[t].panning = rng.below(300) as f32 / 100.0 - 1.5;
        }
        // hosted plugins with hostile descriptions and parameter lists
        for t in 0..song.tracks.len() {
            if rng.below(3) != 0 {
                continue;
            }
            let params: Vec<trakr_core::PluginParam> = (0..rng.below(80))
                .map(|i| trakr_core::PluginParam {
                    id: i,
                    name: "p".into(),
                    min: *rng.pick(&[0.0, -5.0, f32::NAN, f32::INFINITY, 3.0]),
                    max: *rng.pick(&[1.0, 0.0, f32::NAN, f32::NEG_INFINITY, 2.0, 1e30]),
                    default: *rng.pick(&[0.5, f32::NAN, 100.0]),
                    stepped: rng.below(2) == 0,
                })
                .collect();
            let mut spec = DeviceSpec::from_plugin(trakr_core::PluginRef {
                path: "x".into(),
                plugin_id: "fuzz".into(),
                name: "fuzz".into(),
                params,
            });
            spec.params = (0..rng.below(100)).map(|_| rng.below(1000) as f32 / 100.0 - 5.0).collect();
            spec.bypass = rng.below(6) == 0;
            song.tracks[t].devices.push(spec);
        }
        let ntracks = song.tracks.len();
        let lines = 1 + rng.below(40) as u16;
        let mut pat = trakr_core::Pattern::new(ntracks, lines);
        for _ in 0..rng.below(200) {
            let (t, l) = (rng.below(ntracks as u32) as usize, rng.below(lines as u32 + 5) as u16);
            let line = pat.line_mut(t, l);
            let c = rng.below(trakr_core::MAX_NOTE_COLUMNS as u32) as usize;
            let n = &mut line.notes[c];
            n.note = *rng.pick(&[48u8, 0, 119, 120, 121, 60, 24, 255]);
            n.instrument = rng.below(6) as u8;
            n.volume = rng.below(256) as u8;
            n.panning = rng.below(256) as u8;
            n.delay = rng.below(256) as u8;
            n.effect = EffectCmd { id: **rng.pick(&FX_IDS), amount: rng.below(256) as u8 };
            let e = rng.below(trakr_core::MAX_EFFECT_COLUMNS as u32) as usize;
            line.effects[e] = EffectCmd { id: **rng.pick(&FX_IDS), amount: rng.below(256) as u8 };
        }
        if rng.below(2) == 0 {
            let mut a =
                trakr_core::Automation::new(rng.below(ntracks as u32) as usize, AutoTarget::TrackVolume);
            a.set_point(0.0, 0.5);
            a.set_point(lines as f32, 1.0);
            pat.automation.push(a);
        }
        song.patterns = vec![Arc::new(pat)];
        song.sequence = vec![0, 0];
        song.slot_meta = vec![trakr_core::SlotMeta { mutes: rng.below(1 << 10) as u64, section: None }];
        song.groove = (0..rng.below(10)).map(|_| *rng.pick(&[0.0, 0.3, -0.7, f32::NAN, 5.0])).collect();
        song
    }

    #[test]
    fn random_songs_never_panic_or_produce_garbage() {
        let mut rng = Rng(0xC0FFEE);
        for round in 0..50 {
            let song = random_song(&mut rng);
            let rate = *rng.pick(&[8000u32, 22_050, 44_100, 48_000, 96_000]);
            let mut chains = build_chains(&song, rate);
            for (t, tr) in song.tracks.iter().enumerate() {
                for (i, d) in tr.devices.iter().enumerate() {
                    if d.plugin.is_some() {
                        chains[t][i] = Box::new(FuzzClap);
                    }
                }
            }
            let (mut engine, mut h) = create(rate);
            h.send(Command::SetSong(Arc::new(song.clone())));
            for (track, chain) in chains.into_iter().enumerate() {
                h.send(Command::SetChain { track, chain });
            }
            h.send(Command::SetMidiOut { enabled: rng.below(2) == 0, clock: rng.below(2) == 0 });
            h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: rng.below(2) == 0 });
            // buffer sizes a real driver might use, including odd ones
            for _ in 0..(rate as usize / 256) {
                let n = *rng.pick(&[64usize, 100, 256, 512, 1000, 1024, 2048]);
                let mut out = vec![0.0f32; n * 2];
                engine.render(&mut out);
                h.collect_garbage();
                if !out.iter().all(|s| s.is_finite()) {
                    let mut no_dev = song.clone();
                    no_dev.tracks.iter_mut().for_each(|t| t.devices.clear());
                    let mut no_filter = song.clone();
                    no_filter
                        .instruments
                        .iter_mut()
                        .for_each(|i| i.filter.mode = trakr_core::FilterMode::Off);
                    let devs: Vec<String> = song
                        .tracks
                        .iter()
                        .enumerate()
                        .flat_map(|(t, tr)| {
                            tr.devices.iter().map(move |d| format!("t{t}:{}:{:?}", d.id, d.params))
                        })
                        .collect();
                    let mut culprits = Vec::new();
                    for (t, tr) in song.tracks.iter().enumerate() {
                        for d in 0..tr.devices.len() {
                            let mut one = song.clone();
                            one.tracks[t].devices.remove(d);
                            if finite_for(&one, rate) {
                                culprits.push(format!("t{t}:{}", tr.devices[d].id));
                            }
                        }
                    }
                    panic!(
                        "round {round} rate {rate}: culprits {culprits:?}; non-finite output. finite without devices: {}, without instrument filters: {}. devices: {devs:?}",
                        finite_for(&no_dev, rate),
                        finite_for(&no_filter, rate)
                    );
                }
                // mid-render edits and transport changes must be safe too
                match rng.below(40) {
                    0 => {
                        h.send(Command::Stop);
                        h.send(Command::Play { seq_pos: 0, line: rng.below(50) as u16, loop_pattern: false });
                    }
                    1 => {
                        h.send(Command::SetSong(Arc::new(random_song(&mut rng))));
                    }
                    2 => {
                        h.send(Command::NoteOn {
                            track: rng.below(10) as usize,
                            instrument: rng.below(5) as u8,
                            note: rng.below(130) as u8,
                            volume: 1.0,
                        });
                    }
                    _ => {}
                }
            }
        }
    }

    fn finite_for(song: &Song, rate: u32) -> bool {
        let chains = build_chains(song, rate);
        let (mut engine, mut h) = create(rate);
        h.send(Command::SetSong(Arc::new(song.clone())));
        for (track, chain) in chains.into_iter().enumerate() {
            h.send(Command::SetChain { track, chain });
        }
        h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: true });
        for _ in 0..(rate as usize * 2 / 256) {
            let mut out = vec![0.0f32; 512];
            engine.render(&mut out);
            if !out.iter().all(|s| s.is_finite()) {
                return false;
            }
        }
        true
    }

    // ---- interpolation quality ----

    #[test]
    fn better_interpolation_modes_track_a_pitched_sine_more_closely() {
        use trakr_core::Interpolation::*;
        let f0 = 8000.0f32;
        let semis = 3.0f32;
        let ratio = 2f32.powf(semis / 12.0);
        let rms_error = |mode| {
            let data: Vec<f32> = (0..20_000)
                .map(|i| (i as f32 / RATE as f32 * f0 * std::f32::consts::TAU).sin() * 0.5)
                .collect();
            let mut song = song_with(data);
            song.instruments[0].interpolation = mode;
            Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0].note = 48 + semis as u8;
            let out = render(song, 6000);
            let mut sum = 0.0f32;
            for (i, o) in out.iter().enumerate().take(5000).skip(200) {
                let ideal = (i as f32 / RATE as f32 * f0 * ratio * std::f32::consts::TAU).sin() * 0.5;
                sum += (o - ideal).powi(2);
            }
            (sum / 4800.0).sqrt()
        };
        let (lin, cub, sinc) = (rms_error(Linear), rms_error(Cubic), rms_error(Sinc));
        // An 8 kHz tone pitched up 3 semitones is where cheap interpolation falls apart.
        assert!(cub < lin * 0.5, "cubic {cub} should clearly beat linear {lin}");
        assert!(sinc < cub * 0.2, "sinc {sinc} should be far better than cubic {cub}");
        assert!(sinc < 0.002, "sinc error {sinc}");
    }

    #[test]
    fn sinc_playback_does_not_alias_when_pitched_up() {
        use trakr_core::Interpolation::*;
        // A 15 kHz tone an octave up lands at 30 kHz, above Nyquist: it must vanish, not fold to 14 kHz.
        let rms = |mode| {
            let data: Vec<f32> = (0..30_000)
                .map(|i| (i as f32 / RATE as f32 * 15_000.0 * std::f32::consts::TAU).sin() * 0.5)
                .collect();
            let mut song = song_with(data);
            song.instruments[0].interpolation = mode;
            Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0].note = 48 + 12;
            let out = render(song, 6000);
            (out[1000..5000].iter().map(|v| v * v).sum::<f32>() / 4000.0).sqrt()
        };
        let (cubic, sinc) = (rms(Cubic), rms(Sinc));
        assert!(cubic > 0.05, "the plain interpolators alias audibly ({cubic})");
        assert!(sinc < cubic * 0.15, "band-limited sinc leaves {sinc} vs {cubic}");
        // and it leaves an in-band tone alone
        let tone: Vec<f32> = (0..30_000)
            .map(|i| (i as f32 / RATE as f32 * 2_000.0 * std::f32::consts::TAU).sin() * 0.5)
            .collect();
        let mut song = song_with(tone);
        song.instruments[0].interpolation = Sinc;
        Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0].note = 48 + 12;
        let out = render(song, 6000);
        let level = (out[1000..5000].iter().map(|v| v * v).sum::<f32>() / 4000.0).sqrt();
        assert!((level - 0.3535).abs() < 0.05, "in-band level {level}");
    }

    #[test]
    fn pitch_bend_shifts_sounding_voices() {
        // A full-cycle sine, looped; one octave up should double the zero-crossing rate.
        let data: Vec<f32> =
            (0..2000).map(|i| (i as f32 / 2000.0 * std::f32::consts::TAU * 20.0).sin() * 0.5).collect();
        let mut song = song_with(data);
        song.instruments[0].samples[0].loop_mode = trakr_core::LoopMode::Forward;
        song.instruments[0].samples[0].loop_end = 2000;
        Arc::make_mut(&mut song.patterns[0]).line_mut(0, 0).notes[0].note = 48;
        let crossings = |bend: f32| {
            let (mut e, mut h) = create(RATE);
            h.send(Command::SetSong(Arc::new(song.clone())));
            h.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false });
            h.send(Command::PitchBend(bend));
            let mut out = vec![0.0f32; 8000];
            e.render(&mut out);
            out.chunks(2)
                .map(|f| f[0])
                .collect::<Vec<_>>()
                .windows(2)
                .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
                .count()
        };
        let (flat, up) = (crossings(0.0), crossings(12.0));
        assert!(flat > 10, "baseline crossings {flat}");
        let ratio = up as f32 / flat as f32;
        assert!((1.8..2.2).contains(&ratio), "an octave of bend should double the pitch: {flat} -> {up}");
        let (_, mut h) = create(RATE);
        h.send(Command::PitchBend(f32::NAN)); // must not poison the engine
    }
}
