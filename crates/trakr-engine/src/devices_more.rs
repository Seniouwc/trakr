//! More native devices: sidechain and bus compressors, multiband compressor, multitap delay,
//! repeater, cabinet, DC offset and the hydra modulator.

use crate::devices::Device;
use crate::dsp::{db_to_gain, gain_to_db, sine_phase, smooth_coeff, Biquad, Coeffs, DelayLine};

fn p(params: &[f32], i: usize, default: f32) -> f32 {
    params.get(i).copied().filter(|v| v.is_finite()).unwrap_or(default)
}

pub(crate) fn create(id: &str, fs: f32) -> Option<Box<dyn Device>> {
    Some(match id {
        "native.sidechain" => Box::new(SidechainComp { env_db: -90.0, fs }),
        "native.buscomp" => Box::new(BusComp { power: 0.0, gain_db: 0.0, fs }),
        "native.multiband" => Box::new(Multiband::new(fs)),
        "native.multitap" => Box::new(Multitap::new(fs)),
        "native.repeater" => Box::new(Repeater::new(fs)),
        "native.cabinet" => Box::new(Cabinet {
            hp: [Biquad::default(); 2],
            lp: [Biquad::default(); 2],
            pk: [Biquad::default(); 2],
            sh: [Biquad::default(); 2],
            fs,
        }),
        "native.dcoffset" => Box::new(DcOffset { x1: [0.0; 2], y1: [0.0; 2] }),
        "native.hydra" => Box::new(Hydra { phase: 0.0, held: 0.0, rng: 0x1357_9BDF, value: 0.0, fs }),
        _ => return None,
    })
}

// ---- sidechain compressor ------------------------------------------------------------------

struct SidechainComp {
    env_db: f32,
    fs: f32,
}

impl Device for SidechainComp {
    fn id(&self) -> &'static str {
        "native.sidechain"
    }

    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        self.process_sc(params, buf, None);
    }

    /// Params: source track (0 = the device's own signal), threshold, ratio, attack, release, makeup, mix.
    fn process_sc(&mut self, params: &[f32], buf: &mut [f32], sidechain: Option<&[f32]>) {
        let (thr, ratio) = (p(params, 1, -24.0), p(params, 2, 4.0).max(1.0));
        let atk = smooth_coeff(self.fs, p(params, 3, 5.0).max(0.1) * 0.001);
        let rel = smooth_coeff(self.fs, p(params, 4, 150.0).max(5.0) * 0.001);
        let makeup = db_to_gain(p(params, 5, 0.0));
        let mix = p(params, 6, 1.0).clamp(0.0, 1.0);
        let sidechain = sidechain.filter(|s| s.len() >= buf.len());
        for (i, fr) in buf.chunks_exact_mut(2).enumerate() {
            let key = match sidechain {
                Some(s) => s[i * 2].abs().max(s[i * 2 + 1].abs()),
                None => fr[0].abs().max(fr[1].abs()),
            };
            let level = gain_to_db(key);
            let k = if level > self.env_db { atk } else { rel };
            self.env_db += (level - self.env_db) * k;
            let over = (self.env_db - thr).max(0.0);
            let g = db_to_gain(-over * (1.0 - 1.0 / ratio)) * makeup;
            let g = 1.0 - mix + mix * g;
            fr[0] *= g;
            fr[1] *= g;
        }
    }
}

// ---- bus compressor ------------------------------------------------------------------------

struct BusComp {
    /// Smoothed signal power.
    power: f32,
    gain_db: f32,
    fs: f32,
}

