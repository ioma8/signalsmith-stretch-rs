//! Minimal real-FFT wrapper used by the STFT.
//!
//! `microfft` supplies the complex transform. The packing/unpacking and the
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

use std::convert::TryInto;
use std::marker::PhantomData;

use microfft::Complex32;
pub(crate) use num_complex::Complex;

use crate::sample::Sample;

/// Maximum power-of-two FFT size supported by `microfft`.
pub(crate) const MAX_FFT_SIZE: usize = 32_768;

pub(crate) struct RealFft<S: Sample> {
    size: usize,
    half_size: usize,
    tmp_freq: Vec<Complex32>,
    tmp_time: Vec<Complex32>,
    twiddles: Vec<Complex32>,
    half_bin_twists: Vec<Complex32>,
    sample: PhantomData<S>,
}

impl<S: Sample> RealFft<S> {
    pub(crate) fn new(size: usize) -> Self {
        assert!(
            size.is_power_of_two() && (4..=MAX_FFT_SIZE).contains(&size),
            "FFT size must be a power of two between 4 and {MAX_FFT_SIZE}"
        );
        let half_size = size / 2;

        let mut twiddles = Vec::with_capacity(half_size / 2 + 1);
        for i in 0..=half_size / 2 {
            let phase = (i as f64 + 0.5) * (-2.0 * std::f64::consts::PI / size as f64)
                - std::f64::consts::FRAC_PI_2;
            let (sin, cos) = phase.sin_cos();
            twiddles.push(Complex32::new(cos as f32, sin as f32));
        }

        let mut half_bin_twists = Vec::with_capacity(half_size);
        for i in 0..half_size {
            let phase = i as f64 * (-2.0 * std::f64::consts::PI / size as f64);
            let (sin, cos) = phase.sin_cos();
            half_bin_twists.push(Complex32::new(cos as f32, sin as f32));
        }

        Self {
            size,
            half_size,
            tmp_freq: vec![Complex32::default(); half_size],
            tmp_time: vec![Complex32::default(); half_size],
            twiddles,
            half_bin_twists,
            sample: PhantomData,
        }
    }

    /// Real-FFT size: the next power of two at or above `block_samples`.
    pub(crate) fn real_fft_size(block_samples: usize) -> usize {
        assert!(block_samples > 0, "block size must be positive");
        block_samples.next_power_of_two().clamp(4, MAX_FFT_SIZE)
    }

    /// Real forward FFT into exactly `size / 2` bins. The modified spectrum
    /// has no separate Nyquist bin, so callers must provide `size / 2` slots.
    pub(crate) fn fft(&mut self, time: &[S], freq: &mut [Complex<S>]) {
        assert!(time.len() >= self.size);
        assert!(freq.len() >= self.half_size);

        for i in 0..self.half_size {
            let tr = time[2 * i].to_f32().expect("sample type");
            let ti = time[2 * i + 1].to_f32().expect("sample type");
            let twist = self.half_bin_twists[i];
            self.tmp_time[i] =
                Complex32::new(tr * twist.re - ti * twist.im, ti * twist.re + tr * twist.im);
        }
        cfft(self.half_size, &mut self.tmp_time);

        let half = 0.5_f32;
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

            freq[i] = Complex::new(
                S::from_f32(odd_r + even_r).expect("sample type"),
                S::from_f32(odd_i + even_i).expect("sample type"),
            );
            freq[conj_i] = Complex::new(
                S::from_f32(odd_r - even_r).expect("sample type"),
                S::from_f32(even_i - odd_i).expect("sample type"),
            );
        }
    }

    /// Real inverse FFT, unnormalised as used by the STFT.
    pub(crate) fn ifft(&mut self, freq: &[Complex<S>], time: &mut [S]) {
        assert!(freq.len() >= self.half_size);
        assert!(time.len() >= self.size);

        for i in 0..=self.half_size / 2 {
            let conj_i = self.half_size - 1 - i;
            let twiddle = self.twiddles[i];
            let fi = Complex32::new(
                freq[i].re.to_f32().expect("sample type"),
                freq[i].im.to_f32().expect("sample type"),
            );
            let fc = Complex32::new(
                freq[conj_i].re.to_f32().expect("sample type"),
                -freq[conj_i].im.to_f32().expect("sample type"),
            );

            let odd = fi + fc;
            let even_rot = fi - fc;
            let even = Complex32::new(
                even_rot.re * twiddle.re + even_rot.im * twiddle.im,
                even_rot.im * twiddle.re - even_rot.re * twiddle.im,
            );

            self.tmp_freq[i] = odd + even;
            self.tmp_freq[conj_i] = Complex32::new(odd.re - even.re, even.im - odd.im);
        }
        ifft(self.half_size, &mut self.tmp_freq);

        // microfft normalises its inverse FFT; the Stretch windows expect the
        // unnormalised convention.
        let scale = self.half_size as f32;
        for i in 0..self.half_size {
            let tr = self.tmp_freq[i].re * scale;
            let ti = self.tmp_freq[i].im * scale;
            let twist = self.half_bin_twists[i];
            time[2 * i] = S::from_f32(tr * twist.re + ti * twist.im).expect("sample type");
            time[2 * i + 1] = S::from_f32(ti * twist.re - tr * twist.im).expect("sample type");
        }
    }
}

