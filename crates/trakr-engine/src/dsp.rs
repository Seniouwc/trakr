//! Small DSP building blocks shared by voices and devices. Everything here is allocation-free once built.

use std::f32::consts::TAU;

/// Normalized biquad coefficients (RBJ cookbook).
#[derive(Clone, Copy, Debug)]
pub struct Coeffs {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

impl Default for Coeffs {
    fn default() -> Self {
        Self::PASS
    }
}

impl Coeffs {
    pub const PASS: Coeffs = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 };

    fn norm(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    fn setup(fs: f32, f: f32, q: f32) -> (f32, f32, f32) {
        let f = f.clamp(10.0, fs * 0.49);
        let w = TAU * f / fs;
        (w.cos(), w.sin(), w.sin() / (2.0 * q.max(0.05)))
    }

    pub fn lowpass(fs: f32, f: f32, q: f32) -> Self {
        let (c, _, a) = Self::setup(fs, f, q);
        Self::norm((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn highpass(fs: f32, f: f32, q: f32) -> Self {
        let (c, _, a) = Self::setup(fs, f, q);
        Self::norm((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn bandpass(fs: f32, f: f32, q: f32) -> Self {
        let (c, _, a) = Self::setup(fs, f, q);
        Self::norm(a, 0.0, -a, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn notch(fs: f32, f: f32, q: f32) -> Self {
        let (c, _, a) = Self::setup(fs, f, q);
        Self::norm(1.0, -2.0 * c, 1.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }

    pub fn peak(fs: f32, f: f32, q: f32, gain_db: f32) -> Self {
        let (c, _, a) = Self::setup(fs, f, q);
        let big_a = 10f32.powf(gain_db / 40.0);
        Self::norm(1.0 + a * big_a, -2.0 * c, 1.0 - a * big_a, 1.0 + a / big_a, -2.0 * c, 1.0 - a / big_a)
    }

    pub fn low_shelf(fs: f32, f: f32, gain_db: f32) -> Self {
        let (c, s, _) = Self::setup(fs, f, 0.707);
        let big_a = 10f32.powf(gain_db / 40.0);
        let beta = 2.0 * big_a.sqrt() * (s / 2.0 * 2f32.sqrt());
        Self::norm(
            big_a * ((big_a + 1.0) - (big_a - 1.0) * c + beta),
            2.0 * big_a * ((big_a - 1.0) - (big_a + 1.0) * c),
            big_a * ((big_a + 1.0) - (big_a - 1.0) * c - beta),
            (big_a + 1.0) + (big_a - 1.0) * c + beta,
            -2.0 * ((big_a - 1.0) + (big_a + 1.0) * c),
            (big_a + 1.0) + (big_a - 1.0) * c - beta,
        )
    }

    pub fn high_shelf(fs: f32, f: f32, gain_db: f32) -> Self {
        let (c, s, _) = Self::setup(fs, f, 0.707);
        let big_a = 10f32.powf(gain_db / 40.0);
        let beta = 2.0 * big_a.sqrt() * (s / 2.0 * 2f32.sqrt());
        Self::norm(
            big_a * ((big_a + 1.0) + (big_a - 1.0) * c + beta),
            -2.0 * big_a * ((big_a - 1.0) + (big_a + 1.0) * c),
            big_a * ((big_a + 1.0) + (big_a - 1.0) * c - beta),
            (big_a + 1.0) - (big_a - 1.0) * c + beta,
            2.0 * ((big_a - 1.0) - (big_a + 1.0) * c),
            (big_a + 1.0) - (big_a - 1.0) * c - beta,
        )
    }
}

/// Transposed direct form II state for one channel.
#[derive(Clone, Copy, Default, Debug)]
pub struct Biquad {
    z1: f32,
    z2: f32,
}

impl Biquad {
    #[inline]
    pub fn process(&mut self, c: &Coeffs, x: f32) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        // A filter whose coefficients are swept hard (high Q, fast modulation) can run away; start over
        // instead of letting infinities reach the output.
        if y.is_nan() || y.abs() >= 1.0e6 {
            self.z1 = 0.0;
            self.z2 = 0.0;
            return 0.0;
        }
        y
    }
}

/// One-pole smoothing coefficient for a time constant in seconds.
pub fn smooth_coeff(fs: f32, seconds: f32) -> f32 {
    1.0 - (-1.0 / (fs * seconds.max(1e-5))).exp()
}

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

pub fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// Fractional-delay ring buffer with linear interpolation.
pub struct DelayLine {
    buf: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4)], pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    /// Sample written `delay` samples ago (call after `write` for a delay of at least 1).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let n = self.buf.len();
        let delay = delay.clamp(1.0, (n - 2) as f32);
        let i = delay.floor();
        let frac = delay - i;
        let a = self.buf[(self.pos + n - i as usize) % n];
        let b = self.buf[(self.pos + n - i as usize - 1) % n];
        a + (b - a) * frac
    }
}

pub fn sine_phase(phase: f32) -> f32 {
    (phase * TAU).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runaway_filter_recovers_instead_of_reaching_the_output() {
        // deliberately unstable poles: the state would grow without bound
        let bad = Coeffs { b0: 1.0, b1: 0.0, b2: 0.0, a1: -2.5, a2: 1.5 };
        let mut f = Biquad::default();
        let mut peak = 0.0f32;
        for i in 0..20_000 {
            let y = f.process(&bad, if i % 100 == 0 { 1.0 } else { 0.0 });
            assert!(y.is_finite());
            peak = peak.max(y.abs());
        }
        assert!(peak < 1.0e6 * 1.01, "output stays bounded: {peak}");
        // a normal filter is untouched
        let c = Coeffs::lowpass(44_100.0, 1000.0, 0.707);
        let mut g = Biquad::default();
        let out: Vec<f32> = (0..2000).map(|_| g.process(&c, 0.5)).collect();
        assert!((out[1999] - 0.5).abs() < 1e-3, "DC passes a low-pass: {}", out[1999]);
    }
}
