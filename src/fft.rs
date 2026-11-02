//! Minimal real-FFT wrapper used by the STFT.
//!
//! `rustfft` supplies the complex transform. The packing/unpacking and
//! half-bin-shifted twiddles follow the Stretch spectrum convention.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "DSP indexing and numerical structure are intentional"
)]

use std::sync::Arc;

pub(crate) use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::sample::{Sample, sf};

pub(crate) struct RealFft<S: Sample> {
    size: usize,
    half_size: usize,
    forward: Arc<dyn Fft<S>>,
    inverse: Arc<dyn Fft<S>>,
    tmp_freq: Vec<Complex<S>>,
    tmp_time: Vec<Complex<S>>,
    twiddles: Vec<Complex<S>>,
    half_bin_twists: Vec<Complex<S>>,
}

impl<S: Sample> RealFft<S> {
    pub(crate) fn new(size: usize) -> Self {
        assert!(size > 0 && size.is_multiple_of(2), "FFT size must be even");
        let half_size = size / 2;
        let mut planner = FftPlanner::<S>::new();
        let forward = planner.plan_fft_forward(half_size);
        let inverse = planner.plan_fft_inverse(half_size);

        let mut twiddles = Vec::with_capacity(half_size / 2 + 1);
        for i in 0..=half_size / 2 {
            let phase = sf::<S>(
                (i as f64 + 0.5) * (-2.0 * std::f64::consts::PI / size as f64)
                    - std::f64::consts::FRAC_PI_2,
            );
            let (sin, cos) = phase.sin_cos();
            twiddles.push(Complex::new(cos, sin));
        }

        let mut half_bin_twists = Vec::with_capacity(half_size);
        for i in 0..half_size {
            let phase = sf::<S>(i as f64 * (-2.0 * std::f64::consts::PI / size as f64));
            let (sin, cos) = phase.sin_cos();
            half_bin_twists.push(Complex::new(cos, sin));
        }

        Self {
            size,
            half_size,
            forward,
            inverse,
            tmp_freq: vec![Complex::default(); half_size],
            tmp_time: vec![Complex::default(); half_size],
            twiddles,
            half_bin_twists,
        }
    }

    /// Split-FFT fast-size helper; it does not include the outer real-FFT
    /// rounding step.
    pub(crate) fn split_fast_size_above(size: usize) -> usize {
        let mut pow2 = 1;
        while pow2 < 16 && pow2 < size {
            pow2 *= 2;
        }
        while pow2 * 8 < size {
            pow2 *= 2;
        }
        let mut multiple = size.div_ceil(pow2);
        if multiple == 7 {
            multiple += 1;
        }
        multiple * pow2
    }

    /// Real-FFT size using the split-FFT helper and the dynamic-STFT doubling.
    pub(crate) fn real_fft_size(block_samples: usize) -> usize {
        let half = block_samples.div_ceil(2);
        Self::split_fast_size_above(half.div_ceil(2)) * 4
    }

    /// Real forward FFT into exactly `size / 2` bins.  The modified spectrum
    /// has no separate Nyquist bin, so callers must provide `size / 2` slots.
    pub(crate) fn fft(&mut self, time: &[S], freq: &mut [Complex<S>]) {
        assert!(time.len() >= self.size);
        assert!(freq.len() >= self.half_size);
        let half = S::from_f64(0.5).expect("sample type");
        for i in 0..self.half_size {
            let tr = time[2 * i];
            let ti = time[2 * i + 1];
            let twist = self.half_bin_twists[i];
            self.tmp_time[i] =
                Complex::new(tr * twist.re - ti * twist.im, ti * twist.re + tr * twist.im);
        }
        self.forward.process(&mut self.tmp_time);

        for i in 0..=self.half_size / 2 {
            let conj_i = self.half_size - 1 - i;
            let twiddle = self.twiddles[i];
            let fi = self.tmp_time[i];
            let fc = self.tmp_time[conj_i].conj();

            let odd_r = (fi.re + fc.re) * half;
            let odd_i = (fi.im + fc.im) * half;
            let even_ir = (fi.re - fc.re) * half;
            let even_ii = (fi.im - fc.im) * half;
            let even_r = even_ir * twiddle.re - even_ii * twiddle.im;
            let even_i = even_ii * twiddle.re + even_ir * twiddle.im;

            freq[i] = Complex::new(odd_r + even_r, odd_i + even_i);
            freq[conj_i] = Complex::new(odd_r - even_r, even_i - odd_i);
        }
    }

    /// Real inverse FFT, unnormalised as used by the STFT.
    pub(crate) fn ifft(&mut self, freq: &[Complex<S>], time: &mut [S]) {
        assert!(freq.len() >= self.half_size);
        assert!(time.len() >= self.size);

        for i in 0..=self.half_size / 2 {
            let conj_i = self.half_size - 1 - i;
            let twiddle = self.twiddles[i];
            let fi = freq[i];
            let fc = freq[conj_i].conj();

            let odd = fi + fc;
            let even_rot = fi - fc;
            let even = Complex::new(
                even_rot.re * twiddle.re + even_rot.im * twiddle.im,
                even_rot.im * twiddle.re - even_rot.re * twiddle.im,
            );

            self.tmp_freq[i] = odd + even;
            self.tmp_freq[conj_i] = Complex::new(odd.re - even.re, even.im - odd.im);
        }
        self.inverse.process(&mut self.tmp_freq);

        for i in 0..self.half_size {
            let tr = self.tmp_freq[i].re;
            let ti = self.tmp_freq[i].im;
            let twist = self.half_bin_twists[i];
            time[2 * i] = tr * twist.re + ti * twist.im;
            time[2 * i + 1] = ti * twist.re - tr * twist.im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_half_bin(input: &[f32]) -> Vec<Complex<f32>> {
        let n = input.len();
        let h = n / 2;
        (0..h)
            .map(|k| {
                let mut sum = Complex::new(0.0, 0.0);
                for (i, &x) in input.iter().enumerate() {
                    let phase = -std::f64::consts::TAU * (i as f64) * (k as f64 + 0.5) / n as f64;
                    let (sin, cos) = phase.sin_cos();
                    sum += Complex::new(cos as f32, sin as f32) * x;
                }
                sum
            })
            .collect()
    }

    #[test]
    fn real_fft_matches_half_bin_dft() {
        let input: Vec<f32> = (0..16).map(|i| (i as f32 * 0.37).sin() + 0.2).collect();
        let mut fft = RealFft::<f32>::new(16);
        let mut freq = vec![Complex::default(); 8];
        fft.fft(&input, &mut freq);
        let expected = naive_half_bin(&input);
        for k in 0..8 {
            assert!((freq[k] - expected[k]).norm() < 1e-4);
        }
    }

    #[test]
    fn real_fft_roundtrip() {
        let input: Vec<f32> = (0..16).map(|i| (i as f32 * 0.37).sin() + 0.2).collect();
        let mut fft = RealFft::<f32>::new(16);
        let mut freq = vec![Complex::default(); 8];
        fft.fft(&input, &mut freq);
        let mut output = vec![0.0; 16];
        fft.ifft(&freq, &mut output);
        for i in 0..16 {
            assert!(
                (output[i] - input[i] * 16.0).abs() < 1e-3,
                "{i}: {} vs {}",
                output[i],
                input[i] * 16.0
            );
        }
    }
}
