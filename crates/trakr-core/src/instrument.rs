use crate::envelope::Envelope;
use crate::phrase::{Phrase, PhraseMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Reverse,
    PingPong,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved f32 PCM. Stored outside the song JSON by `trakr-format`.
    #[serde(skip)]
    pub data: Arc<Vec<f32>>,
    /// Internal note at which the sample plays at its original pitch (48 = C-4).
    pub base_note: u8,
    /// -128..=127 maps to -1..+1 semitone.
    pub fine_tune: i8,
    pub volume: f32,
    pub panning: f32,
    pub loop_mode: LoopMode,
    pub loop_start: usize,
    pub loop_end: usize,
    /// Loop only until note-off, then play on to the end of the sample even with a volume envelope.
    #[serde(default)]
    pub loop_release: bool,
    /// Play to the end whatever happens: note-off and release do not stop the sample (drum hits).
    #[serde(default)]
    pub one_shot: bool,
    /// Inclusive key range this sample responds to.
    pub key_range: (u8, u8),
    /// Slice marker positions in frames, ascending.
    #[serde(default)]
    pub slices: Vec<usize>,
    /// 0 = off. Otherwise the sample is played so that it lasts exactly this many pattern lines at the
    /// current tempo (the pitch follows the speed), keeping loops and breaks in time with the song.
    #[serde(default)]
    pub beat_sync_lines: u32,
    /// Inclusive velocity range (0..=127) this sample responds to.
    #[serde(default = "full_velocity")]
    pub vel_range: (u8, u8),
}

impl Sample {
    pub fn new(name: impl Into<String>, sample_rate: u32, channels: u8, data: Vec<f32>) -> Self {
        let frames = data.len() / channels.max(1) as usize;
        Self {
            name: name.into(),
            sample_rate,
            channels: channels.max(1),
            data: Arc::new(data),
            base_note: 48,
            fine_tune: 0,
            volume: 1.0,
            panning: 0.0,
            loop_mode: LoopMode::Off,
            loop_start: 0,
            loop_end: frames,
            loop_release: false,
            one_shot: false,
            key_range: (0, 119),
            slices: Vec::new(),
            beat_sync_lines: 0,
            vel_range: (0, 127),
        }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }

    /// Replace the audio data and keep loop points and slices inside the new length.
    pub(crate) fn set_data_public(&mut self, data: Vec<f32>) {
        self.set_data(data);
    }

    fn set_data(&mut self, data: Vec<f32>) {
        let frames = data.len() / self.channels.max(1) as usize;
        self.data = Arc::new(data);
        self.loop_start = self.loop_start.min(frames);
        self.loop_end = self.loop_end.min(frames).max(self.loop_start);
        self.slices.retain(|&p| p < frames);
    }

    /// Clamp and order a frame range to this sample.
    pub fn clamp_range(&self, a: usize, b: usize) -> (usize, usize) {
        let n = self.frames();
        let (a, b) = (a.min(b).min(n), a.max(b).min(n));
        (a, b)
    }

    /// Apply `f(frame_index_in_range, range_len, &mut [channel samples])` to each frame in `a..b`.
    fn map_range(&mut self, a: usize, b: usize, mut f: impl FnMut(usize, usize, &mut [f32])) {
        let (a, b) = self.clamp_range(a, b);
        if a == b {
            return;
        }
        let ch = self.channels.max(1) as usize;
        let mut out = (*self.data).clone();
        for (i, frame) in out[a * ch..b * ch].chunks_exact_mut(ch).enumerate() {
            f(i, b - a, frame);
        }
        self.set_data(out);
    }

