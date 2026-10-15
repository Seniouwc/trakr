//! Further native devices: comb filter, flanger, pitch shifter, saturator, transient shaper, track delay
//! and auto filter.

use crate::devices::Device;
use crate::dsp::{db_to_gain, smooth_coeff, Biquad, Coeffs, DelayLine};

fn p(params: &[f32], i: usize, default: f32) -> f32 {
    params.get(i).copied().filter(|v| v.is_finite()).unwrap_or(default)
}

pub(crate) fn create(id: &str, fs: f32) -> Option<Box<dyn Device>> {
    Some(match id {
        "native.comb" => Box::new(Comb::new(fs)),
        "native.flanger" => Box::new(Flanger::new(fs)),
        "native.pitchshift" => Box::new(PitchShift::new(fs)),
        "native.saturator" => Box::new(Saturator { hp: [Biquad::default(); 2], fs }),
        "native.transient" => Box::new(Transient { fast: 0.0, slow: 0.0, gain: 1.0, fs }),
        "native.trackdelay" => Box::new(TrackDelay::new(fs)),
        "native.autofilter" => Box::new(AutoFilter {
            env: 0.0,
            f: [Biquad::default(); 2],
            coeffs: Coeffs::lowpass(fs, 1000.0, 1.0),
            fs,
        }),
        _ => return None,
    })
}

// ---- comb filter -------------------------------------------------------------------------------

struct Comb {
    lines: [DelayLine; 2],
    fs: f32,
}

impl Comb {
    fn new(fs: f32) -> Self {
        let n = (fs * 0.05) as usize + 4; // down to 20 Hz
        Self { lines: [DelayLine::new(n), DelayLine::new(n)], fs }
    }
}

impl Device for Comb {
    fn id(&self) -> &'static str {
        "native.comb"
    }

    /// Params: frequency (Hz), feedback (-0.95..0.95), mix.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let freq = p(params, 0, 440.0).clamp(20.0, 8000.0);
        let fb = p(params, 1, 0.7).clamp(-0.95, 0.95);
        let mix = p(params, 2, 0.5).clamp(0.0, 1.0);
        let delay = self.fs / freq;
        for fr in buf.chunks_exact_mut(2) {
            for (c, x) in fr.iter_mut().enumerate() {
                let wet = self.lines[c].read(delay);
                self.lines[c].write(*x + wet * fb);
                *x = *x * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---- flanger -----------------------------------------------------------------------------------

struct Flanger {
    lines: [DelayLine; 2],
    phase: f32,
    fs: f32,
}

impl Flanger {
    fn new(fs: f32) -> Self {
        let n = (fs * 0.02) as usize + 4;
        Self { lines: [DelayLine::new(n), DelayLine::new(n)], phase: 0.0, fs }
    }
}

impl Device for Flanger {
    fn id(&self) -> &'static str {
        "native.flanger"
    }

    /// Params: rate (Hz), depth (ms), centre delay (ms), feedback, mix.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let rate = p(params, 0, 0.3).clamp(0.01, 10.0);
        let depth = p(params, 1, 2.0).clamp(0.0, 8.0) * 0.001 * self.fs;
        let centre = p(params, 2, 3.0).clamp(0.5, 10.0) * 0.001 * self.fs;
        let fb = p(params, 3, 0.5).clamp(-0.95, 0.95);
        let mix = p(params, 4, 0.5).clamp(0.0, 1.0);
        let step = rate / self.fs;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + step).fract();
            for (c, x) in fr.iter_mut().enumerate() {
                // right channel runs a quarter cycle behind for width
                let ph = (self.phase + c as f32 * 0.25).fract();
                let d = centre + depth * (ph * std::f32::consts::TAU).sin();
                let wet = self.lines[c].read(d.max(1.0));
                self.lines[c].write(*x + wet * fb);
                *x = *x * (1.0 - mix * 0.5) + wet * mix;
            }
        }
    }
}

// ---- pitch shifter -----------------------------------------------------------------------------

/// Two crossfaded read heads over a delay line (the classic delay-line pitch shifter).
struct PitchShift {
    lines: [DelayLine; 2],
    phase: f32,
    fs: f32,
}

impl PitchShift {
    fn new(fs: f32) -> Self {
        let n = (fs * 0.12) as usize + 8;
        Self { lines: [DelayLine::new(n), DelayLine::new(n)], phase: 0.0, fs }
    }
}

