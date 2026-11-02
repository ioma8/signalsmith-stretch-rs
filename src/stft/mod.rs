//! Self-normalising STFT used by the time/pitch algorithm.
//!
//! This is an internal implementation detail. The structure is kept close to
//! the reference numerical flow so latency and spectrum conventions remain
//! explicit.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "DSP indexing and numerical structure are intentional"
)]

#[cfg(not(feature = "std"))]
use alloc::vec;
use alloc::vec::Vec;
use core::mem;

use crate::fft::{Complex, RealFft};
use crate::sample::{Sample, sf};

const ALMOST_ZERO: f64 = 1e-30;

mod windows;

pub(crate) use windows::WindowShape;
use windows::{ApproximateConfinedGaussian, Kaiser, force_perfect_reconstruction};

pub(crate) struct StftInput<S> {
    pub(crate) pos: usize,
    pub(crate) buffer: Vec<S>,
}

pub(crate) struct StftOutput<S> {
    pub(crate) pos: usize,
    pub(crate) buffer: Vec<S>,
    pub(crate) window_products: Vec<S>,
}

impl<S: Clone> Clone for StftInput<S> {
    fn clone(&self) -> Self {
        Self {
            pos: self.pos,
            buffer: self.buffer.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.pos = source.pos;
        self.buffer.clone_from(&source.buffer);
    }
}

impl<S: Clone> Clone for StftOutput<S> {
    fn clone(&self) -> Self {
        Self {
            pos: self.pos,
            buffer: self.buffer.clone(),
            window_products: self.window_products.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.pos = source.pos;
        self.buffer.clone_from(&source.buffer);
        self.window_products.clone_from(&source.window_products);
    }
}

impl<S> StftOutput<S> {
    pub(crate) fn swap(&mut self, other: &mut Self) {
        mem::swap(&mut self.pos, &mut other.pos);
        mem::swap(&mut self.buffer, &mut other.buffer);
        mem::swap(&mut self.window_products, &mut other.window_products);
    }
}

impl<S> StftInput<S> {
    pub(crate) fn swap(&mut self, other: &mut Self) {
        mem::swap(&mut self.pos, &mut other.pos);
        mem::swap(&mut self.buffer, &mut other.buffer);
    }
}

pub(crate) struct DynamicStft<S: Sample> {
    analysis_channels: usize,
    synthesis_channels: usize,
    input_length_samples: usize,
    block_samples: usize,
    fft_samples: usize,
    fft_bins: usize,
    default_interval: usize,
    analysis_window: Vec<S>,
    synthesis_window: Vec<S>,
    analysis_offset: usize,
    synthesis_offset: usize,
    spectrum_buffer: Vec<Complex<S>>,
    time_buffer: Vec<S>,
    fft: RealFft<S>,
    pub(crate) input: StftInput<S>,
    pub(crate) output: StftOutput<S>,
}

impl<S: Sample> DynamicStft<S> {
    pub(crate) fn new(
        in_channels: usize,
        out_channels: usize,
        block_samples: usize,
        extra_input_history: usize,
        interval_samples: usize,
        fft_min_size: usize,
    ) -> Self {
        assert!(block_samples > 0, "block size must be positive");
        let fft_min = if fft_min_size == 0 {
            block_samples
        } else {
            fft_min_size
        };
        let fft_samples = RealFft::<S>::real_fft_size(fft_min);
        assert!(fft_samples > 0, "invalid FFT size");
        let fft_bins = fft_samples / 2;
        let input_length_samples = block_samples
            .checked_add(extra_input_history)
            .expect("input history size overflow");
        assert!(input_length_samples > 0, "input history too small");
        let spectrum_len = fft_bins
            .checked_mul(in_channels.max(out_channels))
            .expect("spectrum buffer size overflow");
        let input_buffer_len = input_length_samples
            .checked_mul(in_channels)
            .expect("input buffer size overflow");
        let output_buffer_len = block_samples
            .checked_mul(out_channels)
            .expect("output buffer size overflow");

        let mut stft = Self {
            analysis_channels: in_channels,
            synthesis_channels: out_channels,
            input_length_samples,
            block_samples,
            fft_samples,
            fft_bins,
            default_interval: 0,
            analysis_window: vec![S::zero(); block_samples],
            synthesis_window: vec![S::zero(); block_samples],
            analysis_offset: 0,
            synthesis_offset: 0,
            spectrum_buffer: vec![Complex::default(); spectrum_len],
            time_buffer: vec![S::zero(); fft_samples.max(block_samples)],
            fft: RealFft::new(fft_samples),
            input: StftInput {
                pos: 0,
                buffer: vec![S::zero(); input_buffer_len],
            },
            output: StftOutput {
                pos: 0,
                buffer: vec![S::zero(); output_buffer_len],
                window_products: vec![S::zero(); block_samples],
            },
        };

        stft.set_interval(
            if interval_samples == 0 {
                (block_samples / 4).max(1)
            } else {
                interval_samples
            },
            WindowShape::Acg,
        );
        stft.reset(S::one());
        stft
    }

