//! Native track devices. Each owns its state (allocated when built on the UI thread) and processes
//! interleaved stereo blocks in place without allocating. Parameters arrive in catalogue order.

use crate::dsp::{db_to_gain, gain_to_db, sine_phase, smooth_coeff, Biquad, Coeffs, DelayLine};

pub trait Device: Send {
    /// Catalogue id this instance implements (empty for pass-through placeholders).
    fn id(&self) -> &'static str;

    /// Process `buf` (interleaved stereo) in place with the current parameter values.
    fn process(&mut self, params: &[f32], buf: &mut [f32]);

    /// For modulators: the control value (-1..=1) produced by the last `process` call. The engine
    /// adds it to the target parameter's normalised position.
    fn modulation(&self) -> Option<f32> {
        None
    }

    /// Like `process`, with another track's signal as a side input (sidechain). Devices that do not
    /// use one ignore it.
    fn process_sc(&mut self, params: &[f32], buf: &mut [f32], _sidechain: Option<&[f32]>) {
        self.process(params, buf);
    }

    /// Called before each block so tempo-synced devices know the song tempo.
    fn set_tempo(&mut self, _bpm: f32) {}

    /// Called before each block with the last note played on this track and its velocity (0..1).
    fn set_track_note(&mut self, _note: u8, _velocity: f32) {}

    /// Instrument plugins: start a note (velocity 0..1). Effects ignore it.
    fn note_on(&mut self, _key: u8, _velocity: f32) {}

    fn note_off(&mut self, _key: u8) {}

    /// True for devices that generate sound from notes; `process` then fills the buffer instead of
    /// filtering it.
    fn is_instrument(&self) -> bool {
        false
    }
}

/// Build a device for a catalogue id at the engine's sample rate.
pub fn create(id: &str, rate: u32) -> Option<Box<dyn Device>> {
    let fs = rate as f32;
    Some(match id {
        "native.gainer" => Box::new(Gainer { gain: 1.0, fs, primed: false }),
        "native.filter" => Box::new(FilterDev { l: Biquad::default(), r: Biquad::default(), fs }),
        "native.eq" => Box::new(Eq { bands: [[Biquad::default(); 2]; 3], fs }),
        "native.delay" => Box::new(Delay::new(fs)),
        "native.reverb" => Box::new(Reverb::new(fs)),
        "native.chorus" => Box::new(Chorus::new(fs)),
        "native.distortion" => Box::new(Distortion { gain: 1.0, fs, primed: false }),
        "native.compressor" => Box::new(Compressor { env_db: -90.0, fs }),
        "native.eq10" => Box::new(Eq10 { bands: [[Biquad::default(); 2]; 10], fs }),
        "native.ladder" => Box::new(Ladder { stage: [[0.0; 4]; 2], fs }),
        "native.gate" => Box::new(Gate { env_db: -120.0, gain: 1.0, fs }),
        "native.limiter" => Box::new(Limiter::new(fs)),
        "native.phaser" => Box::new(Phaser { stages: [[0.0; 4]; 2], fb: [0.0; 2], phase: 0.0, fs }),
        "native.ringmod" => Box::new(RingMod { phase: 0.0, fs }),
        "native.bitcrusher" => Box::new(BitCrusher { held: [0.0; 2], count: 0.0 }),
        "native.width" => Box::new(Width),
        "native.autopan" => Box::new(AutoPan { phase: 0.0, fs }),
        "native.lfo" => Box::new(Lfo { phase: 0.0, held: 0.0, rng: 0x9E37_79B9, value: 0.0, fs, bpm: 120.0 }),
        "native.follower" => Box::new(Follower { env: 0.0, value: 0.0, fs }),
        "native.exciter" => Box::new(Exciter { hp: [Biquad::default(); 2], fs }),
        _ => return crate::devices_more::create(id, fs).or_else(|| crate::devices_adv::create(id, fs)),
    })
}

/// Like [`create`], but unknown ids become a pass-through so chain indices always match the song's device list.
pub fn create_or_passthrough(id: &str, rate: u32) -> Box<dyn Device> {
    create(id, rate).unwrap_or_else(|| Box::new(Passthrough))
}

struct Passthrough;

impl Device for Passthrough {
    fn id(&self) -> &'static str {
        ""
    }

    fn process(&mut self, _params: &[f32], _buf: &mut [f32]) {}
}

fn p(params: &[f32], i: usize, default: f32) -> f32 {
    params.get(i).copied().unwrap_or(default)
}

// ---- gainer -----------------------------------------------------------------------------

struct Gainer {
    gain: f32,
    fs: f32,
    primed: bool,
}