impl Device for PitchShift {
    fn id(&self) -> &'static str {
        "native.pitchshift"
    }

    /// Params: semitones (-24..24), window (ms), mix.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let semis = p(params, 0, 0.0).clamp(-24.0, 24.0);
        let window = p(params, 1, 50.0).clamp(10.0, 100.0) * 0.001 * self.fs;
        let mix = p(params, 2, 1.0).clamp(0.0, 1.0);
        let ratio = 2f32.powf(semis / 12.0);
        // the read head sweeps through the window at (1 - ratio) samples per sample
        let step = (1.0 - ratio) / window;
        for fr in buf.chunks_exact_mut(2) {
            self.phase = (self.phase + step).rem_euclid(1.0);
            let (p1, p2) = (self.phase, (self.phase + 0.5).fract());
            // sine-squared crossfade keeps the level steady while a head wraps
            let (g1, g2) =
                ((std::f32::consts::PI * p1).sin().powi(2), (std::f32::consts::PI * p2).sin().powi(2));
            for (c, x) in fr.iter_mut().enumerate() {
                self.lines[c].write(*x);
                let wet =
                    self.lines[c].read(p1 * window + 1.0) * g1 + self.lines[c].read(p2 * window + 1.0) * g2;
                *x = *x * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---- saturator ---------------------------------------------------------------------------------

struct Saturator {
    hp: [Biquad; 2],
    fs: f32,
}

impl Device for Saturator {
    fn id(&self) -> &'static str {
        "native.saturator"
    }

    /// Params: drive (dB), tone/bias (-1..1: asymmetry for even harmonics), output (dB), mix.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let drive = db_to_gain(p(params, 0, 6.0).clamp(0.0, 36.0));
        let bias = p(params, 1, 0.0).clamp(-1.0, 1.0) * 0.5;
        let out = db_to_gain(p(params, 2, 0.0).clamp(-24.0, 12.0));
        let mix = p(params, 3, 1.0).clamp(0.0, 1.0);
        let dc = Coeffs::highpass(self.fs, 10.0, 0.707); // the bias creates DC; remove it again
        let norm = 1.0 / (1.0 + (drive - 1.0) * 0.04); // keep the loudness in the same neighbourhood
        for fr in buf.chunks_exact_mut(2) {
            for (c, x) in fr.iter_mut().enumerate() {
                let y = ((*x * drive + bias).tanh() - bias.tanh()) * norm;
                let y = self.hp[c].process(&dc, y) * out;
                *x = *x * (1.0 - mix) + y * mix;
            }
        }
    }
}

// ---- transient shaper --------------------------------------------------------------------------

struct Transient {
    fast: f32,
    slow: f32,
    gain: f32,
    fs: f32,
}

impl Device for Transient {
    fn id(&self) -> &'static str {
        "native.transient"
    }

    /// Params: attack (-1..1, boosts or softens onsets), sustain (-1..1, boosts or shortens tails).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let attack = p(params, 0, 0.0).clamp(-1.0, 1.0);
        let sustain = p(params, 1, 0.0).clamp(-1.0, 1.0);
        let (kf, ks) = (smooth_coeff(self.fs, 0.001), smooth_coeff(self.fs, 0.03));
        let kg = smooth_coeff(self.fs, 0.002);
        for fr in buf.chunks_exact_mut(2) {
            let level = fr[0].abs().max(fr[1].abs());
            self.fast += (level - self.fast) * kf;
            self.slow += (level - self.slow) * ks;
            // a fast envelope above the slow one is an onset; below it, the tail
            let onset = ((self.fast - self.slow) / (self.slow + 1e-4)).clamp(0.0, 4.0) / 4.0;
            let tail = ((self.slow - self.fast) / (self.slow + 1e-4)).clamp(0.0, 1.0);
            let target = (1.0 + attack * onset * 2.0 + sustain * tail).clamp(0.1, 4.0);
            self.gain += (target - self.gain) * kg;
            fr[0] *= self.gain;
            fr[1] *= self.gain;
        }
    }
}

// ---- track delay -------------------------------------------------------------------------------

/// Delays the whole signal by a fixed time (no feedback): nudge a track behind the beat.
struct TrackDelay {
    lines: [DelayLine; 2],
    fs: f32,
}

impl TrackDelay {
    fn new(fs: f32) -> Self {
        let n = (fs * 0.5) as usize + 4;
        Self { lines: [DelayLine::new(n), DelayLine::new(n)], fs }
    }
}

impl Device for TrackDelay {
    fn id(&self) -> &'static str {
        "native.trackdelay"
    }

    /// Params: delay (ms, 0..500).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let d = (p(params, 0, 0.0).clamp(0.0, 500.0) * 0.001 * self.fs).round();
        for fr in buf.chunks_exact_mut(2) {
            for (c, x) in fr.iter_mut().enumerate() {
                self.lines[c].write(*x);
                *x = if d < 1.0 { *x } else { self.lines[c].read(d + 1.0) };
            }
        }
    }
}

