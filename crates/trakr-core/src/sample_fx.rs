//! Sample processing beyond the basics: loop crossfade, time-stretch, transient detection, dither.

use crate::instrument::{LoopMode, Sample};

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
}