impl Device for Gainer {
    fn id(&self) -> &'static str {
        "native.gainer"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let target = db_to_gain(p(params, 0, 0.0));
        let target = if p(params, 0, 0.0) <= -59.9 { 0.0 } else { target };
        let pan = p(params, 1, 0.0).clamp(-1.0, 1.0);
        let (pl, pr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
        if !self.primed {
            self.gain = target; // start at the set gain instead of gliding in from unity
            self.primed = true;
        }
        let k = smooth_coeff(self.fs, 0.005);
        for f in buf.chunks_exact_mut(2) {
            self.gain += (target - self.gain) * k;
            f[0] *= self.gain * pl;
            f[1] *= self.gain * pr;
        }
    }
}

// ---- filter -------------------------------------------------------------------------------

struct FilterDev {
    l: Biquad,
    r: Biquad,
    fs: f32,
}

impl Device for FilterDev {
    fn id(&self) -> &'static str {
        "native.filter"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (f, q) = (p(params, 1, 8000.0), p(params, 2, 0.707));
        let c = match p(params, 0, 0.0).round() as i32 {
            1 => Coeffs::highpass(self.fs, f, q),
            2 => Coeffs::bandpass(self.fs, f, q),
            3 => Coeffs::notch(self.fs, f, q),
            _ => Coeffs::lowpass(self.fs, f, q),
        };
        for fr in buf.chunks_exact_mut(2) {
            fr[0] = self.l.process(&c, fr[0]);
            fr[1] = self.r.process(&c, fr[1]);
        }
    }
}

// ---- EQ -----------------------------------------------------------------------------------

struct Eq {
    bands: [[Biquad; 2]; 3],
    fs: f32,
}

impl Device for Eq {
    fn id(&self) -> &'static str {
        "native.eq"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let cs = [
            Coeffs::low_shelf(self.fs, p(params, 1, 100.0), p(params, 0, 0.0)),
            Coeffs::peak(self.fs, p(params, 3, 1000.0), p(params, 4, 1.0), p(params, 2, 0.0)),
            Coeffs::high_shelf(self.fs, p(params, 6, 6000.0), p(params, 5, 0.0)),
        ];
        for fr in buf.chunks_exact_mut(2) {
            for (band, c) in self.bands.iter_mut().zip(&cs) {
                fr[0] = band[0].process(c, fr[0]);
                fr[1] = band[1].process(c, fr[1]);
            }
        }
    }
}

// ---- delay --------------------------------------------------------------------------------

struct Delay {
    l: DelayLine,
    r: DelayLine,
    time: f32,
    fs: f32,
    primed: bool,
}

impl Delay {
    fn new(fs: f32) -> Self {
        let n = (fs * 2.05) as usize;
        Self { l: DelayLine::new(n), r: DelayLine::new(n), time: fs * 0.3, fs, primed: false }
    }
}

impl Device for Delay {
    fn id(&self) -> &'static str {
        "native.delay"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let target = (p(params, 0, 300.0) * 0.001 * self.fs).clamp(1.0, (self.l.len() - 3) as f32);
        let (fb, mix, pp) = (p(params, 1, 0.4), p(params, 2, 0.3), p(params, 3, 0.0));
        if !self.primed {
            self.time = target; // start at the set time instead of gliding in from a default
            self.primed = true;
        }
        let k = smooth_coeff(self.fs, 0.05); // glide the time so changes don't click
        for fr in buf.chunks_exact_mut(2) {
            self.time += (target - self.time) * k;
            let (dl, dr) = (self.l.read(self.time), self.r.read(self.time));
            // Ping-pong crosses the feedback between channels.
            let (fl, fr_) = (dl * (1.0 - pp) + dr * pp, dr * (1.0 - pp) + dl * pp);
            self.l.write(fr[0] + fl * fb);
            self.r.write(fr[1] + fr_ * fb);
            fr[0] = fr[0] * (1.0 - mix) + dl * mix;
            fr[1] = fr[1] * (1.0 - mix) + dr * mix;
        }
    }
}

// ---- reverb (Freeverb topology) -----------------------------------------------------------

struct Comb {
    line: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(n: usize) -> Self {
        Self { line: vec![0.0; n.max(2)], pos: 0, store: 0.0 }
    }

    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.line[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.line[self.pos] = x + self.store * feedback;
        self.pos = (self.pos + 1) % self.line.len();
        out
    }
}

struct Allpass {
    line: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(n: usize) -> Self {
        Self { line: vec![0.0; n.max(2)], pos: 0 }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let buffered = self.line[self.pos];
        let out = buffered - x;
        self.line[self.pos] = x + buffered * 0.5;
        self.pos = (self.pos + 1) % self.line.len();
        out
    }
}