// ---- auto filter -------------------------------------------------------------------------------

struct AutoFilter {
    env: f32,
    f: [Biquad; 2],
    coeffs: Coeffs,
    fs: f32,
}

impl Device for AutoFilter {
    fn id(&self) -> &'static str {
        "native.autofilter"
    }

    /// Params: base cutoff (Hz), range (octaves the envelope opens it), resonance (Q), sensitivity (dB
    /// boost into the follower), mode (0 low-pass, 1 band-pass, 2 high-pass).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let base = p(params, 0, 300.0).clamp(20.0, 12_000.0);
        let range = p(params, 1, 4.0).clamp(0.0, 8.0);
        let q = p(params, 2, 2.0).clamp(0.3, 12.0);
        let sens = db_to_gain(p(params, 3, 0.0).clamp(-12.0, 24.0));
        let mode = p(params, 4, 0.0).round() as i32;
        let (ka, kr) = (smooth_coeff(self.fs, 0.003), smooth_coeff(self.fs, 0.12));
        for (n, fr) in buf.chunks_exact_mut(2).enumerate() {
            let level = (fr[0].abs().max(fr[1].abs()) * sens).min(1.0);
            let k = if level > self.env { ka } else { kr };
            self.env += (level - self.env) * k;
            if n % 16 == 0 {
                let cut = (base * 2f32.powf(range * self.env)).clamp(20.0, self.fs * 0.45);
                let c = match mode {
                    1 => Coeffs::bandpass(self.fs, cut, q),
                    2 => Coeffs::highpass(self.fs, cut, q),
                    _ => Coeffs::lowpass(self.fs, cut, q),
                };
                self.coeffs = c;
            }
            for (ch, x) in fr.iter_mut().enumerate() {
                *x = self.f[ch].process(&self.coeffs, *x);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 44_100.0;

    fn sine(freq: f32, frames: usize, amp: f32) -> Vec<f32> {
        let mut v = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let s = (i as f32 / FS * freq * std::f32::consts::TAU).sin() * amp;
            v.extend([s, s]);
        }
        v
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt()
    }

    fn run(id: &str, params: &[f32], input: &[f32]) -> Vec<f32> {
        let mut d = create(id, FS).unwrap();
        let mut out = input.to_vec();
        for c in out.chunks_mut(1024) {
            d.process(params, c);
        }
        out
    }

    fn crossings(v: &[f32]) -> usize {
        v.chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn every_extra_device_is_finite_on_extreme_input_and_params() {
        let mut noise = Vec::new();
        let mut x = 0x1234_5678u32;
        for _ in 0..8000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            noise.push((x >> 8) as f32 / (1 << 23) as f32 * 2.0 - 1.0);
        }
        for id in [
            "native.comb",
            "native.flanger",
            "native.pitchshift",
            "native.saturator",
            "native.transient",
            "native.trackdelay",
            "native.autofilter",
        ] {
            let info = trakr_core::device_info(id).unwrap_or_else(|| panic!("{id} is in the catalogue"));
            for extreme in [0usize, 1] {
                let params: Vec<f32> =
                    info.params.iter().map(|q| if extreme == 0 { q.min } else { q.max }).collect();
                let out = run(id, &params, &noise);
                assert!(out.iter().all(|s| s.is_finite() && s.abs() < 64.0), "{id} extreme {extreme}");
            }
            let junk = vec![f32::NAN, f32::INFINITY, -1e30];
            assert!(run(id, &junk, &noise).iter().all(|s| s.is_finite()), "{id} survives garbage params");
            assert_eq!(create(id, FS).unwrap().id(), id);
        }
    }

    #[test]
    fn comb_filter_rings_at_its_frequency() {
        let burst: Vec<f32> = (0..4410).flat_map(|i| if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] }).collect();
        let out = run("native.comb", &[441.0, 0.8, 1.0], &burst);
        // an impulse comes back every 100 samples
        let peaks: Vec<usize> =
            out.chunks(2).enumerate().filter(|(_, f)| f[0].abs() > 0.1).map(|(i, _)| i).collect();
        assert!(peaks.len() >= 5, "{peaks:?}");
        assert!(peaks.windows(2).all(|w| (w[1] - w[0]) as i32 == 100), "{peaks:?}");
    }

    #[test]
    fn pitch_shifter_moves_the_pitch() {
        let input = sine(500.0, 22_050, 0.5);
        let up = run("native.pitchshift", &[12.0, 50.0, 1.0], &input);
        let down = run("native.pitchshift", &[-12.0, 50.0, 1.0], &input);
        let base = crossings(&input[22_050..]) as f32;
        let (u, d) = (crossings(&up[22_050..]) as f32, crossings(&down[22_050..]) as f32);
        assert!((u / base - 2.0).abs() < 0.15, "octave up: {u} vs {base}");
        assert!((d / base - 0.5).abs() < 0.1, "octave down: {d} vs {base}");
        assert!(rms(&up[22_050..]) > 0.15, "the shifted signal keeps its level");
        let same = run("native.pitchshift", &[0.0, 50.0, 1.0], &input);
        assert!((crossings(&same[22_050..]) as f32 / base - 1.0).abs() < 0.05);
    }

    #[test]
    fn saturator_adds_harmonics_and_stays_bounded() {
        let input = sine(200.0, 8820, 0.9);
        let out = run("native.saturator", &[24.0, 0.0, 0.0, 1.0], &input);
        assert!(out.iter().all(|s| s.abs() <= 1.05));
        // heavy drive flattens the peaks: the waveform gets closer to a square than the input was
        let clean = run("native.saturator", &[0.0, 0.0, 0.0, 0.0], &input);
        assert!(rms(&out[4410..]) > 0.3, "drive keeps the loudness up");
        assert!(rms(&clean[4410..]) > 0.0);
        let near_top = |v: &[f32]| {
            let peak = v.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            v.iter().filter(|s| s.abs() > peak * 0.9).count()
        };
        let (flat, flat_in) = (near_top(&out[4410..]), near_top(&input[4410..]));
        assert!(flat > flat_in, "clipped shape spends more time near the rails: {flat} vs {flat_in}");
    }

    #[test]
    fn transient_shaper_boosts_onsets_and_cuts_tails() {
        // a decaying burst
        let burst: Vec<f32> = (0..8820)
            .flat_map(|i| {
                let e = (-(i as f32) / 2000.0).exp();
                let s = (i as f32 * 0.2).sin() * 0.5 * e;
                [s, s]
            })
            .collect();
        let punch = run("native.transient", &[1.0, 0.0], &burst);
        let soft = run("native.transient", &[-1.0, 0.0], &burst);
        let early = |v: &[f32]| rms(&v[0..800]);
        assert!(early(&punch) > early(&soft) * 1.15, "attack control: {} vs {}", early(&punch), early(&soft));
        let neutral = run("native.transient", &[0.0, 0.0], &burst);
        assert!(
            neutral.iter().zip(&burst).all(|(a, b)| (a - b).abs() < 1e-4),
            "zero settings pass audio unchanged"
        );
    }

    #[test]
    fn track_delay_shifts_the_signal() {
        let burst: Vec<f32> = (0..2000).flat_map(|i| if i == 0 { [1.0, 0.5] } else { [0.0, 0.0] }).collect();
        let out = run("native.trackdelay", &[10.0], &burst); // 441 samples
        let at = out.chunks(2).position(|f| f[0].abs() > 0.5).unwrap();
        assert_eq!(at, 441);
        assert!((out[441 * 2 + 1] - 0.5).abs() < 1e-6, "both channels are delayed");
        let none = run("native.trackdelay", &[0.0], &burst);
        assert_eq!(none, burst);
    }

    #[test]
    fn auto_filter_opens_with_loud_input() {
        // 4 kHz sine through a low-pass that starts at 300 Hz: loud input opens it, quiet input stays dull
        let loud = run("native.autofilter", &[300.0, 6.0, 1.0, 0.0, 0.0], &sine(4000.0, 22_050, 0.9));
        let quiet = run("native.autofilter", &[300.0, 6.0, 1.0, 0.0, 0.0], &sine(4000.0, 22_050, 0.01));
        let gain = |o: &[f32], amp: f32| rms(&o[10_000..]) / (amp * 0.707);
        assert!(
            gain(&loud, 0.9) > gain(&quiet, 0.01) * 4.0,
            "{} vs {}",
            gain(&loud, 0.9),
            gain(&quiet, 0.01)
        );
    }

    #[test]
    fn flanger_modulates_and_mix_zero_is_dry() {
        let input = sine(1000.0, 22_050, 0.5);
        let dry = run("native.flanger", &[0.3, 2.0, 3.0, 0.5, 0.0], &input);
        assert!(dry.iter().zip(&input).all(|(a, b)| (a - b).abs() < 1e-5));
        let wet = run("native.flanger", &[0.3, 2.0, 3.0, 0.5, 1.0], &input);
        let env: Vec<f32> = wet[10_000..].chunks(2000).map(rms).collect();
        let (lo, hi) = env.iter().fold((f32::MAX, 0.0f32), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(hi > lo * 1.05, "comb sweep makes the level wander: {env:?}");
    }
}
