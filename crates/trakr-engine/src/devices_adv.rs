//! More native devices: maximizer, analog-style filter, mid/side, convolver, key / velocity trackers,
//! meta mixer, formula modulator and the `#Send` device (whose routing the engine performs).

use crate::devices::{Device, Limiter};
use crate::dsp::{db_to_gain, Biquad, Coeffs};
use crate::formula::{Formula, Inputs};
use std::f32::consts::PI;
use trakr_core::fft::fft;

fn p(params: &[f32], i: usize, default: f32) -> f32 {
    params.get(i).copied().filter(|v| v.is_finite()).unwrap_or(default)
}

pub(crate) fn create(id: &str, fs: f32) -> Option<Box<dyn Device>> {
    Some(match id {
        "native.maximizer" => Box::new(Maximizer { lim: Limiter::new(fs) }),
        "native.analogfilter" => Box::new(AnalogFilter { ic1: [0.0; 2], ic2: [0.0; 2], fs }),
        "native.midside" => Box::new(MidSide { low: [Biquad::default(); 2], fs }),
        "native.convolver" => Box::new(Convolver::empty()),
        "native.keytracker" => Box::new(Tracker { key: true, note: 48, vel: 1.0, value: 0.0 }),
        "native.veltracker" => Box::new(Tracker { key: false, note: 48, vel: 1.0, value: 0.0 }),
        "native.metamixer" => Box::new(MetaMixer),
        "native.formula" => Box::new(FormulaDev::new(Formula::default(), fs)),
        "native.send" => Box::new(SendMarker),
        _ => return None,
    })
}

/// A formula modulator running `source`. Returns the compile error, if any, with a silent device.
pub fn create_formula(source: &str, fs: f32) -> (Box<dyn Device>, Option<String>) {
    match Formula::compile(source) {
        Ok(f) => (Box::new(FormulaDev::new(f, fs)), None),
        Err(e) => (Box::new(FormulaDev::new(Formula::default(), fs)), Some(e)),
    }
}

/// A convolution reverb using `ir` (interleaved, `channels` 1 or 2, recorded at `ir_rate`) at engine rate `fs`.
pub fn create_convolver(ir: &[f32], channels: usize, ir_rate: u32, fs: u32) -> Box<dyn Device> {
    Box::new(Convolver::new(ir, channels.max(1), ir_rate.max(1), fs.max(1)))
}

// ---- maximizer -----------------------------------------------------------------------------

struct Maximizer {
    lim: Limiter,
}

impl Device for Maximizer {
    fn id(&self) -> &'static str {
        "native.maximizer"
    }

    /// Params: gain (dB), ceiling (dB), release (ms), soft clip (0..1): drive into a tanh shaper, then a
    /// look-ahead limiter at the ceiling.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let gain = db_to_gain(p(params, 0, 6.0).clamp(0.0, 24.0));
        let soft = p(params, 3, 0.3).clamp(0.0, 1.0);
        for s in buf.iter_mut() {
            let x = *s * gain;
            // blend between the clean signal and a saturated one, matching the level at small signals
            *s = x * (1.0 - soft) + x.tanh() * soft;
        }
        self.lim.process(&[0.0, p(params, 1, -0.3), p(params, 2, 100.0)], buf);
    }
}

// ---- analog-style state variable filter -------------------------------------------------------

struct AnalogFilter {
    ic1: [f32; 2],
    ic2: [f32; 2],
    fs: f32,
}

