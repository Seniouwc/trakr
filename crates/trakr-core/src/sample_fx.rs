//! Sample processing beyond the basics: loop crossfade, time-stretch, transient detection, dither.

use crate::instrument::{LoopMode, Sample};
use std::sync::Arc;

/// Simple deterministic noise source for dither.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 // 0..1
    }
}

/// Round `data` to `bits` bits with triangular dither, optionally shaping the quantisation noise upward
/// (a first-order error feedback) where it is less audible. `bits` is clamped to 2..=24.
/// Re-filter `b` so each frame's smoothed spectral envelope equals that of `a` (same length).
fn envelope_match(a: &[f32], b: &[f32]) -> Vec<f32> {
    use crate::fft::fft;
    const N: usize = 2048;
    const HOP: usize = N / 4;
    const LIFTER: usize = 40;
    let hann: Vec<f32> =
        (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
    let envelope = |re: &[f32], im: &[f32]| -> Vec<f32> {
        // log magnitude -> cepstrum -> keep the low quefrencies -> back to a smooth log spectrum
        let mut lr: Vec<f32> = re.iter().zip(im).map(|(r, i)| (r.hypot(*i) + 1e-6).ln()).collect();
        let mut li = vec![0.0f32; N];
        fft(&mut lr, &mut li, true);
        for q in 0..N {
            let keep = q <= LIFTER || q >= N - LIFTER;
            if !keep {
                lr[q] = 0.0;
                li[q] = 0.0;
            } else {
                lr[q] /= N as f32;
                li[q] /= N as f32;
            }
        }
        fft(&mut lr, &mut li, false);
        lr
    };
    let len = a.len().min(b.len());
    let mut out = vec![0.0f32; len + N];
    let mut norm = vec![0.0f32; len + N];
    let mut start = 0usize;
    while start < len {
        let take = |x: &[f32]| -> (Vec<f32>, Vec<f32>) {
            let re: Vec<f32> = (0..N)
                .map(|i| x.get(start + i).copied().filter(|_| start + i < len).unwrap_or(0.0) * hann[i])
                .collect();
            (re, vec![0.0f32; N])
        };
        let (mut ar, mut ai) = take(a);
        let (mut br, mut bi) = take(b);
        fft(&mut ar, &mut ai, false);
        fft(&mut br, &mut bi, false);
        let (ea, eb) = (envelope(&ar, &ai), envelope(&br, &bi));
        for k in 0..N {
            let g = (ea[k] - eb[k]).exp().clamp(0.05, 20.0);
            br[k] *= g;
            bi[k] *= g;
        }
        fft(&mut br, &mut bi, true);
        for i in 0..N {
            out[start + i] += br[i] / N as f32 * hann[i];
            norm[start + i] += hann[i] * hann[i];
        }
        start += HOP;
    }
    (0..len).map(|i| if norm[i] > 1e-3 { out[i] / norm[i] } else { b[i] }).collect()
}

pub fn dither(data: &mut [f32], channels: usize, bits: u32, shaped: bool) {
    let bits = bits.clamp(2, 24);
    let steps = (1u32 << (bits - 1)) as f32;
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let mut err = vec![0.0f32; channels.max(1)];
    for (i, s) in data.iter_mut().enumerate() {
        let ch = i % channels.max(1);
        let tpdf = noise.next() - noise.next(); // -1..1 triangular, in LSB units
        let target = *s * steps - if shaped { err[ch] } else { 0.0 };
        let q = (target + tpdf).round().clamp(-steps, steps - 1.0);
        err[ch] = q - target;
        *s = q / steps;
    }
}

impl Sample {
    /// Make the loop seamless by blending the last `frames` of the loop with the audio just before the
    /// loop start (equal power). Returns false when there is no loop or no room before it.
    pub fn loop_crossfade(&mut self, frames: usize) -> bool {
        if self.loop_mode == LoopMode::Off {
            return false;
        }
        let (ls, le) = (self.loop_start, self.loop_end.min(self.frames()));
        if le <= ls + 4 {
            return false;
        }
        let n = frames.min(ls).min((le - ls) / 2);
        if n < 2 {
            return false;
        }
        let ch = self.channels.max(1) as usize;
        let mut out = (*self.data).clone();
        for i in 0..n {
            let t = (i + 1) as f32 / n as f32;
            let (g_end, g_pre) =
                ((t * std::f32::consts::FRAC_PI_2).cos(), (t * std::f32::consts::FRAC_PI_2).sin());
            for c in 0..ch {
                let end = (le - n + i) * ch + c;
                let pre = (ls - n + i) * ch + c;
                out[end] = self.data[end] * g_end + self.data[pre] * g_pre;
            }
        }
        self.set_data_public(out);
        true
    }

    /// Draw a straight line of sample values from `(from, v0)` to `(to, v1)` (frames, values clamped to
    /// -1..1). `channel` limits it to one channel; `None` writes all channels.
    pub fn draw(&mut self, from: usize, to: usize, v0: f32, v1: f32, channel: Option<usize>) {
        let ch = self.channels.max(1) as usize;
        let frames = self.frames();
        if frames == 0 {
            return;
        }
        let (mut a, mut b, mut va, mut vb) = (from, to, v0, v1);
        if a > b {
            std::mem::swap(&mut a, &mut b);
            std::mem::swap(&mut va, &mut vb);
        }
        let b = b.min(frames - 1);
        if a > b {
            return;
        }
        let (va, vb) = (va.clamp(-1.0, 1.0), vb.clamp(-1.0, 1.0));
        let data = Arc::make_mut(&mut self.data);
        let span = (b - a).max(1) as f32;
        for f in a..=b {
            let v = va + (vb - va) * ((f - a) as f32 / span);
            match channel {
                Some(c) if c < ch => data[f * ch + c] = v,
                Some(_) => {}
                None => data[f * ch..f * ch + ch].fill(v),
            }
        }
    }

    /// Shift the pitch by `semitones` without changing the length (tape-style resample, then WSOLA
    /// stretch back). Formants move with the pitch.
    pub fn pitch_shift_keep_length(&mut self, semitones: f32) {
        let frames = self.frames();
        if frames == 0 || semitones.abs() < 1e-3 {
            return;
        }
        let ratio = 2f64.powf(semitones as f64 / 12.0);
        let (ls, le) = (self.loop_start, self.loop_end);
        self.pitch_shift(semitones);
        // the resampled sample is `frames / ratio` long; stretch it back
        self.time_stretch(ratio);
        // loop points and slices are not meaningful after a stretch of a stretch: restore the originals
        let n = self.frames();
        self.loop_start = ls.min(n);
        self.loop_end = le.min(n);
        self.slices.retain(|&s| s < n);
    }

    /// Shift the pitch by `semitones` without changing the length and keep the spectral envelope (the
    /// vowel colour of a voice or instrument body) where it was: the pitch-shifted signal is re-filtered
    /// so its smoothed spectrum matches the original's, frame by frame (cepstral envelope, overlap-add).
    pub fn pitch_shift_formant(&mut self, semitones: f32) {
        let frames = self.frames();
        if frames < 4096 || semitones.abs() < 1e-3 {
            return self.pitch_shift_keep_length(semitones);
        }
        let ch = self.channels.max(1) as usize;
        let original = self.data.clone();
        let mut shifted = self.clone();
        shifted.pitch_shift_keep_length(semitones);
        let mut out = vec![0.0f32; frames * ch];
        for c in 0..ch {
            let a: Vec<f32> = (0..frames).map(|f| original[f * ch + c]).collect();
            let b: Vec<f32> =
                (0..frames).map(|f| shifted.data.get(f * ch + c).copied().unwrap_or(0.0)).collect();
            let y = envelope_match(&a, &b);
            for (f, v) in y.into_iter().enumerate() {
                out[f * ch + c] = v.clamp(-1.0, 1.0);
            }
        }
        self.data = Arc::new(out);
    }

    /// Change the length by `factor` (2.0 = twice as long) without changing the pitch (WSOLA). Loop
    /// points and slices move with the audio.
    pub fn time_stretch(&mut self, factor: f64) {
        let factor = factor.clamp(0.1, 10.0);
        let ch = self.channels.max(1) as usize;
        let frames = self.frames();
        if frames < 64 || (factor - 1.0).abs() < 1e-3 {
            return;
        }
        let win = ((self.sample_rate as f64 * 0.04) as usize).clamp(256, 4096) & !1; // ~40 ms
        let hop = win / 2;
        let search = hop / 2;
        let mono: Vec<f32> =
            (0..frames).map(|f| (0..ch).map(|c| self.data[f * ch + c]).sum::<f32>() / ch as f32).collect();
        let out_frames = ((frames as f64) * factor) as usize;
        let hann: Vec<f32> = (0..win)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / win as f32).cos())
            .collect();
        let mut out = vec![0.0f32; (out_frames + win) * ch];
        let mut norm = vec![0.0f32; out_frames + win];
        let mut prev_pos: Option<usize> = None;
        let mut o = 0usize;
        while o < out_frames {
            let target = (o as f64 / factor) as usize;
            let mut pos = target.min(frames.saturating_sub(win));
            if let Some(p) = prev_pos {
                // find the offset whose waveform best continues the previous window's natural successor
                let natural = (p + hop).min(frames.saturating_sub(win));
                let lo = target.saturating_sub(search);
                let hi = (target + search).min(frames.saturating_sub(win));
                let mut best = (f32::MIN, pos);
                let step = ((hi.saturating_sub(lo)) / 64).max(1);
                let mut cand = lo;
                while cand <= hi {
                    let mut corr = 0.0f32;
                    for i in (0..hop).step_by(4) {
                        corr += mono[natural + i] * mono[cand + i];
                    }
                    if corr > best.0 {
                        best = (corr, cand);
                    }
                    cand += step;
                }
                pos = best.1;
            }
            for i in 0..win {
                if pos + i >= frames {
                    break;
                }
                let w = hann[i];
                norm[o + i] += w;
                for c in 0..ch {
                    out[(o + i) * ch + c] += self.data[(pos + i) * ch + c] * w;
                }
            }
            prev_pos = Some(pos);
            o += hop;
        }
        let mut result = Vec::with_capacity(out_frames * ch);
        for f in 0..out_frames {
            let n = norm[f].max(1e-3);
            for c in 0..ch {
                result.push(out[f * ch + c] / n);
            }
        }
        let scale = |p: usize| ((p as f64) * factor).round() as usize;
        self.loop_start = scale(self.loop_start);
        self.loop_end = scale(self.loop_end);
        self.slices = self.slices.iter().map(|&p| scale(p)).collect();
        self.set_data_public(result);
    }

    /// Frames where a new sound starts: the energy jumps well above its recent average.
    /// `sensitivity` 0..=1 (higher finds more). Markers are at least 40 ms apart.
    pub fn detect_transients(&self, sensitivity: f32) -> Vec<usize> {
        let ch = self.channels.max(1) as usize;
        let frames = self.frames();
        let hop = ((self.sample_rate as usize) / 400).max(32); // ~2.5 ms
        let n = frames / hop;
        if n < 8 {
            return Vec::new();
        }
        let energy: Vec<f32> = (0..n)
            .map(|b| {
                let mut e = 0.0;
                for f in b * hop..(b + 1) * hop {
                    let m: f32 = (0..ch).map(|c| self.data[f * ch + c]).sum::<f32>() / ch as f32;
                    e += m * m;
                }
                (e / hop as f32).sqrt()
            })
            .collect();
        let ratio = 3.5 - 2.5 * sensitivity.clamp(0.0, 1.0); // how far above the average counts as an attack
        let min_gap = (self.sample_rate as usize * 40 / 1000) / hop + 1;
        let floor = energy.iter().fold(0.0f32, |m, e| m.max(*e)) * 0.02;
        let mut out: Vec<usize> = Vec::new();
        let mut last = usize::MAX;
        for b in 4..n {
            let avg: f32 = energy[b - 4..b].iter().sum::<f32>() / 4.0;
            if energy[b] > floor
                && energy[b] > avg * ratio + 1e-6
                && (last == usize::MAX || b - last >= min_gap)
            {
                // walk back to the quietest point just before the attack
                let mut start = b;
                while start > 0 && b - start < 4 && energy[start - 1] < energy[start] {
                    start -= 1;
                }
                out.push(start * hop);
                last = b;
            }
        }
        out.retain(|&p| p > 0);
        out
    }

    /// Replace the slice markers with detected transients. Returns how many were placed.
    pub fn auto_slice(&mut self, sensitivity: f32) -> usize {
        self.slices = self.detect_transients(sensitivity);
        self.slices.len()
    }

    /// Round the audio to `bits` bits with dither (a destructive bit-depth reduction).
    pub fn reduce_bit_depth(&mut self, bits: u32, shaped: bool) {
        let mut d = (*self.data).clone();
        dither(&mut d, self.channels as usize, bits, shaped);
        self.set_data_public(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin() * 0.8)
            .collect()
    }

    #[test]
    fn loop_crossfade_removes_the_jump_at_the_loop_point() {
        // a ramp loop jumps from high to low when it wraps; after the crossfade it does not
        let data: Vec<f32> = (0..2000).map(|i| i as f32 / 2000.0).collect();
        let mut s = Sample::new("ramp", 44_100, 1, data);
        s.loop_mode = LoopMode::Forward;
        s.loop_start = 500;
        s.loop_end = 1500;
        let jump = |s: &Sample| (s.data[s.loop_end - 1] - s.data[s.loop_start]).abs();
        let before = jump(&s);
        assert!(s.loop_crossfade(300));
        assert!(jump(&s) < before * 0.2, "{before} -> {}", jump(&s));
        // nothing outside the crossfaded region changed
        assert_eq!(s.data[100], 100.0 / 2000.0);
        assert_eq!(s.data[1600], 1600.0 / 2000.0);
        // no loop or no room: refused
        let mut off = Sample::new("x", 44_100, 1, vec![0.0; 100]);
        assert!(!off.loop_crossfade(10));
        off.loop_mode = LoopMode::Forward;
        off.loop_start = 0;
        off.loop_end = 100;
        assert!(!off.loop_crossfade(10), "nothing before the loop start to fade in");
    }

    #[test]
    fn time_stretch_changes_length_not_pitch() {
        let rate = 22_050;
        let mut s = Sample::new("a", rate, 1, sine(441.0, rate, 1.0));
        s.slices = vec![10_000];
        s.time_stretch(2.0);
        let len = s.frames() as f32 / rate as f32;
        assert!((len - 2.0).abs() < 0.05, "{len}");
        assert_eq!(s.slices, vec![20_000]);
        // still about 441 Hz: count zero crossings in the middle second
        let mid = &s.data[rate as usize / 2..rate as usize / 2 + rate as usize];
        let crossings = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((crossings as i32 - 441).abs() <= 12, "{crossings} cycles");
        assert!(s.data.iter().all(|v| v.is_finite()));
        // shorter, stereo and degenerate inputs
        let mut st =
            Sample::new("s", rate, 2, sine(300.0, rate, 0.5).into_iter().flat_map(|v| [v, -v]).collect());
        st.time_stretch(0.5);
        assert!((st.frames() as f32 / rate as f32 - 0.25).abs() < 0.03);
        let mut tiny = Sample::new("t", rate, 1, vec![0.1; 10]);
        tiny.time_stretch(3.0);
        assert_eq!(tiny.frames(), 10);
    }

    #[test]
    fn transients_are_found_at_the_hits() {
        let rate = 44_100;
        let mut data = vec![0.0f32; rate as usize * 2];
        for hit in [4000usize, 24_000, 50_000, 70_000] {
            for i in 0..3000 {
                data[hit + i] = (i as f32 * 0.3).sin() * (-(i as f32) / 600.0).exp();
            }
        }
        let mut s = Sample::new("drums", rate, 1, data);
        let found = s.detect_transients(0.5);
        assert_eq!(found.len(), 4, "{found:?}");
        for (f, want) in found.iter().zip([4000usize, 24_000, 50_000, 70_000]) {
            assert!(f.abs_diff(want) < 400, "{f} vs {want}");
        }
        assert_eq!(s.auto_slice(0.5), 4);
        assert_eq!(s.slices.len(), 4);
        let silent = Sample::new("z", rate, 1, vec![0.0; 20_000]);
        assert!(silent.detect_transients(1.0).is_empty());
    }

    #[test]
    fn dither_quantises_and_keeps_the_signal() {
        let mut d: Vec<f32> = sine(1000.0, 44_100, 0.2);
        let orig = d.clone();
        dither(&mut d, 1, 8, false);
        let step = 1.0 / 128.0;
        let err: f32 = d.iter().zip(&orig).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(err <= step * 1.6, "{err}");
        assert!(d.iter().all(|v| (v * 128.0 - (v * 128.0).round()).abs() < 1e-3), "on the 8-bit grid");
        // silence stays (nearly) silent: dither noise is at most one step
        let mut z = vec![0.0f32; 1000];
        dither(&mut z, 1, 16, true);
        assert!(z.iter().all(|v| v.abs() <= 2.0 / 32768.0));
        let mut s = Sample::new("s", 44_100, 2, vec![0.3; 200]);
        s.reduce_bit_depth(4, true);
        assert!(s.data.iter().all(|v| (v * 8.0 - (v * 8.0).round()).abs() < 1e-3));
    }

    #[test]
    fn drawing_writes_a_line_of_values() {
        let mut s = Sample::new("d", 44_100, 2, vec![0.0; 200]);
        s.draw(10, 20, -1.0, 1.0, None);
        assert_eq!(s.data[10 * 2], -1.0);
        assert_eq!(s.data[20 * 2 + 1], 1.0);
        assert!((s.data[15 * 2] - 0.0).abs() < 1e-6, "linear in between");
        s.draw(50, 40, 0.5, 0.0, Some(1)); // reversed endpoints and one channel only
        assert_eq!(s.data[40 * 2 + 1], 0.0);
        assert_eq!(s.data[50 * 2 + 1], 0.5);
        assert_eq!(s.data[45 * 2], 0.0, "the other channel is untouched");
        s.draw(90, 5000, 2.0, 2.0, None); // past the end and out of range values are clamped
        assert_eq!(s.data[99 * 2], 1.0);
        let mut empty = Sample::new("e", 44_100, 1, vec![]);
        empty.draw(0, 10, 1.0, 1.0, None);
        assert!(empty.data.is_empty());
    }

    #[test]
    fn pitch_shift_keeps_the_length() {
        let data: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut s = Sample::new("p", 44_100, 1, data);
        s.pitch_shift_keep_length(7.0);
        let diff = (s.frames() as i64 - 44_100).abs();
        assert!(diff < 3000, "length stays close to the original: {}", s.frames());
        let crossings = |v: &[f32]| v.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        let before = 44_100.0 * 0.05 / std::f32::consts::TAU;
        let ratio = crossings(&s.data) / before;
        assert!((ratio - 2f32.powf(7.0 / 12.0)).abs() < 0.08, "pitch moved by a fifth: {ratio}");
        let mut same = Sample::new("p", 44_100, 1, vec![0.1; 1000]);
        same.pitch_shift_keep_length(0.0);
        assert_eq!(same.frames(), 1000);
    }

    #[test]
    fn formant_shift_keeps_the_spectral_envelope() {
        use crate::fft::magnitude_spectrum;
        // a "vowel": harmonics of 120 Hz shaped by one broad resonance at 1 kHz
        let rate = 22_050.0f32;
        let n = 22_050;
        let data: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                (1..=40)
                    .map(|h| {
                        let f = 120.0 * h as f32;
                        let amp = (-((f - 1000.0) / 350.0).powi(2)).exp();
                        amp * (std::f32::consts::TAU * f * t).sin()
                    })
                    .sum::<f32>()
                    * 0.1
            })
            .collect();
        let centroid = |v: &[f32]| {
            let spec = magnitude_spectrum(&v[v.len() / 4..v.len() / 4 + 8192], 8192);
            let (mut num, mut den) = (0.0f32, 0.0f32);
            for (k, m) in spec.iter().enumerate().take(1200) {
                let f = k as f32 * rate / 8192.0;
                num += f * m * m;
                den += m * m;
            }
            num / den.max(1e-9)
        };
        let original = Sample::new("v", 22_050, 1, data);
        let base = centroid(&original.data);
        let mut plain = original.clone();
        plain.pitch_shift_keep_length(7.0);
        let mut kept = original.clone();
        kept.pitch_shift_formant(7.0);
        let (c_plain, c_kept) = (centroid(&plain.data), centroid(&kept.data));
        assert!(c_plain > base * 1.2, "a plain shift drags the resonance up: {base} -> {c_plain}");
        assert!(
            (c_kept - base).abs() < (c_plain - base).abs() * 0.6,
            "formant-preserving shift stays nearer the original: base {base}, plain {c_plain}, kept {c_kept}"
        );
        // still the same length, finite, and short or silent input is handled
        assert_eq!(kept.frames(), original.frames());
        assert!(kept.data.iter().all(|v| v.is_finite()));
        let mut tiny = Sample::new("t", 22_050, 1, vec![0.1; 100]);
        tiny.pitch_shift_formant(5.0);
        assert!(tiny.frames() > 0);
        let mut silent = Sample::new("s", 22_050, 2, vec![0.0; 20_000]);
        silent.pitch_shift_formant(-5.0);
        assert!(silent.data.iter().all(|v| v.abs() < 1e-3));
    }
}