    /// Highest absolute sample value in `a..b`.
    pub fn peak(&self, a: usize, b: usize) -> f32 {
        let (a, b) = self.clamp_range(a, b);
        let ch = self.channels.max(1) as usize;
        self.data[a * ch..b * ch].iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// Scale so the loudest sample hits full scale. No-op for silence.
    pub fn normalize(&mut self) {
        self.normalize_range(0, self.frames());
    }

    /// Scale `a..b` so its loudest sample hits full scale.
    pub fn normalize_range(&mut self, a: usize, b: usize) {
        let peak = self.peak(a, b);
        if peak > 0.0 {
            self.gain(a, b, 1.0 / peak);
        }
    }

    /// Multiply `a..b` by `factor`.
    pub fn gain(&mut self, a: usize, b: usize, factor: f32) {
        self.map_range(a, b, |_, _, f| f.iter_mut().for_each(|s| *s *= factor));
    }

    pub fn silence(&mut self, a: usize, b: usize) {
        self.gain(a, b, 0.0);
    }

    /// Flip the polarity of `a..b`.
    pub fn invert(&mut self, a: usize, b: usize) {
        self.gain(a, b, -1.0);
    }

    /// Subtract the mean of each channel so the waveform is centered on zero.
    pub fn remove_dc(&mut self) {
        let ch = self.channels.max(1) as usize;
        let n = self.frames().max(1) as f64;
        let mut mean = vec![0.0f64; ch];
        for f in self.data.chunks_exact(ch) {
            for (m, s) in mean.iter_mut().zip(f) {
                *m += *s as f64;
            }
        }
        let mean: Vec<f32> = mean.iter().map(|m| (*m / n) as f32).collect();
        let frames = self.frames();
        self.map_range(0, frames, |_, _, f| f.iter_mut().zip(&mean).for_each(|(s, m)| *s -= m));
    }

    pub fn reverse(&mut self) {
        self.reverse_range(0, self.frames());
    }

    pub fn reverse_range(&mut self, a: usize, b: usize) {
        let (a, b) = self.clamp_range(a, b);
        let ch = self.channels.max(1) as usize;
        let mut out = (*self.data).clone();
        let frames: Vec<&[f32]> = self.data[a * ch..b * ch].chunks_exact(ch).collect();
        for (i, f) in frames.iter().rev().enumerate() {
            out[(a + i) * ch..(a + i + 1) * ch].copy_from_slice(f);
        }
        self.set_data(out);
    }

    /// Linear fade from silence over the first `frames` frames.
    pub fn fade_in(&mut self, frames: usize) {
        self.fade_in_range(0, frames);
    }

    /// Linear fade from silence to full over `a..b`.
    pub fn fade_in_range(&mut self, a: usize, b: usize) {
        self.map_range(a, b, |i, n, f| f.iter_mut().for_each(|s| *s *= i as f32 / n as f32));
    }

    /// Linear fade to silence over the last `frames` frames.
    pub fn fade_out(&mut self, frames: usize) {
        let n = self.frames();
        self.fade_out_range(n.saturating_sub(frames), n);
    }

    /// Linear fade from full to silence over `a..b`.
    pub fn fade_out_range(&mut self, a: usize, b: usize) {
        self.map_range(a, b, |i, n, f| f.iter_mut().for_each(|s| *s *= (n - i) as f32 / n as f32));
    }

    /// Keep only frames `start..end`.
    pub fn trim(&mut self, start: usize, end: usize) {
        let (start, end) = self.clamp_range(start, end);
        let ch = self.channels.max(1) as usize;
        self.loop_start = self.loop_start.saturating_sub(start);
        self.loop_end = self.loop_end.saturating_sub(start).min(end - start);
        self.slices = self.slices.iter().filter(|&&p| p >= start && p < end).map(|p| p - start).collect();
        self.set_data(self.data[start * ch..end * ch].to_vec());
    }

    /// Interleaved copy of frames `a..b`.
    pub fn copy_range(&self, a: usize, b: usize) -> Vec<f32> {
        let (a, b) = self.clamp_range(a, b);
        let ch = self.channels.max(1) as usize;
        self.data[a * ch..b * ch].to_vec()
    }

    /// Remove frames `a..b`, shifting later audio, loop points and slices down.
    pub fn delete(&mut self, a: usize, b: usize) {
        let (a, b) = self.clamp_range(a, b);
        if a == b {
            return;
        }
        let ch = self.channels.max(1) as usize;
        let cut = b - a;
        let shift = |p: usize| if p >= b { p - cut } else { p.min(a) };
        self.loop_start = shift(self.loop_start);
        self.loop_end = shift(self.loop_end);
        self.slices = self.slices.iter().filter(|&&p| p < a || p >= b).map(|&p| shift(p)).collect();
        let mut out = Vec::with_capacity(self.data.len() - cut * ch);
        out.extend_from_slice(&self.data[..a * ch]);
        out.extend_from_slice(&self.data[b * ch..]);
        self.set_data(out);
    }

    /// Insert interleaved `data` (same channel count) at frame `at`, shifting later audio up.
    pub fn insert(&mut self, at: usize, data: &[f32]) {
        let ch = self.channels.max(1) as usize;
        let at = at.min(self.frames());
        let n = data.len() / ch;
        if n == 0 {
            return;
        }
        let shift = |p: usize| if p >= at { p + n } else { p };
        self.loop_start = shift(self.loop_start);
        self.loop_end = shift(self.loop_end);
        self.slices = self.slices.iter().map(|&p| shift(p)).collect();
        let mut out = Vec::with_capacity(self.data.len() + n * ch);
        out.extend_from_slice(&self.data[..at * ch]);
        out.extend_from_slice(&data[..n * ch]);
        out.extend_from_slice(&self.data[at * ch..]);
        self.set_data(out);
    }

    pub fn to_mono(&mut self) {
        if self.channels == 2 {
            let out = self.data.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
            self.channels = 1;
            self.set_data(out);
        }
    }

    /// Duplicate a mono sample into both channels.
    pub fn to_stereo(&mut self) {
        if self.channels == 1 {
            let out = self.data.iter().flat_map(|&s| [s, s]).collect();
            self.channels = 2;
            self.set_data(out);
        }
    }

    /// Linear-interpolating resample of the data by `ratio` (new length = old length * ratio).
    fn resample_by(&mut self, ratio: f64) {
        let ch = self.channels.max(1) as usize;
        let old = self.frames();
        let new = ((old as f64 * ratio).round() as usize).max(1);
        let mut out = Vec::with_capacity(new * ch);
        for i in 0..new {
            let pos = i as f64 / ratio;
            let i0 = (pos as usize).min(old.saturating_sub(1));
            let i1 = (i0 + 1).min(old.saturating_sub(1));
            let t = (pos - i0 as f64) as f32;
            for c in 0..ch {
                out.push(self.data[i0 * ch + c] * (1.0 - t) + self.data[i1 * ch + c] * t);
            }
        }
        let scale = |p: usize| (p as f64 * ratio).round() as usize;
        self.loop_start = scale(self.loop_start);
        self.loop_end = scale(self.loop_end);
        self.slices = self.slices.iter().map(|&p| scale(p)).collect();
        self.set_data(out);
    }

    /// Convert to `rate` Hz, keeping the sound the same (data is resampled, rate field updated).
    pub fn resample_to(&mut self, rate: u32) {
        if rate == 0 || rate == self.sample_rate {
            return;
        }
        let ratio = rate as f64 / self.sample_rate as f64;
        self.resample_by(ratio);
        self.sample_rate = rate;
    }

    /// Tape-style pitch change: the sample plays `semitones` higher (and shorter) at the same base note.
    pub fn pitch_shift(&mut self, semitones: f32) {
        self.resample_by(2f64.powf(-semitones as f64 / 12.0));
    }

    /// Add a slice marker at `frame` (ignored at the very start or end, or on a duplicate).
    pub fn add_slice(&mut self, frame: usize) {
        if frame > 0 && frame < self.frames() && !self.slices.contains(&frame) {
            self.slices.push(frame);
            self.slices.sort_unstable();
        }
    }

    /// Place `parts - 1` evenly spaced slice markers, replacing existing ones.
    pub fn slice_evenly(&mut self, parts: usize) {
        self.slices.clear();
        for i in 1..parts.max(1) {
            self.add_slice(self.frames() * i / parts);
        }
    }

    /// One sample per slice, each mapped to its own key starting at `first_key`.
    pub fn split_at_slices(&self, first_key: u8) -> Vec<Sample> {
        let ch = self.channels.max(1) as usize;
        let mut bounds = vec![0];
        bounds.extend(self.slices.iter().copied());
        bounds.push(self.frames());
        bounds
            .windows(2)
            .enumerate()
            .filter(|(_, w)| w[1] > w[0])
            .map(|(i, w)| {
                let mut s = Sample::new(
                    format!("{} {:02}", self.name, i + 1),
                    self.sample_rate,
                    self.channels,
                    self.data[w[0] * ch..w[1] * ch].to_vec(),
                );
                let key = (first_key as usize + i).min(119) as u8;
                s.base_note = key;
                s.key_range = (key, key);
                s.volume = self.volume;
                s.panning = self.panning;
                s
            })
            .collect()
    }
}

/// How samples are read between their stored frames when played at another pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    /// No interpolation: the nearest stored frame. Gritty and exact, like old samplers and trackers.
    Nearest,
    /// Straight line between neighbours: cheapest, dull and slightly noisy on big pitch shifts.
    Linear,
    /// 4-point Hermite spline: smooth and cheap. The default.
    #[default]
    Cubic,
    /// 8-point windowed sinc: the cleanest, a little more CPU.
    Sinc,
}