impl Device for AnalogFilter {
    fn id(&self) -> &'static str {
        "native.analogfilter"
    }

    /// Params: mode (0 low-pass, 1 band-pass, 2 high-pass, 3 notch), cutoff, resonance (0..1), drive (dB), mix.
    /// A zero-delay-feedback state variable filter with tanh saturation in the input and the integrators.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let mode = p(params, 0, 0.0).round() as i32;
        let cutoff = p(params, 1, 1000.0).clamp(20.0, self.fs * 0.45);
        let res = p(params, 2, 0.3).clamp(0.0, 1.0);
        let drive = db_to_gain(p(params, 3, 0.0).clamp(0.0, 36.0));
        let mix = p(params, 4, 1.0).clamp(0.0, 1.0);
        let g = (PI * cutoff / self.fs).tan();
        let k = 2.0 - 1.98 * res; // 2 (no resonance) .. 0.02 (self-oscillation edge)
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let comp = 1.0 / drive.sqrt(); // keep the loudness roughly level when driving
        for fr in buf.chunks_exact_mut(2) {
            #[allow(clippy::needless_range_loop)]
            for ch in 0..2 {
                let dry = fr[ch];
                let v0 = (dry * drive).tanh();
                let v3 = v0 - self.ic2[ch];
                let v1 = a1 * self.ic1[ch] + a2 * v3;
                let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
                self.ic1[ch] = (2.0 * v1 - self.ic1[ch]).clamp(-8.0, 8.0);
                self.ic2[ch] = (2.0 * v2 - self.ic2[ch]).clamp(-8.0, 8.0);
                self.ic1[ch] = self.ic1[ch].tanh(); // soft saturation of the state
                let out = match mode {
                    1 => v1 * k,
                    2 => v0 - k * v1 - v2,
                    3 => v0 - k * v1,
                    _ => v2,
                };
                let out = if out.is_finite() { out * comp } else { 0.0 };
                fr[ch] = dry * (1.0 - mix) + out * mix;
            }
        }
        if !self.ic1.iter().chain(&self.ic2).all(|v| v.is_finite()) {
            self.ic1 = [0.0; 2];
            self.ic2 = [0.0; 2];
        }
    }
}

// ---- mid / side -------------------------------------------------------------------------------

struct MidSide {
    low: [Biquad; 2],
    fs: f32,
}

impl Device for MidSide {
    fn id(&self) -> &'static str {
        "native.midside"
    }

    /// Params: mid gain (dB), side gain (dB), bass mono below (Hz, 0 = off), swap (0/1: mid and side exchange roles).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let gm = db_to_gain(p(params, 0, 0.0).clamp(-48.0, 18.0));
        let gs = db_to_gain(p(params, 1, 0.0).clamp(-48.0, 18.0));
        let bass = p(params, 2, 0.0).clamp(0.0, 500.0);
        let c = Coeffs::lowpass(self.fs, bass.max(20.0), std::f32::consts::FRAC_1_SQRT_2);
        for fr in buf.chunks_exact_mut(2) {
            let (m, mut s) = ((fr[0] + fr[1]) * 0.5, (fr[0] - fr[1]) * 0.5);
            if bass > 0.0 {
                let a = self.low[0].process(&c, s);
                let low = self.low[1].process(&c, a);
                s -= low; // below the corner the side signal is removed: the bass collapses to mono
            }
            let (m, s) = (m * gm, s * gs);
            fr[0] = m + s;
            fr[1] = m - s;
        }
    }
}

// ---- convolver --------------------------------------------------------------------------------

const PART: usize = 512;
const FFT_N: usize = PART * 2;
const MAX_PARTS: usize = 256;

struct Convolver {
    /// Impulse response spectra per channel: `parts * FFT_N` values each for re and im.
    ir_re: [Vec<f32>; 2],
    ir_im: [Vec<f32>; 2],
    parts: usize,
    /// Spectra of recent input blocks, newest at `fdl_pos`.
    fdl_re: [Vec<f32>; 2],
    fdl_im: [Vec<f32>; 2],
    fdl_pos: usize,
    prev: [Vec<f32>; 2],
    inbuf: [Vec<f32>; 2],
    outbuf: [Vec<f32>; 2],
    dry: [Vec<f32>; 2],
    fill: usize,
    work_re: Vec<f32>,
    work_im: Vec<f32>,
    acc_re: Vec<f32>,
    acc_im: Vec<f32>,
}

impl Convolver {
    fn empty() -> Self {
        Self::new(&[], 1, 44_100, 44_100)
    }