struct Reverb {
    combs: [Vec<Comb>; 2],
    allpass: [Vec<Allpass>; 2],
}

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

impl Reverb {
    fn new(fs: f32) -> Self {
        let scale = fs / 44_100.0;
        let sz = |n: usize, extra: usize| (((n + extra) as f32) * scale) as usize;
        let make = |extra: usize| {
            (
                COMB_TUNING.iter().map(|&n| Comb::new(sz(n, extra))).collect::<Vec<_>>(),
                ALLPASS_TUNING.iter().map(|&n| Allpass::new(sz(n, extra))).collect::<Vec<_>>(),
            )
        };
        let (cl, al) = make(0);
        let (cr, ar) = make(STEREO_SPREAD);
        Self { combs: [cl, cr], allpass: [al, ar] }
    }
}

impl Device for Reverb {
    fn id(&self) -> &'static str {
        "native.reverb"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let feedback = 0.7 + 0.28 * p(params, 0, 0.6).clamp(0.0, 1.0);
        let damp = 0.4 * p(params, 1, 0.4).clamp(0.0, 1.0);
        let width = p(params, 2, 1.0).clamp(0.0, 1.0);
        let mix = p(params, 3, 0.25).clamp(0.0, 1.0);
        let (wet1, wet2) = (mix * (width * 0.5 + 0.5), mix * ((1.0 - width) * 0.5));
        let dry = 1.0 - mix;
        for fr in buf.chunks_exact_mut(2) {
            let input = (fr[0] + fr[1]) * 0.015;
            let mut out = [0.0f32; 2];
            for (o, (combs, allpass)) in
                out.iter_mut().zip(self.combs.iter_mut().zip(self.allpass.iter_mut()))
            {
                let mut acc = 0.0;
                for c in combs.iter_mut() {
                    acc += c.tick(input, feedback, damp);
                }
                for a in allpass.iter_mut() {
                    acc = a.tick(acc);
                }
                *o = acc;
            }
            // Freeverb's output scaling.
            let (wl, wr) = (out[0] * 3.0, out[1] * 3.0);
            fr[0] = fr[0] * dry + wl * wet1 + wr * wet2;
            fr[1] = fr[1] * dry + wr * wet1 + wl * wet2;
        }
    }
}

// ---- chorus / flanger ---------------------------------------------------------------------

struct Chorus {
    l: DelayLine,
    r: DelayLine,
    phase: f32,
    fs: f32,
    fb: [f32; 2],
}

impl Chorus {
    fn new(fs: f32) -> Self {
        let n = (fs * 0.05) as usize + 4;
        Self { l: DelayLine::new(n), r: DelayLine::new(n), phase: 0.0, fs, fb: [0.0; 2] }
    }
}

impl Device for Chorus {
    fn id(&self) -> &'static str {
        "native.chorus"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (rate, depth, mix, fb) =
            (p(params, 0, 0.8), p(params, 1, 3.0), p(params, 2, 0.5), p(params, 3, 0.0));
        let ms = 0.001 * self.fs;
        let base = 5.0 * ms;
        let inc = rate / self.fs;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + inc).fract();
            let (ml, mr) = (0.5 + 0.5 * sine_phase(self.phase), 0.5 + 0.5 * sine_phase(self.phase + 0.25));
            self.l.write(fr[0] + self.fb[0] * fb);
            self.r.write(fr[1] + self.fb[1] * fb);
            let wl = self.l.read(base + depth * ms * ml);
            let wr = self.r.read(base + depth * ms * mr);
            self.fb = [wl, wr];
            fr[0] = fr[0] * (1.0 - mix) + wl * mix;
            fr[1] = fr[1] * (1.0 - mix) + wr * mix;
        }
    }
}

// ---- distortion ---------------------------------------------------------------------------

struct Distortion {
    gain: f32,
    fs: f32,
    primed: bool,
}