/// What an instrument LFO modulates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LfoTarget {
    /// depth = fraction of full volume (0..1)
    #[default]
    Volume,
    /// depth = swing across the stereo field (0..1)
    Pan,
    /// depth = semitones (0..12)
    Pitch,
    /// depth = octaves (0..5); needs an instrument filter
    Cutoff,
    /// depth = octaves of filter resonance (0..3); needs an instrument filter
    Resonance,
    /// depth = added drive (0..1, see `Instrument::drive`)
    Drive,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    Saw,
    Square,
    /// A new random level each cycle, held.
    Random,
    /// A new random level each cycle, glided to.
    SmoothRandom,
    /// Walks through `InstLfo::steps`, one step per `1/n` of a cycle.
    Stepper,
}

/// Steps in a stepper LFO.
pub const MAX_STEPS: usize = 16;

/// A low-frequency oscillator that runs per note.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstLfo {
    pub target: LfoTarget,
    pub shape: LfoShape,
    /// Hz, 0.05..30.
    pub rate: f32,
    pub depth: f32,
    /// Seconds for the LFO to reach full depth after the note starts.
    #[serde(default)]
    pub fade_in: f32,
    /// Levels (-1..1) for `LfoShape::Stepper`; only the first `steps` are used.
    #[serde(default = "default_steps")]
    pub step_values: [f32; MAX_STEPS],
    #[serde(default = "default_step_count")]
    pub steps: u8,
}

