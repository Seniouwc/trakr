use crate::dsp::{Biquad, Coeffs};
use std::sync::Arc;
use std::sync::OnceLock;
use trakr_core::{Envelope, FilterMode, Instrument, InstrumentFilter, Interpolation, LoopMode, Nna, Sample};

/// How fast a voice's gain and pitch follow their targets (smooths ticks, fades and envelopes).
const SMOOTH_SECONDS: f32 = 0.003;
/// Windowed-sinc taps for 8-point interpolation: `SINC_PHASES` fractional positions of 8 taps each.
const SINC_PHASES: usize = 256;
static SINC_TABLE: OnceLock<Vec<[f32; 8]>> = OnceLock::new();

/// Build the interpolation tables. Must run before audio starts (`create` does) so the audio thread
/// never allocates here.
pub fn init_tables() {
    SINC_TABLE.get_or_init(|| {
        (0..=SINC_PHASES) // one extra phase so the last interval can be interpolated
            .map(|p| {
                let frac = p as f32 / SINC_PHASES as f32;
                let mut taps = [0.0f32; 8];
                for (k, t) in taps.iter_mut().enumerate() {
                    let x = (k as f32 - 3.0) - frac; // distance from the interpolation point
                    let sinc = if x.abs() < 1e-6 {
                        1.0
                    } else {
                        (std::f32::consts::PI * x).sin() / (std::f32::consts::PI * x)
                    };
                    let w = 0.5 + 0.5 * (std::f32::consts::PI * x / 4.0).cos(); // Hann over +-4 taps
                    *t = sinc * w;
                }
                let sum: f32 = taps.iter().sum();
                taps.iter_mut().for_each(|t| *t /= sum); // unity gain at DC for every phase
                taps
            })
            .collect()
    });
}

/// Envelopes, LFOs and tick targets are re-evaluated about every millisecond.
fn control_block(rate: u32) -> u32 {
    (rate / 1000).clamp(1, 64)
}

/// One playing sample. Linear interpolation; forward, reverse and ping-pong loops; per-tick effects
/// (slides, glide, vibrato, tremolo, arpeggio, volume slide, fades) and instrument envelopes.
/// TODO: sinc interpolation, filters, NNA.
pub struct Voice {
    pub track: usize,
    pub column: usize,
    data: Arc<Vec<f32>>,
    channels: usize,
    frames: usize,
    pos: f64,
    /// +1 forward, -1 backward.
    dir: f64,
    sample_rate: f64,
    /// Added to a note number to get semitones relative to the sample's native pitch.
    note_bias: f32,
    /// Current pitch in semitones relative to the sample's native pitch (slides and glide move it).
    semis: f32,
    glide: Option<(f32, f32)>,
    slide: f32,
    vib: (f32, f32),
    vib_phase: f32,
    arp: Option<(f32, f32)>,
    arp_offset: f32,
    trem: (f32, f32),
    trem_phase: f32,
    trem_gain: f32,
    vslide: f32,
    /// Gain from fades and volume slides, 0..=2.
    dyn_gain: f32,
    fade: Option<(f32, f32)>,
    /// Gain from the volume column (and the initial note volume).
    col_vol: f32,
    base_gain: f32,
    pan: f32,
    loop_mode: LoopMode,
    loop_start: usize,
    loop_end: usize,
    finished: bool,
    releasing: bool,
    /// 5 ms click-free ending; starts once the voice is released and its volume envelope is done.
    ending: bool,
    end_fade: f32,
    age_frames: u64,
    released_at: Option<f32>,
    vol_env: Arc<Envelope>,
    pan_env: Arc<Envelope>,
    pitch_env: Arc<Envelope>,
    cutoff_env: Arc<Envelope>,
    filter: InstrumentFilter,
    filter_coeffs: Coeffs,
    filter_state: [Biquad; 2],
    gm_cur: f32,
    gm_target: f32,
    step_cur: f64,
    step_target: f64,
    pan_now: f32,
    ctr: u32,
    /// Jump straight to the targets on the first control update instead of smoothing from zero.
    snap: bool,
    nna: Nna,
    interp: Interpolation,
    /// An effect command was applied on the current line; its slides and fades start on tick 1.
    fresh_fx: bool,
}

/// Per-trigger options from effect commands.
#[derive(Clone, Copy, Default)]
pub struct TriggerOpts {
    /// Start offset in 1/256 of the sample length.
    pub offset: u8,
    pub backwards: bool,
}

fn nibbles(amount: u8) -> (f32, f32) {
    ((amount >> 4) as f32, (amount & 15) as f32)
}