impl Device for Distortion {
    fn id(&self) -> &'static str {
        "native.distortion"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let drive = db_to_gain(p(params, 0, 12.0));
        let (mix, out) = (p(params, 1, 1.0), db_to_gain(p(params, 2, -6.0)));
        if !self.primed {
            self.gain = drive;
            self.primed = true;
        }
        let k = smooth_coeff(self.fs, 0.01);
        for fr in buf.chunks_exact_mut(2) {
            self.gain += (drive - self.gain) * k;
            for s in fr.iter_mut() {
                let wet = (*s * self.gain).tanh() * out;
                *s = *s * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---- compressor ---------------------------------------------------------------------------

struct Compressor {
    /// Smoothed detector level in dB.
    env_db: f32,
    fs: f32,
}

impl Device for Compressor {
    fn id(&self) -> &'static str {
        "native.compressor"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (thr, ratio) = (p(params, 0, -18.0), p(params, 1, 4.0).max(1.0));
        let atk = smooth_coeff(self.fs, p(params, 2, 10.0) * 0.001);
        let rel = smooth_coeff(self.fs, p(params, 3, 120.0) * 0.001);
        let makeup = db_to_gain(p(params, 4, 0.0));
        for fr in buf.chunks_exact_mut(2) {
            let level = gain_to_db(fr[0].abs().max(fr[1].abs()));
            let k = if level > self.env_db { atk } else { rel };
            self.env_db += (level - self.env_db) * k;
            let over = (self.env_db - thr).max(0.0);
            let g = db_to_gain(-over * (1.0 - 1.0 / ratio)) * makeup;
            fr[0] *= g;
            fr[1] *= g;
        }
    }
}

// ---- 10-band EQ ----------------------------------------------------------------------------

const EQ10_FREQS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16_000.0];

struct Eq10 {
    bands: [[Biquad; 2]; 10],
    fs: f32,
}

impl Device for Eq10 {
    fn id(&self) -> &'static str {
        "native.eq10"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let mut cs = [Coeffs::PASS; 10];
        let mut active = [false; 10];
        for (i, c) in cs.iter_mut().enumerate() {
            let g = p(params, i, 0.0);
            if g.abs() > 0.01 {
                *c = Coeffs::peak(self.fs, EQ10_FREQS[i], 1.4, g);
                active[i] = true;
            }
        }
        for fr in buf.chunks_exact_mut(2) {
            for i in 0..10 {
                if active[i] {
                    fr[0] = self.bands[i][0].process(&cs[i], fr[0]);
                    fr[1] = self.bands[i][1].process(&cs[i], fr[1]);
                }
            }
        }
    }
}

// ---- 24 dB ladder filter --------------------------------------------------------------------

struct Ladder {
    stage: [[f32; 4]; 2],
    fs: f32,
}

impl Device for Ladder {
    fn id(&self) -> &'static str {
        "native.ladder"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let cutoff = p(params, 0, 2000.0).clamp(20.0, self.fs * 0.45);
        let k = p(params, 1, 0.3).clamp(0.0, 1.0) * 3.9; // 4.0 would self-oscillate
        let drive = db_to_gain(p(params, 2, 0.0));
        let g = 1.0 - (-std::f32::consts::TAU * cutoff / self.fs).exp();
        for fr in buf.chunks_exact_mut(2) {
            for (ch, s) in self.stage.iter_mut().enumerate() {
                let x = (fr[ch] * drive - k * s[3]).tanh();
                s[0] += g * (x - s[0]);
                s[1] += g * (s[0] - s[1]);
                s[2] += g * (s[1] - s[2]);
                s[3] += g * (s[2] - s[3]);
                // Make-up for the passband loss the feedback causes, and undo the drive boost.
                fr[ch] = s[3] * (1.0 + k * 0.25) / drive.max(1.0).sqrt();
            }
        }
    }
}

// ---- gate -----------------------------------------------------------------------------------

struct Gate {
    env_db: f32,
    gain: f32,
    fs: f32,
}

impl Device for Gate {
    fn id(&self) -> &'static str {
        "native.gate"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let thr = p(params, 0, -50.0);
        let open = smooth_coeff(self.fs, p(params, 1, 1.0) * 0.001);
        let close = smooth_coeff(self.fs, p(params, 2, 100.0) * 0.001);
        let floor = db_to_gain(-p(params, 3, 60.0));
        let det = smooth_coeff(self.fs, 0.002);
        for fr in buf.chunks_exact_mut(2) {
            let level = gain_to_db(fr[0].abs().max(fr[1].abs()));
            let k = if level > self.env_db { 1.0 } else { det };
            self.env_db += (level - self.env_db) * k;
            let target = if self.env_db > thr { 1.0 } else { floor };
            let k = if target > self.gain { open } else { close };
            self.gain += (target - self.gain) * k;
            fr[0] *= self.gain;
            fr[1] *= self.gain;
        }
    }
}

// ---- lookahead limiter ----------------------------------------------------------------------

pub(crate) struct Limiter {
    l: DelayLine,
    r: DelayLine,
    /// Gain each of the last `look + 1` input samples would need, newest at `pos`.
    needs: Vec<f32>,
    pos: usize,
    look: usize,
    gain: f32,
    fs: f32,
}

impl Limiter {
    pub(crate) fn new(fs: f32) -> Self {
        let look = ((fs * 0.002) as usize).max(8);
        Self {
            l: DelayLine::new(look + 4),
            r: DelayLine::new(look + 4),
            needs: vec![1.0; look + 1],
            pos: 0,
            look,
            gain: 1.0,
            fs,
        }
    }
}