    pub(crate) const fn block_samples(&self) -> usize {
        self.block_samples
    }

    pub(crate) const fn fft_samples(&self) -> usize {
        self.fft_samples
    }

    pub(crate) const fn default_interval(&self) -> usize {
        self.default_interval
    }

    pub(crate) const fn bands(&self) -> usize {
        self.fft_bins
    }

    pub(crate) const fn analysis_latency(&self) -> usize {
        self.block_samples - self.analysis_offset
    }

    pub(crate) const fn synthesis_latency(&self) -> usize {
        self.synthesis_offset
    }

    pub(crate) fn bin_to_freq(&self, bin: S) -> S {
        (bin + S::from_f64(0.5).expect("sample type"))
            / S::from_usize(self.fft_samples).expect("sample type")
    }

    pub(crate) fn freq_to_bin(&self, freq: S) -> S {
        freq * S::from_usize(self.fft_samples).expect("sample type")
            - S::from_f64(0.5).expect("sample type")
    }

    pub(crate) fn reset(&mut self, product_weight: S) {
        self.input.pos = self.block_samples;
        self.output.pos = 0;
        self.input.buffer.fill(S::zero());
        self.output.buffer.fill(S::zero());
        self.spectrum_buffer.fill(Complex::default());
        self.output.window_products.fill(S::zero());

        self.add_window_product();
        if self.block_samples > self.default_interval {
            for i in (0..(self.block_samples - self.default_interval)).rev() {
                let value = self.output.window_products[i + self.default_interval];
                self.output.window_products[i] += value;
            }
        }
        for value in &mut self.output.window_products {
            *value = *value * product_weight + sf::<S>(ALMOST_ZERO);
        }
        self.move_output(self.default_interval);
    }

    pub(crate) fn write_input(&mut self, channel: usize, offset: usize, data: &[S]) {
        assert!(
            channel < self.analysis_channels,
            "input channel out of range"
        );
        assert!(
            data.len() <= self.input_length_samples,
            "input block too long"
        );
        let buffer = &mut self.input.buffer
            [channel * self.input_length_samples..(channel + 1) * self.input_length_samples];

        let offset_pos = (self.input.pos + offset) % self.input_length_samples;
        let input_wrap_index = self.input_length_samples - offset_pos;
        let chunk1 = data.len().min(input_wrap_index);
        buffer[offset_pos..offset_pos + chunk1].copy_from_slice(&data[..chunk1]);
        if data.len() > chunk1 {
            buffer[..data.len() - chunk1].copy_from_slice(&data[chunk1..]);
        }
    }

    pub(crate) fn move_input(&mut self, samples: usize) {
        assert!(samples <= self.input_length_samples, "input move too large");
        self.input.pos = (self.input.pos + samples) % self.input_length_samples;
    }

    pub(crate) fn finish_output(&mut self, strength: S, offset: usize) {
        if self.block_samples == 0 {
            return;
        }
        let mut max_window_product = S::zero();
        let chunk1 = offset.max(self.block_samples.min(self.block_samples - self.output.pos));

        for i in offset..chunk1 {
            let i2 = self.output.pos + i;
            let wp = &mut self.output.window_products[i2];
            max_window_product = max_window_product.max(*wp);
            *wp += (max_window_product - *wp) * strength;
        }
        for i in chunk1..self.block_samples {
            let i2 = i + self.output.pos - self.block_samples;
            let wp = &mut self.output.window_products[i2];
            max_window_product = max_window_product.max(*wp);
            *wp += (max_window_product - *wp) * strength;
        }
    }

    pub(crate) fn read_output(&self, channel: usize, offset: usize, out: &mut [S]) {
        assert!(
            channel < self.synthesis_channels,
            "output channel out of range"
        );
        assert!(out.len() <= self.block_samples, "output read too long");
        assert!(
            offset <= self.block_samples,
            "output read offset exceeds block"
        );
        let buffer =
            &self.output.buffer[channel * self.block_samples..(channel + 1) * self.block_samples];

        let offset_pos = (self.output.pos + offset) % self.block_samples;
        let output_wrap_index = self.block_samples - offset_pos;
        let chunk1 = out.len().min(output_wrap_index);
        for i in 0..chunk1 {
            let i2 = offset_pos + i;
            out[i] = buffer[i2] / self.output.window_products[i2];
        }
        for i in chunk1..out.len() {
            let i2 = i + offset_pos - self.block_samples;
            out[i] = buffer[i2] / self.output.window_products[i2];
        }
    }

