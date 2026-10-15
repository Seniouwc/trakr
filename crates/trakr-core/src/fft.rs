//! A small radix-2 FFT, enough for spectrum displays and partitioned convolution.

use std::f32::consts::PI;

/// In-place complex FFT. `re.len()` must be a power of two and equal to `im.len()`.
/// `inverse` computes the unscaled inverse transform (divide by the length yourself).
pub fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n, "fft length must be a power of two");
    if n < 2 {
        return;
    }
    // bit-reversal permutation
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let sign = if inverse { 1.0 } else { -1.0 };
    let mut len = 2;
    while len <= n {
        let ang = sign * 2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let n_cr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = n_cr;
            }
        }
        len <<= 1;
    }
}

/// Magnitude spectrum (`size / 2` bins, linear, scaled so a full-scale sine reads about 1.0) of `data`
/// with a Hann window. `size` must be a power of two; shorter input is zero padded.
pub fn magnitude_spectrum(data: &[f32], size: usize) -> Vec<f32> {
    let mut re = vec![0.0f32; size];
    let mut im = vec![0.0f32; size];
    let n = data.len().min(size);
    for (i, r) in re.iter_mut().enumerate().take(n) {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / n.max(2) as f32 - 0.0).cos();
        *r = data[i] * w;
    }
    fft(&mut re, &mut im, false);
    let scale = 4.0 / n.max(1) as f32; // Hann window gain 0.5, and the one-sided spectrum doubles
    (0..size / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * scale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_a_sine_and_inverts() {
        let n = 1024;
        let k = 37;
        let sig: Vec<f32> = (0..n).map(|i| (2.0 * PI * k as f32 * i as f32 / n as f32).sin()).collect();
        let (mut re, mut im) = (sig.clone(), vec![0.0; n]);
        fft(&mut re, &mut im, false);
        let mags: Vec<f32> = (0..n / 2).map(|i| (re[i] * re[i] + im[i] * im[i]).sqrt()).collect();
        let peak = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, k);
        fft(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&sig) {
            assert!((a / n as f32 - b).abs() < 1e-3);
        }
    }

    #[test]
    fn spectrum_reads_near_one_for_a_full_scale_sine() {
        let n = 2048;
        let sig: Vec<f32> = (0..n).map(|i| (2.0 * PI * 64.0 * i as f32 / n as f32).sin()).collect();
        let s = magnitude_spectrum(&sig, n);
        assert!((s[64] - 1.0).abs() < 0.15, "{}", s[64]);
        assert!(s[200] < 0.01);
        assert_eq!(magnitude_spectrum(&[], 16).len(), 8);
    }
}