impl Device for Limiter {
    fn id(&self) -> &'static str {
        "native.limiter"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let pre = db_to_gain(p(params, 0, 0.0));
        let ceiling = db_to_gain(p(params, 1, -1.0));
        let rel = 1.0 / (self.fs * p(params, 2, 80.0) * 0.001).max(1.0);
        let slope = 1.0 / self.look as f32; // a full-scale drop takes exactly the lookahead time
        for fr in buf.chunks_exact_mut(2) {
            let (a, b) = (fr[0] * pre, fr[1] * pre);
            let peak = a.abs().max(b.abs());
            self.pos = (self.pos + 1) % self.needs.len();
            self.needs[self.pos] = if peak > ceiling { ceiling / peak } else { 1.0 };
            // The gain must already be low enough when the loudest sample of the lookahead window
            // reaches the output, so aim for the smallest need in the window and ramp down to it.
            let target = self.needs.iter().fold(1.0f32, |m, &n| m.min(n));
            if target < self.gain {
                self.gain = (self.gain - slope).max(target);
            } else {
                self.gain = (self.gain + rel).min(target);
            }
            self.l.write(a);
            self.r.write(b);
            fr[0] = self.l.read(self.look as f32) * self.gain;
            fr[1] = self.r.read(self.look as f32) * self.gain;
        }
    }
}

// ---- phaser ---------------------------------------------------------------------------------

struct Phaser {
    /// Per channel: four first-order all-pass states.
    stages: [[f32; 4]; 2],
    fb: [f32; 2],
    phase: f32,
    fs: f32,
}

impl Device for Phaser {
    fn id(&self) -> &'static str {
        "native.phaser"
    }

    #[allow(clippy::needless_range_loop)] // index is shared by several per-channel arrays
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (rate, depth, fb, mix) =
            (p(params, 0, 0.4), p(params, 1, 0.7), p(params, 2, 0.5), p(params, 3, 0.5));
        let inc = rate / self.fs;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + inc).fract();
            // Sweep the notch frequencies exponentially between 200 Hz and 3.2 kHz.
            let lfo = 0.5 + 0.5 * sine_phase(self.phase);
            let f = 200.0 * 16f32.powf(lfo * depth);
            let t = (std::f32::consts::PI * f / self.fs).tan();
            let a = (t - 1.0) / (t + 1.0);
            for ch in 0..2 {
                let dry = fr[ch];
                let mut x = dry + self.fb[ch] * fb;
                for z in &mut self.stages[ch] {
                    let y = a * x + *z;
                    *z = x - a * y;
                    x = y;
                }
                self.fb[ch] = x;
                fr[ch] = dry * (1.0 - mix * 0.5) + x * mix * 0.5;
            }
        }
    }
}

// ---- ring modulator -------------------------------------------------------------------------

struct RingMod {
    phase: f32,
    fs: f32,
}

impl Device for RingMod {
    fn id(&self) -> &'static str {
        "native.ringmod"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (freq, mix) = (p(params, 0, 440.0), p(params, 1, 1.0));
        let inc = freq / self.fs;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + inc).fract();
            let m = sine_phase(self.phase);
            fr[0] = fr[0] * (1.0 - mix) + fr[0] * m * mix;
            fr[1] = fr[1] * (1.0 - mix) + fr[1] * m * mix;
        }
    }
}

// ---- bit crusher ----------------------------------------------------------------------------

struct BitCrusher {
    held: [f32; 2],
    count: f32,
}

impl Device for BitCrusher {
    fn id(&self) -> &'static str {
        "native.bitcrusher"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let bits = p(params, 0, 8.0).round().clamp(1.0, 16.0);
        let reduce = p(params, 1, 1.0).max(1.0);
        let mix = p(params, 2, 1.0);
        // Mid-rise quantizer: n bits give 2^n levels.
        let levels = 2f32.powf(bits);
        let quant = |x: f32| ((x.clamp(-1.0, 0.9999) * levels * 0.5).floor() + 0.5) * 2.0 / levels;
        for fr in buf.chunks_exact_mut(2) {
            if self.count <= 0.0 {
                self.held = [quant(fr[0]), quant(fr[1])];
                self.count += reduce;
            }
            self.count -= 1.0;
            fr[0] = fr[0] * (1.0 - mix) + self.held[0] * mix;
            fr[1] = fr[1] * (1.0 - mix) + self.held[1] * mix;
        }
    }
}

// ---- stereo width ---------------------------------------------------------------------------

struct Width;