fn default_steps() -> [f32; MAX_STEPS] {
    let mut v = [0.0; MAX_STEPS];
    for (i, s) in v.iter_mut().enumerate() {
        *s = if i % 2 == 0 { 1.0 } else { -1.0 };
    }
    v
}

fn default_step_count() -> u8 {
    8
}

impl Default for InstLfo {
    fn default() -> Self {
        Self {
            target: LfoTarget::Volume,
            shape: LfoShape::Sine,
            rate: 4.0,
            depth: 0.3,
            fade_in: 0.0,
            step_values: default_steps(),
            steps: default_step_count(),
        }
    }
}

/// Where a note-level modulation value comes from, besides LFOs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModSource {
    /// The played key: -1 at C-0 .. 0 at C-4 .. +1 at C-8.
    Key,
    /// The note velocity: -1 (silent) .. +1 (full).
    Velocity,
    /// One of the instrument's macro knobs (0-based, `MAX_MACROS`), -1..1 around the middle.
    Macro(u8),
}

/// One routed modulation: a source scaled by `depth` into a target.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstMod {
    pub source: ModSource,
    pub target: LfoTarget,
    pub depth: f32,
}

pub const MAX_INST_MODS: usize = 8;
pub const MAX_MACROS: usize = 4;

/// Most LFOs per instrument.
pub const MAX_INST_LFOS: usize = 4;

/// What happens to a still-sounding note when a new note starts in the same column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Nna {
    /// The old note is cut off immediately.
    #[default]
    Cut,
    /// The old note gets a note-off: it plays out its release.
    NoteOff,
    /// The old note keeps playing to its natural end.
    Continue,
}

fn full_velocity() -> (u8, u8) {
    (0, 127)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    #[default]
    Off,
    LowPass,
    HighPass,
    BandPass,
    Notch,
}

/// Per-instrument resonant filter, applied to every voice of the instrument.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InstrumentFilter {
    pub mode: FilterMode,
    /// Cutoff in Hz.
    pub cutoff: f32,
    /// Resonance as Q (0.707 = flat).
    pub resonance: f32,
}

impl Default for InstrumentFilter {
    fn default() -> Self {
        Self { mode: FilterMode::Off, cutoff: 8000.0, resonance: 0.707 }
    }
}

