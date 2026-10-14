//! Native track devices. Each owns its state (allocated when built on the UI thread) and processes
//! interleaved stereo blocks in place without allocating. Parameters arrive in catalogue order.

use crate::dsp::{db_to_gain, gain_to_db, sine_phase, smooth_coeff, Biquad, Coeffs, DelayLine};

pub trait Device: Send {
    /// Process `buf` (interleaved stereo) in place with the current parameter values.
    fn process(&mut self, params: &[f32], buf: &mut [f32]);
}

/// Build a device for a catalogue id at the engine's sample rate.
pub fn create(id: &str, rate: u32) -> Option<Box<dyn Device>> {
    let fs = rate as f32;
    Some(match id {
        "native.gainer" => Box::new(Gainer { gain: 1.0, fs }),
        "native.filter" => Box::new(FilterDev { l: Biquad::default(), r: Biquad::default(), fs }),
        "native.eq" => Box::new(Eq { bands: [[Biquad::default(); 2]; 3], fs }),
        "native.delay" => Box::new(Delay::new(fs)),
        "native.reverb" => Box::new(Reverb::new(fs)),
        "native.chorus" => Box::new(Chorus::new(fs)),
        "native.distortion" => Box::new(Distortion { gain: 1.0, fs }),
        "native.compressor" => Box::new(Compressor { env_db: -90.0, fs }),
        _ => return None,
    })
}

fn p(params: &[f32], i: usize, default: f32) -> f32 {
    params.get(i).copied().unwrap_or(default)
}

// ---- gainer -----------------------------------------------------------------------------

struct Gainer {
    gain: f32,
    fs: f32,
}

impl Device for Gainer {
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let target = db_to_gain(p(params, 0, 0.0));
        let target = if p(params, 0, 0.0) <= -59.9 { 0.0 } else { target };
        let pan = p(params, 1, 0.0).clamp(-1.0, 1.0);
        let (pl, pr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
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
            for (o, (combs, allpass)) in out.iter_mut().zip(self.combs.iter_mut().zip(self.allpass.iter_mut())) {
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
}

impl Device for Distortion {
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let drive = db_to_gain(p(params, 0, 12.0));
        let (mix, out) = (p(params, 1, 1.0), db_to_gain(p(params, 2, -6.0)));
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