fn cfft(size: usize, buffer: &mut [Complex32]) {
    match size {
        2 => {
            let buffer: &mut [Complex32; 2] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_2(buffer);
        }
        4 => {
            let buffer: &mut [Complex32; 4] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_4(buffer);
        }
        8 => {
            let buffer: &mut [Complex32; 8] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_8(buffer);
        }
        16 => {
            let buffer: &mut [Complex32; 16] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_16(buffer);
        }
        32 => {
            let buffer: &mut [Complex32; 32] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_32(buffer);
        }
        64 => {
            let buffer: &mut [Complex32; 64] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_64(buffer);
        }
        128 => {
            let buffer: &mut [Complex32; 128] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_128(buffer);
        }
        256 => {
            let buffer: &mut [Complex32; 256] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_256(buffer);
        }
        512 => {
            let buffer: &mut [Complex32; 512] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_512(buffer);
        }
        1024 => {
            let buffer: &mut [Complex32; 1024] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_1024(buffer);
        }
        2048 => {
            let buffer: &mut [Complex32; 2048] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_2048(buffer);
        }
        4096 => {
            let buffer: &mut [Complex32; 4096] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_4096(buffer);
        }
        8192 => {
            let buffer: &mut [Complex32; 8192] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_8192(buffer);
        }
        16_384 => {
            let buffer: &mut [Complex32; 16_384] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_16384(buffer);
        }
        32_768 => {
            let buffer: &mut [Complex32; 32_768] = buffer.try_into().expect("FFT size");
            let _ = microfft::complex::cfft_32768(buffer);
        }
        _ => unreachable!("unsupported FFT size"),
    }
}

fn ifft(size: usize, buffer: &mut [Complex32]) {
    match size {
        2 => {
            let buffer: &mut [Complex32; 2] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_2(buffer);
        }
        4 => {
            let buffer: &mut [Complex32; 4] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_4(buffer);
        }
        8 => {
            let buffer: &mut [Complex32; 8] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_8(buffer);
        }
        16 => {
            let buffer: &mut [Complex32; 16] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_16(buffer);
        }
        32 => {
            let buffer: &mut [Complex32; 32] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_32(buffer);
        }
        64 => {
            let buffer: &mut [Complex32; 64] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_64(buffer);
        }
        128 => {
            let buffer: &mut [Complex32; 128] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_128(buffer);
        }
        256 => {
            let buffer: &mut [Complex32; 256] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_256(buffer);
        }
        512 => {
            let buffer: &mut [Complex32; 512] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_512(buffer);
        }
        1024 => {
            let buffer: &mut [Complex32; 1024] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_1024(buffer);
        }
        2048 => {
            let buffer: &mut [Complex32; 2048] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_2048(buffer);
        }
        4096 => {
            let buffer: &mut [Complex32; 4096] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_4096(buffer);
        }
        8192 => {
            let buffer: &mut [Complex32; 8192] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_8192(buffer);
        }
        16_384 => {
            let buffer: &mut [Complex32; 16_384] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_16384(buffer);
        }
        32_768 => {
            let buffer: &mut [Complex32; 32_768] = buffer.try_into().expect("FFT size");
            let _ = microfft::inverse::ifft_32768(buffer);
        }
        _ => unreachable!("unsupported FFT size"),
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
    fn real_fft_sizes_are_power_of_two() {
        assert_eq!(RealFft::<f32>::real_fft_size(80), 128);
        assert_eq!(RealFft::<f32>::real_fft_size(576), 1024);
        assert_eq!(RealFft::<f32>::real_fft_size(32768), 32768);
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