impl Device for BusComp {
    fn id(&self) -> &'static str {
        "native.buscomp"
    }

    /// Params: threshold, ratio, attack, release, knee, makeup, mix. RMS detection, soft knee.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (thr, ratio) = (p(params, 0, -18.0), p(params, 1, 2.0).max(1.0));
        let atk = smooth_coeff(self.fs, p(params, 2, 10.0).max(0.1) * 0.001);
        let rel = smooth_coeff(self.fs, p(params, 3, 200.0).max(5.0) * 0.001);
        let knee = p(params, 4, 6.0).max(0.0);
        let makeup = db_to_gain(p(params, 5, 0.0));
        let mix = p(params, 6, 1.0).clamp(0.0, 1.0);
        let det = smooth_coeff(self.fs, 0.01);
        for fr in buf.chunks_exact_mut(2) {
            let sq = 0.5 * (fr[0] * fr[0] + fr[1] * fr[1]);
            self.power += (sq - self.power) * det;
            let level = 10.0 * (self.power.max(1e-12)).log10();
            // soft-knee gain computer
            let over = level - thr;
            let reduction = if 2.0 * over < -knee {
                0.0
            } else if 2.0 * over.abs() <= knee {
                (1.0 - 1.0 / ratio) * (over + knee / 2.0).powi(2) / (2.0 * knee.max(1e-6))
            } else {
                (1.0 - 1.0 / ratio) * over
            };
            let target = -reduction;
            let k = if target < self.gain_db { atk } else { rel };
            self.gain_db += (target - self.gain_db) * k;
            let g = db_to_gain(self.gain_db) * makeup;
            let g = 1.0 - mix + mix * g;
            fr[0] *= g;
            fr[1] *= g;
        }
    }
}

// ---- multiband compressor ------------------------------------------------------------------

struct Multiband {
    /// Linkwitz-Riley crossovers: two cascaded stages per crossover, filter type and channel.
    lp1: [[Biquad; 2]; 2],
    hp1: [[Biquad; 2]; 2],
    lp2: [[Biquad; 2]; 2],
    hp2: [[Biquad; 2]; 2],
    env_db: [f32; 3],
    fs: f32,
}

impl Multiband {
    fn new(fs: f32) -> Self {
        Self {
            lp1: Default::default(),
            hp1: Default::default(),
            lp2: Default::default(),
            hp2: Default::default(),
            env_db: [-90.0; 3],
            fs,
        }
    }
}

impl Device for Multiband {
    fn id(&self) -> &'static str {
        "native.multiband"
    }

    /// Params: crossover 1, crossover 2, thresholds low/mid/high, ratio, attack, release, makeup.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let x1 = p(params, 0, 200.0).clamp(40.0, 2000.0);
        let x2 = p(params, 1, 2500.0).clamp(x1 * 1.5, 16_000.0);
        let thr = [p(params, 2, -24.0), p(params, 3, -24.0), p(params, 4, -24.0)];
        let ratio = p(params, 5, 3.0).max(1.0);
        let atk = smooth_coeff(self.fs, p(params, 6, 10.0).max(0.1) * 0.001);
        let rel = smooth_coeff(self.fs, p(params, 7, 150.0).max(5.0) * 0.001);
        let makeup = db_to_gain(p(params, 8, 0.0));
        let c1 = Coeffs::lowpass(self.fs, x1, std::f32::consts::FRAC_1_SQRT_2);
        let x2 = x2.min(self.fs * 0.45);
        let c2 = Coeffs::lowpass(self.fs, x2, std::f32::consts::FRAC_1_SQRT_2);
        let h1 = Coeffs::highpass(self.fs, x1, std::f32::consts::FRAC_1_SQRT_2);
        let h2 = Coeffs::highpass(self.fs, x2, std::f32::consts::FRAC_1_SQRT_2);
        for fr in buf.chunks_exact_mut(2) {
            let mut bands = [[0.0f32; 2]; 3];
            for ch in 0..2 {
                let x = fr[ch];
                let a = self.lp1[0][ch].process(&c1, x);
                let low = self.lp1[1][ch].process(&c1, a);
                let a = self.hp1[0][ch].process(&h1, x);
                let rest = self.hp1[1][ch].process(&h1, a);
                let a = self.lp2[0][ch].process(&c2, rest);
                let mid = self.lp2[1][ch].process(&c2, a);
                let a = self.hp2[0][ch].process(&h2, rest);
                let high = self.hp2[1][ch].process(&h2, a);
                bands[0][ch] = low;
                bands[1][ch] = mid;
                bands[2][ch] = high;
            }
            let mut out = [0.0f32; 2];
            for b in 0..3 {
                let level = gain_to_db(bands[b][0].abs().max(bands[b][1].abs()));
                let k = if level > self.env_db[b] { atk } else { rel };
                self.env_db[b] += (level - self.env_db[b]) * k;
                let over = (self.env_db[b] - thr[b]).max(0.0);
                let g = db_to_gain(-over * (1.0 - 1.0 / ratio));
                out[0] += bands[b][0] * g;
                out[1] += bands[b][1] * g;
            }
            fr[0] = out[0] * makeup;
            fr[1] = out[1] * makeup;
        }
    }
}