/// Send an instrument's notes to the MIDI output port instead of (or besides) playing its samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiOut {
    /// 0..=15 (shown to users as 1..=16).
    pub channel: u8,
    /// Program change sent before the first note of a channel, if set.
    pub program: Option<u8>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Instrument {
    pub name: String,
    pub samples: Vec<Sample>,
    /// A hosted instrument plugin. When set, notes of this instrument play the plugin (samples are
    /// kept but not used).
    #[serde(default)]
    pub plugin: Option<crate::track::DeviceSpec>,
    /// Effects every note of this instrument passes through before joining its track (a sample FX chain).
    #[serde(default)]
    pub fx: Vec<crate::track::DeviceSpec>,
    pub volume: f32,
    /// Volume multiplier over note time.
    #[serde(default)]
    pub volume_env: Arc<Envelope>,
    /// Panning over note time (0 = left, 0.5 = center, 1 = right).
    #[serde(default)]
    pub pan_env: Arc<Envelope>,
    /// Pitch over note time in semitones: 0.5 = no change, range +-12.
    #[serde(default)]
    pub pitch_env: Arc<Envelope>,
    #[serde(default)]
    pub filter: InstrumentFilter,
    /// Cutoff modulation over note time: 0.5 = none, range +-5 octaves.
    #[serde(default)]
    pub cutoff_env: Arc<Envelope>,
    #[serde(default)]
    pub nna: Nna,
    #[serde(default)]
    pub interpolation: Interpolation,
    #[serde(default)]
    pub phrases: Vec<Arc<Phrase>>,
    #[serde(default)]
    pub phrase_mode: PhraseMode,
    #[serde(default)]
    pub midi_out: Option<MidiOut>,
    /// Per-note LFOs (at most `MAX_INST_LFOS` are used).
    #[serde(default)]
    pub lfos: Vec<InstLfo>,
    /// How much the filter cutoff follows the played key: 0 = not at all, 1 = one octave per octave (around C-4).
    #[serde(default)]
    pub key_track: f32,
    /// How much a low velocity darkens the note: octaves of cutoff lost at velocity 0 (0..5).
    #[serde(default)]
    pub vel_cutoff: f32,
    /// Key / velocity / macro routings (at most `MAX_INST_MODS` are used).
    #[serde(default)]
    pub mods: Vec<InstMod>,
    /// Macro knob positions 0..1 (pattern commands `M1`..`M4` move them while playing).
    #[serde(default = "default_macros")]
    pub macros: [f32; MAX_MACROS],
    /// Seconds a note takes to fade away after note-off (0 = off).
    #[serde(default)]
    pub fade_out: f32,
    /// Waveshaping drive applied to every voice after the filter (0 = clean, 1 = heavily saturated).
    #[serde(default)]
    pub drive: f32,
}

fn default_macros() -> [f32; MAX_MACROS] {
    [0.5; MAX_MACROS]
}