    fn new(ir: &[f32], channels: usize, ir_rate: u32, fs: u32) -> Self {
        // resample the response to the engine rate (linear), cap its length, take it as stereo
        let frames = ir.len() / channels;
        let ratio = fs as f64 / ir_rate as f64;
        let out_frames = (((frames as f64) * ratio) as usize).min(PART * MAX_PARTS);
        let mut chans = [vec![0.0f32; out_frames], vec![0.0f32; out_frames]];
        for (c, ch) in chans.iter_mut().enumerate() {
            let src_c = c.min(channels - 1);
            for (i, o) in ch.iter_mut().enumerate() {
                let pos = i as f64 / ratio;
                let a = pos.floor() as usize;
                let f = (pos - a as f64) as f32;
                let at = |k: usize| {
                    ir.get(k.min(frames.saturating_sub(1)) * channels + src_c).copied().unwrap_or(0.0)
                };
                *o = at(a) * (1.0 - f) + at(a + 1) * f;
            }
        }
        // equal energy whatever the response's level
        let energy: f32 = chans.iter().flat_map(|c| c.iter()).map(|v| v * v).sum::<f32>() / 2.0;
        let norm = if energy > 1e-12 { 1.0 / energy.sqrt() } else { 0.0 };
        let parts = out_frames.div_ceil(PART).max(1);
        let mut me = Self {
            ir_re: [vec![0.0; parts * FFT_N], vec![0.0; parts * FFT_N]],
            ir_im: [vec![0.0; parts * FFT_N], vec![0.0; parts * FFT_N]],
            parts,
            fdl_re: [vec![0.0; parts * FFT_N], vec![0.0; parts * FFT_N]],
            fdl_im: [vec![0.0; parts * FFT_N], vec![0.0; parts * FFT_N]],
            fdl_pos: 0,
            prev: [vec![0.0; PART], vec![0.0; PART]],
            inbuf: [vec![0.0; PART], vec![0.0; PART]],
            outbuf: [vec![0.0; PART], vec![0.0; PART]],
            dry: [vec![0.0; PART], vec![0.0; PART]],
            fill: 0,
            work_re: vec![0.0; FFT_N],
            work_im: vec![0.0; FFT_N],
            acc_re: vec![0.0; FFT_N],
            acc_im: vec![0.0; FFT_N],
        };
        for (c, ch) in chans.iter().enumerate() {
            for k in 0..parts {
                let mut re = vec![0.0f32; FFT_N];
                let mut im = vec![0.0f32; FFT_N];
                #[allow(clippy::needless_range_loop)]
                for i in 0..PART {
                    re[i] = ch.get(k * PART + i).copied().unwrap_or(0.0) * norm;
                }
                fft(&mut re, &mut im, false);
                me.ir_re[c][k * FFT_N..(k + 1) * FFT_N].copy_from_slice(&re);
                me.ir_im[c][k * FFT_N..(k + 1) * FFT_N].copy_from_slice(&im);
            }
        }
        me
    }

    /// Convolve the filled input block (all channels) and refill the output block.
    fn run_block(&mut self) {
        self.fdl_pos = (self.fdl_pos + self.parts - 1) % self.parts;
        for c in 0..2 {
            self.work_re[..PART].copy_from_slice(&self.prev[c]);
            self.work_re[PART..].copy_from_slice(&self.inbuf[c]);
            self.work_im.fill(0.0);
            fft(&mut self.work_re, &mut self.work_im, false);
            let at = self.fdl_pos * FFT_N;
            self.fdl_re[c][at..at + FFT_N].copy_from_slice(&self.work_re);
            self.fdl_im[c][at..at + FFT_N].copy_from_slice(&self.work_im);
            self.prev[c].copy_from_slice(&self.inbuf[c]);

            self.acc_re.fill(0.0);
            self.acc_im.fill(0.0);
            for k in 0..self.parts {
                let f = ((self.fdl_pos + k) % self.parts) * FFT_N;
                let h = k * FFT_N;
                let (fr, fi) = (&self.fdl_re[c][f..f + FFT_N], &self.fdl_im[c][f..f + FFT_N]);
                let (hr, hi) = (&self.ir_re[c][h..h + FFT_N], &self.ir_im[c][h..h + FFT_N]);
                for j in 0..FFT_N {
                    self.acc_re[j] += fr[j] * hr[j] - fi[j] * hi[j];
                    self.acc_im[j] += fr[j] * hi[j] + fi[j] * hr[j];
                }
            }
            fft(&mut self.acc_re, &mut self.acc_im, true);
            for i in 0..PART {
                self.outbuf[c][i] = self.acc_re[PART + i] / FFT_N as f32;
            }
        }
    }
}