// ---- multitap delay ------------------------------------------------------------------------

struct Multitap {
    l: DelayLine,
    r: DelayLine,
    fs: f32,
}

impl Multitap {
    fn new(fs: f32) -> Self {
        let n = (fs * 1.6) as usize;
        Self { l: DelayLine::new(n), r: DelayLine::new(n), fs }
    }
}

impl Device for Multitap {
    fn id(&self) -> &'static str {
        "native.multitap"
    }

    /// Params: three tap times (ms), three tap levels, feedback from the last tap, mix.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let max = (self.l.len() - 3) as f32;
        let t = [0, 1, 2].map(|i| (p(params, i, 200.0 * (i + 1) as f32) * 0.001 * self.fs).clamp(1.0, max));
        let lv = [p(params, 3, 0.6), p(params, 4, 0.4), p(params, 5, 0.25)];
        let fb = p(params, 6, 0.2).clamp(0.0, 0.85);
        let mix = p(params, 7, 0.4).clamp(0.0, 1.0);
        for fr in buf.chunks_exact_mut(2) {
            let (mut wl, mut wr) = (0.0, 0.0);
            for i in 0..3 {
                // alternate taps between the channels for width
                let (a, b) = (self.l.read(t[i]), self.r.read(t[i]));
                if i % 2 == 0 {
                    wl += a * lv[i];
                    wr += b * lv[i] * 0.6;
                } else {
                    wl += a * lv[i] * 0.6;
                    wr += b * lv[i];
                }
            }
            let (fl, frr) = (self.l.read(t[2]), self.r.read(t[2]));
            self.l.write(fr[0] + fl * fb);
            self.r.write(fr[1] + frr * fb);
            fr[0] = fr[0] * (1.0 - mix) + wl * mix;
            fr[1] = fr[1] * (1.0 - mix) + wr * mix;
        }
    }
}

// ---- repeater ------------------------------------------------------------------------------

struct Repeater {
    hist: Vec<f32>,
    frozen: Vec<f32>,
    pos: usize,
    play: usize,
    holding: bool,
    len: usize,
}

impl Repeater {
    fn new(fs: f32) -> Self {
        let n = (fs * 1.05) as usize + 4;
        Self { hist: vec![0.0; n * 2], frozen: vec![0.0; n * 2], pos: 0, play: 0, holding: false, len: 0 }
    }
}

impl Device for Repeater {
    fn id(&self) -> &'static str {
        "native.repeater"
    }

    /// Params: slice length (ms), hold (0/1), mix. While hold is on, the last slice loops.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let frames_cap = self.hist.len() / 2;
        let fs_ms = frames_cap as f32 / 1050.0;
        let want = ((p(params, 0, 250.0).clamp(5.0, 1000.0)) * fs_ms) as usize;
        let hold = p(params, 1, 0.0) >= 0.5;
        let mix = p(params, 2, 1.0).clamp(0.0, 1.0);
        if hold && !self.holding {
            self.len = want.clamp(2, frames_cap - 1);
            // copy the most recent `len` frames, oldest first
            for i in 0..self.len {
                let src = (self.pos + frames_cap - self.len + i) % frames_cap;
                self.frozen[i * 2] = self.hist[src * 2];
                self.frozen[i * 2 + 1] = self.hist[src * 2 + 1];
            }
            self.play = 0;
        }
        self.holding = hold;
        for fr in buf.chunks_exact_mut(2) {
            self.hist[self.pos * 2] = fr[0];
            self.hist[self.pos * 2 + 1] = fr[1];
            self.pos = (self.pos + 1) % frames_cap;
            if hold {
                let (l, r) = (self.frozen[self.play * 2], self.frozen[self.play * 2 + 1]);
                self.play = (self.play + 1) % self.len.max(1);
                fr[0] = fr[0] * (1.0 - mix) + l * mix;
                fr[1] = fr[1] * (1.0 - mix) + r * mix;
            }
        }
    }
}