impl Device for Width {
    fn id(&self) -> &'static str {
        "native.width"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let width = p(params, 0, 1.0).clamp(0.0, 2.0);
        for fr in buf.chunks_exact_mut(2) {
            let (mid, side) = ((fr[0] + fr[1]) * 0.5, (fr[0] - fr[1]) * 0.5 * width);
            fr[0] = mid + side;
            fr[1] = mid - side;
        }
    }
}

// ---- tremolo / auto-pan ---------------------------------------------------------------------

struct AutoPan {
    phase: f32,
    fs: f32,
}

impl Device for AutoPan {
    fn id(&self) -> &'static str {
        "native.autopan"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (rate, depth, pan_mode) =
            (p(params, 0, 2.0), p(params, 1, 0.5).clamp(0.0, 1.0), p(params, 2, 0.0) >= 0.5);
        let inc = rate / self.fs;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + inc).fract();
            let lfo = sine_phase(self.phase); // -1..1
            if pan_mode {
                // Equal-ish power swing between the channels.
                fr[0] *= 1.0 - depth * (0.5 + 0.5 * lfo);
                fr[1] *= 1.0 - depth * (0.5 - 0.5 * lfo);
            } else {
                let g = 1.0 - depth * (0.5 + 0.5 * lfo);
                fr[0] *= g;
                fr[1] *= g;
            }
        }
    }
}

// ---- modulators -----------------------------------------------------------------------------
// They leave the audio untouched and publish one control value per block (see `Device::modulation`).

struct Lfo {
    phase: f32,
    held: f32,
    rng: u32,
    value: f32,
    fs: f32,
    bpm: f32,
}

impl Device for Lfo {
    fn id(&self) -> &'static str {
        "native.lfo"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let shape = p(params, 2, 0.0).round() as i32;
        let mut rate = p(params, 3, 1.0).clamp(0.05, 20.0);
        // tempo sync: how many beats one cycle lasts
        const BEATS: [f32; 9] = [0.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125, 1.0 / 3.0, 1.0 / 6.0];
        let sync = p(params, 6, 0.0).round().clamp(0.0, 8.0) as usize;
        if sync > 0 {
            rate = (self.bpm.clamp(20.0, 999.0) / 60.0 / BEATS[sync]).clamp(0.05, 60.0);
        }
        let depth = p(params, 4, 0.5).clamp(-1.0, 1.0);
        let offset = p(params, 5, 0.0).clamp(0.0, 1.0);
        let before = self.phase;
        self.phase = (self.phase + rate * (buf.len() / 2) as f32 / self.fs).fract();
        if self.phase < before {
            self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.held = (self.rng >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        }
        let ph = (self.phase + offset).fract();
        let v = match shape {
            1 => 1.0 - 4.0 * (ph - 0.5).abs(),
            2 => ph * 2.0 - 1.0,
            3 => {
                if ph < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            4 => self.held,
            _ => sine_phase(ph),
        };
        self.value = v * depth;
    }

    fn modulation(&self) -> Option<f32> {
        Some(self.value)
    }

    fn set_tempo(&mut self, bpm: f32) {
        if bpm.is_finite() {
            self.bpm = bpm;
        }
    }
}

struct Follower {
    env: f32,
    value: f32,
    fs: f32,
}

impl Device for Follower {
    fn id(&self) -> &'static str {
        "native.follower"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let attack = p(params, 2, 10.0).max(1.0) / 1000.0;
        let release = p(params, 3, 150.0).max(5.0) / 1000.0;
        let sens = p(params, 4, 4.0).clamp(0.1, 16.0);
        let depth = p(params, 5, 0.5).clamp(-1.0, 1.0);
        let block = (buf.len() / 2) as f32 / self.fs;
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let tau = if peak > self.env { attack } else { release };
        self.env += (peak - self.env) * (1.0 - (-block / tau).exp());
        self.value = (self.env * sens).clamp(0.0, 1.0) * depth;
    }

    fn modulation(&self) -> Option<f32> {
        Some(self.value)
    }
}

// ---- exciter --------------------------------------------------------------------------------

struct Exciter {
    hp: [Biquad; 2],
    fs: f32,
}