impl Device for Convolver {
    fn id(&self) -> &'static str {
        "native.convolver"
    }

    /// Params: instrument and sample that hold the response (used when the device is built), mix, gain (dB).
    /// The wet signal is delayed by 512 frames (the dry path is delayed to match).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let mix = p(params, 2, 0.3).clamp(0.0, 1.0);
        let gain = db_to_gain(p(params, 3, 0.0).clamp(-48.0, 18.0));
        for fr in buf.chunks_exact_mut(2) {
            let i = self.fill;
            let (wl, wr) = (self.outbuf[0][i], self.outbuf[1][i]);
            let (dl, dr) = (self.dry[0][i], self.dry[1][i]);
            self.inbuf[0][i] = fr[0];
            self.inbuf[1][i] = fr[1];
            self.dry[0][i] = fr[0];
            self.dry[1][i] = fr[1];
            fr[0] = dl * (1.0 - mix) + wl * gain * mix;
            fr[1] = dr * (1.0 - mix) + wr * gain * mix;
            self.fill += 1;
            if self.fill == PART {
                self.fill = 0;
                self.run_block();
            }
        }
    }
}

// ---- key / velocity trackers --------------------------------------------------------------------

/// Modulator following the last note played on the track: its key or its velocity.
struct Tracker {
    key: bool,
    note: u8,
    vel: f32,
    value: f32,
}

impl Device for Tracker {
    fn id(&self) -> &'static str {
        if self.key {
            "native.keytracker"
        } else {
            "native.veltracker"
        }
    }

    /// Key tracker params: target device, target parameter, depth, lowest note, highest note, bipolar.
    /// Velocity tracker params: target device, target parameter, depth, bipolar.
    fn process(&mut self, params: &[f32], _buf: &mut [f32]) {
        let depth = p(params, 2, 1.0).clamp(-1.0, 1.0);
        let (x, bipolar) = if self.key {
            let (lo, hi) = (p(params, 3, 24.0), p(params, 4, 96.0));
            (((self.note as f32 - lo) / (hi - lo).abs().max(1.0)).clamp(0.0, 1.0), p(params, 5, 0.0) >= 0.5)
        } else {
            (self.vel.clamp(0.0, 1.0), p(params, 3, 0.0) >= 0.5)
        };
        self.value = (if bipolar { x * 2.0 - 1.0 } else { x }) * depth;
    }

    fn modulation(&self) -> Option<f32> {
        Some(self.value)
    }

    fn set_track_note(&mut self, note: u8, velocity: f32) {
        self.note = note;
        self.vel = velocity;
    }
}

// ---- meta mixer -------------------------------------------------------------------------------------

/// Four knobs, each moving one parameter of another device. The engine applies the knob values itself.
struct MetaMixer;

impl Device for MetaMixer {
    fn id(&self) -> &'static str {
        "native.metamixer"
    }

    fn process(&mut self, _params: &[f32], _buf: &mut [f32]) {}

    fn modulation(&self) -> Option<f32> {
        Some(1.0)
    }
}

// ---- formula modulator --------------------------------------------------------------------------------

struct FormulaDev {
    formula: Formula,
    t: f32,
    beat: f32,
    bpm: f32,
    note: u8,
    vel: f32,
    value: f32,
    fs: f32,
}

impl FormulaDev {
    fn new(formula: Formula, fs: f32) -> Self {
        Self { formula, t: 0.0, beat: 0.0, bpm: 120.0, note: 48, vel: 1.0, value: 0.0, fs }
    }
}