impl Instrument {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            samples: Vec::new(),
            plugin: None,
            fx: Vec::new(),
            volume: 1.0,
            volume_env: Arc::default(),
            pan_env: Arc::default(),
            pitch_env: Arc::default(),
            filter: InstrumentFilter::default(),
            cutoff_env: Arc::default(),
            nna: Nna::Cut,
            interpolation: Interpolation::Cubic,
            phrases: Vec::new(),
            phrase_mode: PhraseMode::Off,
            midi_out: None,
            lfos: Vec::new(),
            key_track: 0.0,
            vel_cutoff: 0.0,
            mods: Vec::new(),
            macros: default_macros(),
            fade_out: 0.0,
            drive: 0.0,
        }
    }

    /// The phrase a trigger note starts, when phrase playback is on.
    pub fn phrase_for(&self, note: u8) -> Option<usize> {
        if self.phrase_mode != PhraseMode::Keymap {
            return None;
        }
        self.phrases.iter().position(|p| p.covers(note))
    }

    /// First sample whose key range contains `note`.
    /// Sample for a key and velocity (0..=127). A velocity no zone covers falls back to the key match,
    /// so a note never goes silent just because the zones have a gap.
    pub fn sample_for_velocity(&self, note: u8, velocity: u8) -> Option<&Sample> {
        self.samples
            .iter()
            .find(|s| {
                (s.key_range.0..=s.key_range.1).contains(&note)
                    && (s.vel_range.0..=s.vel_range.1).contains(&velocity)
            })
            .or_else(|| self.sample_for(note))
    }

    pub fn sample_for(&self, note: u8) -> Option<&Sample> {
        self.samples.iter().find(|s| (s.key_range.0..=s.key_range.1).contains(&note))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(data: Vec<f32>) -> Sample {
        Sample::new("t", 44_100, 1, data)
    }

    #[test]
    fn normalize_reverse_trim() {
        let mut a = s(vec![0.1, -0.5, 0.25]);
        a.normalize();
        assert_eq!(*a.data, vec![0.2, -1.0, 0.5]);
        a.reverse();
        assert_eq!(*a.data, vec![0.5, -1.0, 0.2]);
        a.trim(1, 3);
        assert_eq!(*a.data, vec![-1.0, 0.2]);
        assert_eq!(a.loop_end, 2);
    }

    #[test]
    fn fades_and_mono_stereo() {
        let mut a = s(vec![1.0; 4]);
        a.fade_in(4);
        assert_eq!(*a.data, vec![0.0, 0.25, 0.5, 0.75]);
        let mut b = s(vec![1.0; 4]);
        b.fade_out(2);
        assert_eq!(*b.data, vec![1.0, 1.0, 1.0, 0.5]);
        let mut c = Sample::new("t", 44_100, 2, vec![1.0, 0.0, 0.5, 0.5]);
        c.to_mono();
        assert_eq!((c.channels, &*c.data), (1, &vec![0.5, 0.5]));
        c.to_stereo();
        assert_eq!((c.channels, &*c.data), (2, &vec![0.5, 0.5, 0.5, 0.5]));
    }

    #[test]
    fn range_ops() {
        let mut a = s(vec![1.0; 6]);
        a.silence(2, 4);
        assert_eq!(*a.data, vec![1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        a.gain(0, 2, 0.5);
        a.invert(4, 6);
        assert_eq!(*a.data, vec![0.5, 0.5, 0.0, 0.0, -1.0, -1.0]);
        a.normalize_range(0, 2);
        assert_eq!(a.data[0], 1.0);
        a.reverse_range(0, 6);
        assert_eq!(*a.data, vec![-1.0, -1.0, 0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn dc_removal() {
        let mut a = s(vec![0.5, 0.7, 0.5, 0.7]);
        a.remove_dc();
        assert!(a.data.iter().sum::<f32>().abs() < 1e-6);
    }

    #[test]
    fn delete_and_insert_keep_markers() {
        let mut a = s((0..10).map(|i| i as f32).collect());
        a.loop_start = 6;
        a.loop_end = 9;
        a.slices = vec![2, 5, 8];
        a.delete(3, 6);
        assert_eq!(*a.data, vec![0.0, 1.0, 2.0, 6.0, 7.0, 8.0, 9.0]);
        assert_eq!((a.loop_start, a.loop_end), (3, 6));
        assert_eq!(a.slices, vec![2, 5]);
        a.insert(3, &[10.0, 11.0]);
        assert_eq!(a.data[3..5], [10.0, 11.0]);
        assert_eq!((a.loop_start, a.loop_end), (5, 8));
        assert_eq!(a.copy_range(3, 5), vec![10.0, 11.0]);
    }

    #[test]
    fn resample_and_pitch() {
        let mut a = s((0..100).map(|i| i as f32 / 100.0).collect());
        a.resample_to(88_200);
        assert_eq!((a.frames(), a.sample_rate), (200, 88_200));
        assert!((a.data[100] - 0.5).abs() < 0.01);
        let mut b = s(vec![0.0; 100]);
        b.pitch_shift(12.0);
        assert_eq!(b.frames(), 50);
        b.pitch_shift(-12.0);
        assert_eq!(b.frames(), 100);
    }

    #[test]
    fn slicing() {
        let mut a = s((0..8).map(|i| i as f32).collect());
        a.slice_evenly(4);
        assert_eq!(a.slices, vec![2, 4, 6]);
        let parts = a.split_at_slices(36);
        assert_eq!(parts.len(), 4);
        assert_eq!(*parts[1].data, vec![2.0, 3.0]);
        assert_eq!((parts[2].base_note, parts[2].key_range), (38, (38, 38)));
        a.add_slice(0);
        a.add_slice(4);
        assert_eq!(a.slices.len(), 3);
    }
}

#[cfg(test)]
mod zone_tests {
    use super::*;

    #[test]
    fn velocity_zones_pick_samples_and_fall_back() {
        let mut inst = Instrument::new("layers");
        let mut soft = Sample::new("soft", 44_100, 1, vec![0.0; 4]);
        soft.vel_range = (0, 63);
        let mut hard = Sample::new("hard", 44_100, 1, vec![0.0; 4]);
        hard.vel_range = (64, 100);
        inst.samples = vec![soft, hard];
        assert_eq!(inst.sample_for_velocity(48, 10).unwrap().name, "soft");
        assert_eq!(inst.sample_for_velocity(48, 90).unwrap().name, "hard");
        assert_eq!(
            inst.sample_for_velocity(48, 127).unwrap().name,
            "soft",
            "gap falls back to the key match"
        );
        assert!(inst.sample_for_velocity(150, 10).is_none());
    }
}