impl Device for Exciter {
    fn id(&self) -> &'static str {
        "native.exciter"
    }

    #[allow(clippy::needless_range_loop)] // index is shared by several per-channel arrays
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (freq, amount) = (p(params, 0, 3000.0), p(params, 1, 0.3));
        let c = Coeffs::highpass(self.fs, freq, 0.707);
        for fr in buf.chunks_exact_mut(2) {
            for ch in 0..2 {
                let hi = self.hp[ch].process(&c, fr[ch]);
                // Saturate the high band to create new harmonics, then blend it back in.
                fr[ch] += (hi * 6.0).tanh() * amount * 0.5;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trakr_core::device_info;

    const FS: u32 = 44_100;

    fn params(id: &str, set: &[(usize, f32)]) -> Vec<f32> {
        let mut p = device_info(id).unwrap().defaults();
        for &(i, v) in set {
            p[i] = v;
        }
        p
    }

    /// Run `secs` of a stereo signal (`gen(i) -> (l, r)`) through a device in 256-frame blocks.
    fn run(id: &str, set: &[(usize, f32)], secs: f32, gen: impl Fn(usize) -> (f32, f32)) -> Vec<(f32, f32)> {
        let mut dev = create(id, FS).unwrap();
        let p = params(id, set);
        let n = (FS as f32 * secs) as usize;
        let mut out = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            let m = (n - i).min(256);
            let mut buf: Vec<f32> = (i..i + m)
                .flat_map(|k| {
                    let (l, r) = gen(k);
                    [l, r]
                })
                .collect();
            dev.process(&p, &mut buf);
            out.extend(buf.chunks_exact(2).map(|f| (f[0], f[1])));
            i += m;
        }
        out
    }

    fn sine(freq: f32, amp: f32) -> impl Fn(usize) -> (f32, f32) {
        move |i| {
            let v = (i as f32 / FS as f32 * freq * std::f32::consts::TAU).sin() * amp;
            (v, v)
        }
    }

    fn peak(v: &[(f32, f32)]) -> f32 {
        v.iter().fold(0.0f32, |m, s| m.max(s.0.abs()).max(s.1.abs()))
    }

    #[test]
    fn every_catalogue_device_can_be_built_and_stays_finite() {
        for d in trakr_core::DEVICES {
            let out = run(d.id, &[], 0.3, sine(300.0, 0.7));
            assert!(out.iter().all(|s| s.0.is_finite() && s.1.is_finite()), "{} produced NaN/inf", d.id);
            assert!(peak(&out) < 8.0, "{} blew up: {}", d.id, peak(&out));
            // extreme settings must stay finite too
            let extreme: Vec<(usize, f32)> = d.params.iter().enumerate().map(|(i, p)| (i, p.max)).collect();
            let out = run(d.id, &extreme, 0.2, sine(5000.0, 1.0));
            assert!(out.iter().all(|s| s.0.is_finite() && s.1.is_finite()), "{} extreme NaN", d.id);
            let low: Vec<(usize, f32)> = d.params.iter().enumerate().map(|(i, p)| (i, p.min)).collect();
            let out = run(d.id, &low, 0.2, sine(50.0, 1.0));
            assert!(out.iter().all(|s| s.0.is_finite() && s.1.is_finite()), "{} minimum NaN", d.id);
        }
    }

    #[test]
    fn eq10_boosts_its_band() {
        let out = run("native.eq10", &[(5, 12.0)], 0.5, sine(1000.0, 0.2));
        let p = peak(&out[10_000..]);
        assert!((p / 0.2 - 3.98).abs() < 0.4, "gain {}", p / 0.2);
        let flat = run("native.eq10", &[], 0.3, sine(1000.0, 0.2));
        assert!((peak(&flat[5000..]) - 0.2).abs() < 0.01);
    }

    #[test]
    fn ladder_is_a_steep_low_pass() {
        let low = run("native.ladder", &[(0, 500.0), (1, 0.0)], 0.5, sine(100.0, 0.5));
        let high = run("native.ladder", &[(0, 500.0), (1, 0.0)], 0.5, sine(4000.0, 0.5));
        assert!(peak(&low[10_000..]) > 0.3, "passband {}", peak(&low[10_000..]));
        assert!(peak(&high[10_000..]) < 0.01, "stopband {}", peak(&high[10_000..]));
        // resonance lifts the region around the cutoff
        let flat = run("native.ladder", &[(0, 500.0), (1, 0.0)], 0.5, sine(500.0, 0.2));
        let reso = run("native.ladder", &[(0, 500.0), (1, 0.9)], 0.5, sine(500.0, 0.2));
        assert!(peak(&reso[10_000..]) > peak(&flat[10_000..]) * 1.5);
    }

    #[test]
    fn gate_closes_on_quiet_signals() {
        let loud = run("native.gate", &[(0, -30.0)], 0.4, sine(200.0, 0.5));
        assert!(peak(&loud[10_000..]) > 0.45, "open gate attenuated: {}", peak(&loud[10_000..]));
        let quiet = run("native.gate", &[(0, -30.0), (3, 60.0)], 0.6, sine(200.0, 0.005));
        assert!(peak(&quiet[20_000..]) < 0.0005, "closed gate leaked: {}", peak(&quiet[20_000..]));
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        // -6 dB ceiling = 0.501; feed a full-scale burst after silence
        let out = run("native.limiter", &[(1, -6.0206)], 0.5, |i| {
            let v = if i > 4000 { (i as f32 * 0.05).sin() * 1.0 } else { 0.0 };
            (v, v)
        });
        assert!(peak(&out) < 0.51, "peak {}", peak(&out));
        assert!(peak(&out[10_000..]) > 0.45, "should not over-attenuate");
        // quiet material passes unchanged (apart from the lookahead delay)
        let quiet = run("native.limiter", &[], 0.3, sine(300.0, 0.2));
        assert!((peak(&quiet[5000..]) - 0.2).abs() < 0.005);
    }

    #[test]
    fn phaser_changes_the_sound_and_stays_bounded() {
        let dry = run("native.phaser", &[(3, 0.0)], 0.5, sine(800.0, 0.5));
        let wet = run("native.phaser", &[(3, 1.0)], 0.5, sine(800.0, 0.5));
        let diff: f32 =
            dry[5000..15_000].iter().zip(&wet[5000..15_000]).map(|(a, b)| (a.0 - b.0).abs()).sum();
        assert!(diff > 20.0, "phaser had no effect: {diff}");
        assert!(peak(&wet) < 1.5);
    }

    #[test]
    fn ring_modulation_of_a_sine_by_itself_is_unipolar() {
        let out = run("native.ringmod", &[(0, 100.0), (1, 1.0)], 0.3, sine(100.0, 0.8));
        // sin * sin = sin^2 >= 0 (apart from the phase offset between the two oscillators)
        let (lo, hi) = out[4000..].iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(s.0), h.max(s.0)));
        assert!(hi > 0.1 || lo < -0.1, "no output");
        let flat = run("native.ringmod", &[(0, 100.0), (1, 0.0)], 0.1, sine(100.0, 0.8));
        assert!((peak(&flat[1000..]) - 0.8).abs() < 0.01, "mix 0 must be transparent");
    }

    #[test]
    fn bit_crusher_quantizes() {
        let out = run("native.bitcrusher", &[(0, 2.0), (1, 1.0), (2, 1.0)], 0.2, sine(100.0, 0.9));
        let mut levels: Vec<i32> = out.iter().map(|s| (s.0 * 1000.0).round() as i32).collect();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() <= 4, "2-bit output should have at most 4 levels: {levels:?}");
        // rate reduction holds samples
        let held = run("native.bitcrusher", &[(0, 16.0), (1, 8.0)], 0.05, sine(1000.0, 0.5));
        assert!(held[1000..1100].windows(2).filter(|w| w[0].0 == w[1].0).count() > 80);
    }

    #[test]
    fn stereo_width_folds_to_mono_and_widens() {
        let mono = run("native.width", &[(0, 0.0)], 0.01, |_| (1.0, 0.0));
        assert!(mono.iter().all(|s| (s.0 - 0.5).abs() < 1e-5 && (s.1 - 0.5).abs() < 1e-5));
        let wide = run("native.width", &[(0, 2.0)], 0.01, |_| (1.0, 0.0));
        assert!((wide[10].0 - 1.5).abs() < 1e-5 && (wide[10].1 + 0.5).abs() < 1e-5);
        let same = run("native.width", &[(0, 1.0)], 0.01, |_| (0.3, -0.7));
        assert!((same[10].0 - 0.3).abs() < 1e-5 && (same[10].1 + 0.7).abs() < 1e-5);
    }

    #[test]
    fn tremolo_modulates_amplitude_and_autopan_swings_channels() {
        let trem = run("native.autopan", &[(0, 5.0), (1, 1.0), (2, 0.0)], 1.0, |_| (1.0, 1.0));
        let (lo, hi) = trem.iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(s.0), h.max(s.0)));
        assert!(lo < 0.05 && hi > 0.95, "tremolo range {lo}..{hi}");
        let pan = run("native.autopan", &[(0, 5.0), (1, 1.0), (2, 1.0)], 1.0, |_| (1.0, 1.0));
        assert!(pan.iter().any(|s| s.0 < 0.1 && s.1 > 0.9) && pan.iter().any(|s| s.1 < 0.1 && s.0 > 0.9));
    }

    #[test]
    fn exciter_adds_high_harmonics_only_when_asked() {
        let src = |i: usize| {
            let t = i as f32 / FS as f32;
            let v = (t * 6000.0 * std::f32::consts::TAU).sin() * 0.3;
            (v, v)
        };
        let off = run("native.exciter", &[(1, 0.0)], 0.1, src);
        let on = run("native.exciter", &[(1, 1.0)], 0.1, src);
        assert!((peak(&off[500..]) - 0.3).abs() < 0.01);
        assert!(peak(&on[500..]) > 0.45, "exciter should add level: {}", peak(&on[500..]));
    }
}