impl Device for FormulaDev {
    fn id(&self) -> &'static str {
        "native.formula"
    }

    /// Params: target device, target parameter, knobs a b c d (0..1), depth. The expression is the device's text.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let dt = (buf.len() / 2) as f32 / self.fs;
        self.t = (self.t + dt) % 100_000.0;
        self.beat = (self.beat + dt * self.bpm / 60.0) % 1_000_000.0;
        let knobs = [p(params, 2, 0.0), p(params, 3, 0.0), p(params, 4, 0.0), p(params, 5, 0.0)];
        let v = self.formula.eval(&Inputs {
            t: self.t,
            beat: self.beat,
            bpm: self.bpm,
            knobs,
            note: self.note as f32,
            vel: self.vel,
        });
        self.value = v * p(params, 6, 1.0).clamp(-1.0, 1.0);
    }

    fn modulation(&self) -> Option<f32> {
        Some(self.value)
    }

    fn set_tempo(&mut self, bpm: f32) {
        if bpm.is_finite() {
            self.bpm = bpm;
        }
    }

    fn set_track_note(&mut self, note: u8, velocity: f32) {
        self.note = note;
        self.vel = velocity;
    }
}

// ---- #Send ------------------------------------------------------------------------------------------------

/// Marks a chain slot as a send; `Engine::run_chain` does the routing (it needs the other tracks' buffers).
struct SendMarker;

impl Device for SendMarker {
    fn id(&self) -> &'static str {
        "native.send"
    }

    fn process(&mut self, _params: &[f32], _buf: &mut [f32]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults(id: &str) -> Vec<f32> {
        trakr_core::DeviceSpec::new(id).expect("known id").params
    }

    fn sine(n: usize) -> Vec<f32> {
        (0..n)
            .flat_map(|i| {
                let v = (i as f32 * 0.05).sin() * 0.9;
                [v, v]
            })
            .collect()
    }

    fn run(id: &str, buf: &mut [f32]) {
        let mut d = create(id, 48000.0).unwrap();
        let params = defaults(id);
        for chunk in buf.chunks_mut(1024) {
            d.process(&params, chunk);
        }
    }

    #[test]
    fn effects_stay_finite_and_bounded() {
        for id in ["native.maximizer", "native.analogfilter", "native.midside"] {
            let mut buf = sine(8192);
            run(id, &mut buf);
            assert!(buf.iter().all(|v| v.is_finite() && v.abs() < 4.0), "{id}");
            assert!(buf.iter().any(|v| v.abs() > 0.001), "{id} silent");
        }
    }

    #[test]
    fn maximizer_respects_ceiling() {
        let mut buf: Vec<f32> = sine(8192).iter().map(|v| v * 3.0).collect();
        run("native.maximizer", &mut buf);
        assert!(buf[4000..].iter().all(|v| v.abs() <= 1.01));
    }

    #[test]
    fn convolver_with_impulse_ir_delays_the_input() {
        let mut ir = vec![0.0f32; 200];
        ir[100] = 1.0;
        let mut d = create_convolver(&ir, 1, 48000, 48000);
        let params = defaults("native.convolver");
        let mut buf = vec![0.0f32; 2 * 4096];
        buf[0] = 1.0;
        buf[1] = 1.0;
        for chunk in buf.chunks_mut(1024) {
            d.process(&params, chunk);
        }
        let peak = (0..4096).max_by(|&a, &b| buf[a * 2].abs().total_cmp(&buf[b * 2].abs())).unwrap();
        assert!((90..=700).contains(&peak), "peak at {peak}");
        assert!(buf.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn trackers_publish_values() {
        let mut k = create("native.keytracker", 48000.0).unwrap();
        k.set_track_note(60, 0.5);
        k.process(&defaults("native.keytracker"), &mut [0.0; 64]);
        let kv = k.modulation().unwrap();
        let mut v = create("native.veltracker", 48000.0).unwrap();
        v.set_track_note(60, 0.25);
        v.process(&defaults("native.veltracker"), &mut [0.0; 64]);
        let vv = v.modulation().unwrap();
        assert!((0.0..=1.0).contains(&kv) && (0.0..=1.0).contains(&vv));
        assert!(kv > 0.0 || vv > 0.0);
    }

    #[test]
    fn formula_device_modulates_and_reports_errors() {
        let (mut d, err) = create_formula("0.5", 48000.0);
        assert!(err.is_none());
        d.process(&defaults("native.formula"), &mut [0.0; 64]);
        assert!(d.modulation().is_some());
        let (_, err) = create_formula("1 +", 48000.0);
        assert!(err.is_some());
    }
}