// ---- cabinet -------------------------------------------------------------------------------

struct Cabinet {
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    pk: [Biquad; 2],
    sh: [Biquad; 2],
    fs: f32,
}

impl Device for Cabinet {
    fn id(&self) -> &'static str {
        "native.cabinet"
    }

    /// Params: type (0 combo, 1 stack, 2 small, 3 radio), mix. A filter model of a speaker, not a measured one.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let (hp, lp, pk_f, pk_g, sh_g): (f32, f32, f32, f32, f32) = match p(params, 0, 0.0).round() as i32 {
            1 => (70.0, 4500.0, 1800.0, 4.0, 2.0),
            2 => (150.0, 6500.0, 3000.0, 2.0, 0.0),
            3 => (400.0, 3500.0, 1500.0, 3.0, -3.0),
            _ => (90.0, 5500.0, 2200.0, 3.0, 1.0),
        };
        let mix = p(params, 1, 1.0).clamp(0.0, 1.0);
        let chp = Coeffs::highpass(self.fs, hp, 0.8);
        let clp = Coeffs::lowpass(self.fs, lp.min(self.fs * 0.45), 0.9);
        let cpk = Coeffs::peak(self.fs, pk_f, 1.0, pk_g);
        let csh = Coeffs::low_shelf(self.fs, 200.0, sh_g);
        for fr in buf.chunks_exact_mut(2) {
            #[allow(clippy::needless_range_loop)]
            for ch in 0..2 {
                let x = fr[ch];
                let y = self.hp[ch].process(&chp, x);
                let y = self.sh[ch].process(&csh, y);
                let y = self.pk[ch].process(&cpk, y);
                let y = self.lp[ch].process(&clp, y);
                fr[ch] = x * (1.0 - mix) + y * mix;
            }
        }
    }
}

// ---- DC offset -----------------------------------------------------------------------------

struct DcOffset {
    x1: [f32; 2],
    y1: [f32; 2],
}

impl Device for DcOffset {
    fn id(&self) -> &'static str {
        "native.dcoffset"
    }

    /// Params: offset added to the signal, remove (0/1: high-pass the DC away first).
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let off = p(params, 0, 0.0).clamp(-1.0, 1.0);
        let remove = p(params, 1, 0.0) >= 0.5;
        for fr in buf.chunks_exact_mut(2) {
            #[allow(clippy::needless_range_loop)]
            for ch in 0..2 {
                let x = fr[ch];
                let y = if remove {
                    // one-pole DC blocker
                    let y = x - self.x1[ch] + 0.995 * self.y1[ch];
                    self.x1[ch] = x;
                    self.y1[ch] = y;
                    y
                } else {
                    x
                };
                fr[ch] = y + off;
            }
        }
    }
}

// ---- hydra ---------------------------------------------------------------------------------

/// One LFO that moves up to three parameters, each with its own depth.
struct Hydra {
    phase: f32,
    held: f32,
    rng: u32,
    value: f32,
    fs: f32,
}