impl Voice {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track: usize,
        column: usize,
        inst: &Instrument,
        s: &Sample,
        note: u8,
        vol: f32,
        pan: f32,
        opts: TriggerOpts,
    ) -> Self {
        let bias = -(s.base_note as f32) + s.fine_tune as f32 / 128.0;
        let frames = s.frames();
        let loop_end = s.loop_end.min(frames);
        let has_loop = s.loop_mode != LoopMode::Off && loop_end > s.loop_start;
        let reverse = opts.backwards || (s.loop_mode == LoopMode::Reverse && has_loop);
        let mut pos = frames as f64 * opts.offset as f64 / 256.0;
        if reverse {
            pos = if has_loop && s.loop_mode == LoopMode::Reverse {
                loop_end as f64 - 1.0
            } else {
                frames as f64 - 1.0
            };
        }
        Self {
            track,
            column,
            data: s.data.clone(),
            channels: s.channels as usize,
            frames,
            pos: pos.max(0.0),
            dir: if reverse { -1.0 } else { 1.0 },
            sample_rate: s.sample_rate as f64,
            note_bias: bias,
            semis: note as f32 + bias,
            glide: None,
            slide: 0.0,
            vib: (0.0, 0.0),
            vib_phase: 0.0,
            arp: None,
            arp_offset: 0.0,
            trem: (0.0, 0.0),
            trem_phase: 0.0,
            trem_gain: 1.0,
            vslide: 0.0,
            dyn_gain: 1.0,
            fade: None,
            col_vol: vol,
            base_gain: s.volume * inst.volume,
            pan: (pan + s.panning).clamp(-1.0, 1.0),
            loop_mode: s.loop_mode,
            loop_start: s.loop_start,
            loop_end,
            finished: frames == 0,
            releasing: false,
            ending: false,
            end_fade: 1.0,
            age_frames: 0,
            released_at: None,
            vol_env: inst.volume_env.clone(),
            pan_env: inst.pan_env.clone(),
            pitch_env: inst.pitch_env.clone(),
            cutoff_env: inst.cutoff_env.clone(),
            filter: inst.filter,
            filter_coeffs: Coeffs::PASS,
            filter_state: [Biquad::default(); 2],
            gm_cur: 0.0,
            gm_target: 0.0,
            step_cur: 0.0,
            step_target: 0.0,
            pan_now: 0.0,
            ctr: 0,
            snap: true,
            nna: inst.nna,
            interp: inst.interpolation,
            fresh_fx: false,
        }
    }

    pub fn release(&mut self) {
        if self.releasing {
            return;
        }
        self.releasing = true;
        self.released_at = Some(self.age_frames as f32);
        if !self.vol_env.enabled {
            self.ending = true;
        }
    }

    /// Column marker for voices that no longer answer to their original column (see NNA).
    pub const DETACHED: usize = usize::MAX - 1;

    pub fn nna(&self) -> Nna {
        self.nna
    }

    /// Stop immediately (with the short click-free fade), ignoring any release envelope.
    pub fn cut(&mut self) {
        self.releasing = true;
        self.ending = true;
    }

    /// Let this voice play on by itself: later notes and effects in its column no longer touch it.
    pub fn detach(&mut self) {
        self.column = Self::DETACHED;
    }

    pub fn is_releasing(&self) -> bool {
        self.releasing
    }

    pub fn alive(&self) -> bool {
        !self.finished
    }

    /// Pitch (in semitones relative to native) that `note` would have on this voice's sample.
    pub fn semis_for_note(&self, note: u8) -> f32 {
        note as f32 + self.note_bias
    }

    pub fn set_col_volume(&mut self, v: f32) {
        self.col_vol = v;
    }

    pub fn set_pan(&mut self, p: f32) {
        self.pan = p.clamp(-1.0, 1.0);
    }

    /// Slide toward `target` semitones at `speed`/16 semitone per tick (0 jumps there).
    pub fn glide_to(&mut self, target: f32, speed: u8) {
        if speed == 0 {
            self.semis = target;
            self.glide = None;
        } else {
            self.glide = Some((target, speed as f32 / 16.0));
        }
    }

    /// Start or change a per-tick effect on this voice. Unknown ids are ignored.
    pub fn apply_effect(&mut self, id: [u8; 2], amount: u8) {
        let (x, y) = nibbles(amount);
        self.fresh_fx = true;
        match &id {
            b"0U" => self.slide = amount as f32 / 16.0,
            b"0D" => self.slide = -(amount as f32) / 16.0,
            b"0V" => self.vib = (x, y / 8.0),
            b"0T" => {
                self.trem = (x, y / 15.0);
                if amount == 0 {
                    self.trem_gain = 1.0;
                }
            }
            b"0A" => {
                self.arp = (amount != 0).then_some((x, y));
                if amount == 0 {
                    self.arp_offset = 0.0;
                }
            }
            b"0W" => self.vslide = (x - y) / 64.0,
            b"0I" if amount > 0 => {
                self.dyn_gain = 0.0;
                self.fade = Some((1.0, 1.0 / amount as f32));
            }
            b"0O" if amount > 0 => self.fade = Some((0.0, 1.0 / amount as f32)),
            _ => {}
        }
    }

    /// Advance per-tick effects. `tick` is the tick index within the line (slides and fades start on tick 1 of the line that issues them).
    pub fn tick(&mut self, tick: u8) {
        let skip = tick == 0 && self.fresh_fx;
        self.fresh_fx = false;
        if !skip {
            self.semis += self.slide;
            if let Some((target, speed)) = self.glide {
                let d = target - self.semis;
                if d.abs() <= speed {
                    self.semis = target;
                    self.glide = None;
                } else {
                    self.semis += speed * d.signum();
                }
            }
            self.dyn_gain = (self.dyn_gain + self.vslide).clamp(0.0, 2.0);
        }
        self.arp_offset = match self.arp {
            Some((x, y)) => match tick % 3 {
                1 => x,
                2 => y,
                _ => 0.0,
            },
            None => 0.0,
        };
        if let (Some((target, step)), false) = (self.fade, skip) {
            if self.dyn_gain < target {
                self.dyn_gain = (self.dyn_gain + step).min(target);
            } else {
                self.dyn_gain = (self.dyn_gain - step).max(target);
            }
            if self.dyn_gain == target {
                self.fade = None;
                if target == 0.0 {
                    self.release();
                    self.ending = true;
                }
            }
        }
    }

    fn age_secs(&self, rate: u32) -> f32 {
        self.age_frames as f32 / rate as f32
    }

    /// Recompute smoothing targets from effects and envelopes.
    fn retarget(&mut self, rate: u32) {
        let age = self.age_secs(rate);
        let rel = self.released_at.map(|f| f / rate as f32);
        // LFOs run in real time (speed nibble x = x * 0.5 Hz), independent of tempo.
        let dt = control_block(rate) as f32 / rate as f32;
        self.vib_phase = (self.vib_phase + self.vib.0 * 0.5 * dt).fract();
        self.trem_phase = (self.trem_phase + self.trem.0 * 0.5 * dt).fract();
        self.trem_gain = 1.0 - self.trem.1 * (0.5 + 0.5 * (self.trem_phase * std::f32::consts::TAU).sin());
        let mut semis = self.semis + self.arp_offset;
        if self.vib.1 > 0.0 {
            semis += self.vib.1 * (self.vib_phase * std::f32::consts::TAU).sin();
        }
        if self.pitch_env.enabled {
            let t = self.pitch_env.effective_time(age, rel);
            semis += (self.pitch_env.value_at(t) - 0.5) * 24.0;
        }
        self.step_target = self.sample_rate * 2f64.powf(semis as f64 / 12.0) / rate as f64;
        let mut gain = self.col_vol * self.dyn_gain * self.trem_gain * self.base_gain;
        if self.vol_env.enabled {
            let t = self.vol_env.effective_time(age, rel);
            gain *= self.vol_env.value_at(t);
            if rel.is_some() && t >= self.vol_env.end_time() {
                self.ending = true;
            }
        }
        self.gm_target = gain;
        let mut pan = self.pan;
        if self.pan_env.enabled {
            let t = self.pan_env.effective_time(age, rel);
            pan += (self.pan_env.value_at(t) - 0.5) * 2.0;
        }
        self.pan_now = pan.clamp(-1.0, 1.0);

        if self.filter.mode != FilterMode::Off {
            let mut cutoff = self.filter.cutoff;
            if self.cutoff_env.enabled {
                let t = self.cutoff_env.effective_time(age, rel);
                cutoff *= 2f32.powf((self.cutoff_env.value_at(t) - 0.5) * 10.0);
                // +-5 octaves
            }
            let (fs, q) = (rate as f32, self.filter.resonance);
            self.filter_coeffs = match self.filter.mode {
                FilterMode::HighPass => Coeffs::highpass(fs, cutoff, q),
                FilterMode::BandPass => Coeffs::bandpass(fs, cutoff, q),
                FilterMode::Notch => Coeffs::notch(fs, cutoff, q),
                _ => Coeffs::lowpass(fs, cutoff, q),
            };
        }
    }

    #[inline]
    fn frame(&self, i: usize, ch: usize) -> f32 {
        self.data.get(i * self.channels + ch.min(self.channels - 1)).copied().unwrap_or(0.0)
    }

    /// Like `frame` but clamps the index into the sample (for the neighbours interpolation looks at).
    #[inline]
    fn at(&self, i: isize, ch: usize) -> f32 {
        self.frame(i.clamp(0, self.frames as isize - 1) as usize, ch)
    }

    /// Sample value at fractional position `i + frac` on channel `ch`.
    #[inline]
    fn sample_at(&self, i: usize, frac: f32, ch: usize) -> f32 {
        let i = i as isize;
        match self.interp {
            Interpolation::Linear => self.at(i, ch) * (1.0 - frac) + self.at(i + 1, ch) * frac,
            Interpolation::Cubic => {
                let (p0, p1, p2, p3) =
                    (self.at(i - 1, ch), self.at(i, ch), self.at(i + 1, ch), self.at(i + 2, ch));
                let c1 = 0.5 * (p2 - p0);
                let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
                let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
                ((c3 * frac + c2) * frac + c1) * frac + p1
            }
            Interpolation::Sinc => match SINC_TABLE.get() {
                Some(table) => {
                    // Interpolate between the two nearest tabulated phases so the table's resolution
                    // does not become a timing error of its own.
                    let pos = frac * SINC_PHASES as f32;
                    let p0 = (pos as usize).min(SINC_PHASES - 1);
                    let w = pos - p0 as f32;
                    let (a, b) = (&table[p0], &table[p0 + 1]);
                    (0..8).map(|k| self.at(i + k as isize - 3, ch) * (a[k] * (1.0 - w) + b[k] * w)).sum()
                }
                None => self.at(i, ch) * (1.0 - frac) + self.at(i + 1, ch) * frac,
            },
        }
    }

    /// Add this voice into interleaved stereo `buf`; returns the peak it contributed.
    pub fn mix(&mut self, buf: &mut [f32], engine_rate: u32, track_vol: f32, track_pan: f32) -> f32 {
        let k = 1.0 - (-1.0 / (engine_rate as f32 * SMOOTH_SECONDS)).exp();
        let k64 = k as f64;
        let end_step = 1.0 / (engine_rate as f32 * 0.005); // 5 ms release
        let looping = self.loop_mode != LoopMode::Off && self.loop_end > self.loop_start;
        let len = (self.loop_end - self.loop_start.min(self.loop_end)) as f64;
        // Without a volume envelope, a released voice plays out past its loop; with one, it keeps looping.
        let stop_loop_on_release = !self.vol_env.enabled;
        let block = control_block(engine_rate);
        let mut peak = 0.0f32;
        for out in buf.chunks_exact_mut(2) {
            if self.finished {
                return peak;
            }
            if self.ctr == 0 {
                self.retarget(engine_rate);
                if self.snap {
                    self.snap = false;
                    self.gm_cur = self.gm_target;
                    self.step_cur = self.step_target;
                }
            }
            self.ctr = (self.ctr + 1) % block;
            self.age_frames += 1;
            self.gm_cur += (self.gm_target - self.gm_cur) * k;
            self.step_cur += (self.step_target - self.step_cur) * k64;

            if self.pos < 0.0 || self.pos as usize >= self.frames {
                self.finished = true;
                return peak;
            }
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            let l = self.sample_at(i, frac, 0);
            let r = if self.channels > 1 { self.sample_at(i, frac, 1) } else { l };
            let (l, r) = if self.filter.mode != FilterMode::Off {
                (
                    self.filter_state[0].process(&self.filter_coeffs, l),
                    self.filter_state[1].process(&self.filter_coeffs, r),
                )
            } else {
                (l, r)
            };
            if self.ending {
                self.end_fade -= end_step;
                if self.end_fade <= 0.0 {
                    self.finished = true;
                    return peak;
                }
            }
            let pan = (self.pan_now + track_pan).clamp(-1.0, 1.0);
            let (gl, gr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
            let a = self.gm_cur * track_vol * self.end_fade;
            let (ol, or) = (l * a * gl, r * a * gr);
            out[0] += ol;
            out[1] += or;
            peak = peak.max(ol.abs()).max(or.abs());

            self.pos += self.step_cur * self.dir;
            if looping && !(self.releasing && stop_loop_on_release) {
                let (ls, le) = (self.loop_start as f64, self.loop_end as f64);
                match self.loop_mode {
                    LoopMode::Forward if self.pos >= le => self.pos -= len,
                    LoopMode::Reverse if self.pos < ls => self.pos += len,
                    LoopMode::PingPong => {
                        if self.dir > 0.0 && self.pos >= le {
                            self.pos = (2.0 * le - self.pos).min(le - 1e-3);
                            self.dir = -1.0;
                        } else if self.dir < 0.0 && self.pos < ls {
                            self.pos = 2.0 * ls - self.pos;
                            self.dir = 1.0;
                        }
                    }
                    _ => {}
                }
            }
        }
        peak
    }
}