    pub(crate) fn add_output(&mut self, channel: usize, data: &[S]) {
        assert!(
            channel < self.synthesis_channels,
            "output channel out of range"
        );
        let length = data.len().min(self.block_samples);
        let buffer = &mut self.output.buffer
            [channel * self.block_samples..(channel + 1) * self.block_samples];

        let offset_pos = self.output.pos;
        let output_wrap_index = self.block_samples - offset_pos;
        let chunk1 = length.min(output_wrap_index);
        for i in 0..chunk1 {
            let i2 = offset_pos + i;
            buffer[i2] += data[i] * self.output.window_products[i2];
        }
        for i in chunk1..length {
            let i2 = i + offset_pos - self.block_samples;
            buffer[i2] += data[i] * self.output.window_products[i2];
        }
    }

    pub(crate) fn move_output(&mut self, samples: usize) {
        assert!(samples <= self.block_samples, "output move too large");
        if samples == 1 {
            for c in 0..self.synthesis_channels {
                self.output.buffer[self.output.pos + c * self.block_samples] = S::zero();
            }
            self.output.window_products[self.output.pos] = sf::<S>(ALMOST_ZERO);
            self.output.pos += 1;
            if self.output.pos >= self.block_samples {
                self.output.pos = 0;
            }
            return;
        }

        let output_wrap_index = self.block_samples - self.output.pos;
        let chunk1 = samples.min(output_wrap_index);
        for c in 0..self.synthesis_channels {
            let buffer =
                &mut self.output.buffer[c * self.block_samples..(c + 1) * self.block_samples];
            for i in 0..chunk1 {
                buffer[self.output.pos + i] = S::zero();
            }
            for i in chunk1..samples {
                buffer[i + self.output.pos - self.block_samples] = S::zero();
            }
        }
        for i in 0..chunk1 {
            self.output.window_products[self.output.pos + i] = sf::<S>(ALMOST_ZERO);
        }
        for i in chunk1..samples {
            self.output.window_products[i + self.output.pos - self.block_samples] =
                sf::<S>(ALMOST_ZERO);
        }
        self.output.pos = (self.output.pos + samples) % self.block_samples;
    }

    pub(crate) fn spectrum(&mut self, channel: usize) -> &mut [Complex<S>] {
        assert!(channel < self.analysis_channels.max(self.synthesis_channels));
        &mut self.spectrum_buffer[channel * self.fft_bins..(channel + 1) * self.fft_bins]
    }

    pub(crate) fn set_interval(&mut self, interval: usize, shape: WindowShape) {
        assert!(interval > 0, "interval must be positive");
        assert!(interval <= self.block_samples, "interval exceeds block");
        self.default_interval = interval;

        match shape {
            WindowShape::Acg => {
                let window = ApproximateConfinedGaussian::with_bandwidth(
                    self.block_samples as f64 / interval as f64,
                );
                window.fill(&mut self.synthesis_window, false);
            }
            WindowShape::Kaiser => {
                let window =
                    Kaiser::with_bandwidth(self.block_samples as f64 / interval as f64, true);
                window.fill(&mut self.synthesis_window, true);
            }
        }

        self.analysis_offset = self.block_samples / 2;
        self.synthesis_offset = self.block_samples / 2;
        if self.analysis_channels == 0 {
            self.analysis_window.fill(S::one());
        } else {
            force_perfect_reconstruction(
                &mut self.synthesis_window,
                self.block_samples,
                self.default_interval,
            );
            self.analysis_window.copy_from_slice(&self.synthesis_window);
        }
        for i in 0..self.block_samples {
            if self.analysis_window[i] > self.analysis_window[self.analysis_offset] {
                self.analysis_offset = i;
            }
            if self.synthesis_window[i] > self.synthesis_window[self.synthesis_offset] {
                self.synthesis_offset = i;
            }
        }
    }

    pub(crate) const fn analyse_steps(&self) -> usize {
        self.analysis_channels
    }

    pub(crate) const fn synthesise_steps(&self) -> usize {
        self.synthesis_channels
    }