impl Device for Hydra {
    fn id(&self) -> &'static str {
        "native.hydra"
    }

    /// Params: shape, rate, then (target device, target parameter, depth) three times.
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let shape = p(params, 0, 0.0).round() as i32;
        let rate = p(params, 1, 1.0).clamp(0.05, 20.0);
        let before = self.phase;
        self.phase = (self.phase + rate * (buf.len() / 2) as f32 / self.fs).fract();
        if self.phase < before {
            self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.held = (self.rng >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
        }
        self.value = match shape {
            1 => 1.0 - 4.0 * (self.phase - 0.5).abs(),
            2 => self.phase * 2.0 - 1.0,
            3 => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            4 => self.held,
            _ => sine_phase(self.phase),
        };
    }

    fn modulation(&self) -> Option<f32> {
        Some(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trakr_core::device_info;

    const FS: f32 = 44_100.0;

    fn dev(id: &str, set: &[(usize, f32)]) -> (Box<dyn Device>, Vec<f32>) {
        let mut params = device_info(id).unwrap().defaults();
        for &(i, v) in set {
            params[i] = v;
        }
        (crate::devices::create(id, FS as u32).unwrap(), params)
    }

    fn sine(freq: f32, frames: usize, amp: f32) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (i as f32 / FS * freq * std::f32::consts::TAU).sin() * amp;
                [v, v]
            })
            .collect()
    }

    fn peak(v: &[f32]) -> f32 {
        v.iter().fold(0.0, |m, s| s.abs().max(m))
    }

    /// Process in 512-frame blocks like the engine and return everything.
    fn run(d: &mut dyn Device, params: &[f32], mut input: Vec<f32>) -> Vec<f32> {
        for chunk in input.chunks_mut(1024) {
            d.process(params, chunk);
        }
        input
    }

    #[test]
    fn bus_compressor_squeezes_loud_signals_and_leaves_quiet_ones() {
        let (mut d, p) = dev("native.buscomp", &[(0, -18.0), (1, 10.0), (2, 1.0), (4, 0.0)]);
        let loud = run(d.as_mut(), &p, sine(440.0, 20_000, 0.9));
        assert!(peak(&loud[30_000..]) < 0.9 * 0.6, "{}", peak(&loud[30_000..]));
        let (mut d, p) = dev("native.buscomp", &[(0, -18.0), (1, 10.0)]);
        let quiet = run(d.as_mut(), &p, sine(440.0, 20_000, 0.02));
        assert!((peak(&quiet[10_000..]) - 0.02).abs() < 0.002);
    }

    #[test]
    fn multiband_only_compresses_the_band_over_its_threshold() {
        let params = |low: f32| [(2, low), (3, 0.0), (4, 0.0), (5, 20.0), (6, 0.5)];
        let (mut d, p) = dev("native.multiband", &params(-40.0));
        let low = run(d.as_mut(), &p, sine(80.0, 30_000, 0.5));
        let (mut d, p) = dev("native.multiband", &params(-40.0));
        let high = run(d.as_mut(), &p, sine(6000.0, 30_000, 0.5));
        assert!(peak(&low[40_000..]) < 0.2, "low band squeezed: {}", peak(&low[40_000..]));
        assert!(peak(&high[40_000..]) > 0.45, "high band untouched: {}", peak(&high[40_000..]));
        // a neutral setting reconstructs the input
        let (mut d, p) = dev("native.multiband", &[(2, 0.0), (3, 0.0), (4, 0.0)]);
        let out = run(d.as_mut(), &p, sine(1000.0, 20_000, 0.5));
        assert!((peak(&out[20_000..]) - 0.5).abs() < 0.03, "{}", peak(&out[20_000..]));
    }

    #[test]
    fn multitap_echoes_at_each_tap_time() {
        let (mut d, p) = dev("native.multitap", &[(0, 100.0), (1, 200.0), (2, 300.0), (6, 0.0), (7, 1.0)]);
        let mut input = vec![0.0; 2 * 20_000];
        input[0] = 1.0;
        input[1] = 1.0;
        let out = run(d.as_mut(), &p, input);
        for (ms, level) in [(100.0, 0.6), (200.0, 0.4), (300.0, 0.25)] {
            let at = (ms * 0.001 * FS) as usize;
            let v = (at - 3..at + 3).map(|i| out[i * 2]).fold(0.0f32, f32::max);
            assert!(v > level * 0.4, "tap {ms}: {v}");
        }
    }

    #[test]
    fn repeater_loops_the_last_slice_while_held() {
        let (mut d, mut p) = dev("native.repeater", &[(0, 100.0), (1, 0.0)]);
        // a 100 ms ramp is the most recent audio when hold switches on
        let ramp: Vec<f32> = (0..4410).flat_map(|i| [i as f32 / 4410.0; 2]).collect();
        let _ = run(d.as_mut(), &p, ramp);
        p[1] = 1.0;
        let out = run(d.as_mut(), &p, vec![0.0; 2 * 9000]);
        // the ramp comes back twice, starting from near 0 each time
        assert!(out[0] < 0.05 && out[2 * 4000] > 0.8, "{} {}", out[0], out[2 * 4000]);
        assert!(out[2 * 4410] < 0.05, "second pass restarts: {}", out[2 * 4410]);
        p[1] = 0.0;
        let after = run(d.as_mut(), &p, vec![0.0; 2 * 100]);
        assert_eq!(peak(&after), 0.0);
    }

    #[test]
    fn cabinet_rolls_off_the_top_and_dc_offset_shifts_and_removes() {
        let (mut d, p) = dev("native.cabinet", &[]);
        let hi = run(d.as_mut(), &p, sine(12_000.0, 10_000, 0.5));
        let (mut d, p) = dev("native.cabinet", &[]);
        let mid = run(d.as_mut(), &p, sine(1000.0, 10_000, 0.5));
        assert!(peak(&hi[5000..]) < 0.1 && peak(&mid[5000..]) > 0.4);
        for ty in 0..4 {
            let (mut d, p) = dev("native.cabinet", &[(0, ty as f32)]);
            assert!(run(d.as_mut(), &p, sine(300.0, 4000, 0.5)).iter().all(|s| s.is_finite()));
        }
        let (mut d, p) = dev("native.dcoffset", &[(0, 0.25)]);
        let out = run(d.as_mut(), &p, vec![0.0; 2000]);
        assert!(out.iter().all(|s| (*s - 0.25).abs() < 1e-6));
        let (mut d, p) = dev("native.dcoffset", &[(0, 0.0), (1, 1.0)]);
        let out = run(d.as_mut(), &p, vec![0.3; 2 * 40_000]);
        assert!(out[2 * 39_000].abs() < 0.01, "{}", out[2 * 39_000]);
    }

    #[test]
    fn sidechain_compressor_follows_the_key_signal_not_its_own_input() {
        let (mut d, p) = dev("native.sidechain", &[(0, 2.0), (1, -30.0), (2, 20.0), (3, 0.1), (4, 5.0)]);
        let mut buf = sine(440.0, 4000, 0.5);
        let loud_key = sine(100.0, 4000, 0.9);
        let silent = vec![0.0; 8000];
        let mut quiet_key = buf.clone();
        d.process_sc(&p, &mut buf, Some(&loud_key));
        assert!(peak(&buf[4000..]) < 0.08, "ducked: {}", peak(&buf[4000..]));
        let (mut d2, p2) = dev("native.sidechain", &[(0, 2.0), (1, -30.0), (2, 20.0), (3, 0.1), (4, 5.0)]);
        d2.process_sc(&p2, &mut quiet_key, Some(&silent));
        assert!(peak(&quiet_key[4000..]) > 0.45, "untouched with a silent key");
    }

    #[test]
    fn lfo_follows_the_song_tempo_when_synced() {
        let (mut d, mut p) = dev("native.lfo", &[(2, 3.0), (4, 1.0), (6, 3.0)]); // square, 1 beat
        d.set_tempo(120.0); // one beat = 0.5 s
        p[3] = 10.0; // the Hz setting must be ignored while synced
        let mut flips = 0;
        let mut last = 0.0f32;
        let mut buf = vec![0.0f32; 2 * 512];
        for _ in 0..(FS as usize / 512) {
            d.process(&p, &mut buf);
            let v = d.modulation().unwrap();
            if v.signum() != last.signum() && last != 0.0 {
                flips += 1;
            }
            last = v;
        }
        // 1 second = 2 cycles = 4 flips (give or take the block quantisation)
        assert!((3..=5).contains(&flips), "{flips}");
    }
}