    pub(crate) fn analyse_step(&mut self, channel: usize, samples_in_past: usize) {
        assert!(channel < self.analysis_channels);
        let offset_pos = (self.input_length_samples * 2 + self.input.pos)
            .wrapping_sub(self.block_samples)
            .wrapping_sub(samples_in_past)
            % self.input_length_samples;
        let input_wrap_index = self.input_length_samples - offset_pos;
        let chunk1 = self.analysis_offset.min(input_wrap_index);
        let chunk2 = self
            .analysis_offset
            .max(self.block_samples.min(input_wrap_index));

        let buffer_size = self.time_buffer.len();
        let buffer = &self.input.buffer
            [channel * self.input_length_samples..(channel + 1) * self.input_length_samples];

        for i in 0..chunk1 {
            let w = -self.analysis_window[i];
            let ti = i + (buffer_size - self.analysis_offset);
            let bi = offset_pos + i;
            self.time_buffer[ti] = buffer[bi] * w;
        }
        for i in chunk1..self.analysis_offset {
            let w = -self.analysis_window[i];
            let ti = i + (buffer_size - self.analysis_offset);
            let bi = i + offset_pos - self.input_length_samples;
            self.time_buffer[ti] = buffer[bi] * w;
        }
        for i in self.analysis_offset..chunk2 {
            let w = self.analysis_window[i];
            let ti = i - self.analysis_offset;
            let bi = offset_pos + i;
            self.time_buffer[ti] = buffer[bi] * w;
        }
        for i in chunk2..self.block_samples {
            let w = self.analysis_window[i];
            let ti = i - self.analysis_offset;
            let bi = i + offset_pos - self.input_length_samples;
            self.time_buffer[ti] = buffer[bi] * w;
        }
        for i in self.block_samples - self.analysis_offset..self.fft_samples - self.analysis_offset
        {
            self.time_buffer[i] = S::zero();
        }

        let time = &self.time_buffer[..self.fft_samples];
        let spectrum =
            &mut self.spectrum_buffer[channel * self.fft_bins..(channel + 1) * self.fft_bins];
        self.fft.fft(time, spectrum);
    }

    pub(crate) fn synthesise_step(&mut self, channel: usize) {
        assert!(channel < self.synthesis_channels);
        if channel == 0 {
            self.add_window_product();
        }
        let spectrum =
            &self.spectrum_buffer[channel * self.fft_bins..(channel + 1) * self.fft_bins];
        let mut time = mem::take(&mut self.time_buffer);
        self.fft.ifft(spectrum, &mut time[..self.fft_samples]);

        let output_wrap_index = self.block_samples - self.output.pos;
        let chunk1 = self.synthesis_offset.min(output_wrap_index);
        let chunk2 = self
            .block_samples
            .min(self.synthesis_offset.max(output_wrap_index));
        let buffer = &mut self.output.buffer
            [channel * self.block_samples..(channel + 1) * self.block_samples];

        for i in 0..chunk1 {
            let w = -self.synthesis_window[i];
            let ti = i + (self.fft_samples - self.synthesis_offset);
            let bi = self.output.pos + i;
            buffer[bi] += time[ti] * w;
        }
        for i in chunk1..self.synthesis_offset {
            let w = -self.synthesis_window[i];
            let ti = i + (self.fft_samples - self.synthesis_offset);
            let bi = i + self.output.pos - self.block_samples;
            buffer[bi] += time[ti] * w;
        }
        for i in self.synthesis_offset..chunk2 {
            let w = self.synthesis_window[i];
            let ti = i - self.synthesis_offset;
            let bi = self.output.pos + i;
            buffer[bi] += time[ti] * w;
        }
        for i in chunk2..self.block_samples {
            let w = self.synthesis_window[i];
            let ti = i - self.synthesis_offset;
            let bi = i + self.output.pos - self.block_samples;
            buffer[bi] += time[ti] * w;
        }
        self.time_buffer = time;
    }

    fn add_window_product(&mut self) {
        let output_wrap_index = self.block_samples - self.output.pos;
        let chunk1 = self.block_samples.min(output_wrap_index);
        let scaling = S::from_usize(self.fft_samples).expect("sample type");

        for i in 0..chunk1 {
            let wa = self.analysis_window[i];
            let ws = self.synthesis_window[i];
            let bi = self.output.pos + i;
            self.output.window_products[bi] += wa * ws * scaling;
        }
        for i in chunk1..self.block_samples {
            let wa = self.analysis_window[i];
            let ws = self.synthesis_window[i];
            let bi = i + self.output.pos - self.block_samples;
            self.output.window_products[bi] += wa * ws * scaling;
        }
    }
}
