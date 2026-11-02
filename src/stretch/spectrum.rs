//! Spectral mapping, smoothing, peak detection, and band interpolation.

use super::math::{cmul, cnorm};
use super::types::{Peak, PitchMapPoint};
use super::{
    MAX_CLEAN_STRETCH, SMOOTH_ENERGY_STEPS, SPLIT_MAIN_PREDICTION, Sample, SignalsmithStretch,
};
use crate::fft::Complex;
use crate::sample::{ceil_isize, round_isize, saturating_isize, sf, su};

impl<S: Sample> SignalsmithStretch<S> {
    pub(super) fn process_spectrum(&mut self, mut step: usize) {
        let mut time_factor = self.block_process.time_factor;
        let smoothing_bins =
            su::<S>(self.stft.fft_samples()) / su::<S>(self.stft.default_interval());
        let long_vertical_step = round_isize(smoothing_bins);
        time_factor = time_factor.max(S::one() / sf(MAX_CLEAN_STRETCH));
        let random_time_factor = time_factor > sf(MAX_CLEAN_STRETCH);
        let distribute_min = if random_time_factor {
            sf::<S>(MAX_CLEAN_STRETCH * 2.0) - time_factor
        } else {
            -time_factor
        };

        if self.block_process.new_spectrum {
            if step < self.channels {
                let channel = step;
                let offset = channel * self.bands;
                let phase0 = self.stft.bin_to_freq(S::zero())
                    * su::<S>(self.stft.default_interval())
                    * sf(core::f64::consts::TAU);
                let (sin0, cos0) = phase0.sin_cos();
                let mut rot = Complex::new(cos0, sin0);
                let freq_step = self.stft.bin_to_freq(S::one()) - self.stft.bin_to_freq(S::zero());
                let phase_step =
                    freq_step * su::<S>(self.stft.default_interval()) * sf(core::f64::consts::TAU);
                let (sin_step, cos_step) = phase_step.sin_cos();
                let rot_step = Complex::new(cos_step, sin_step);

                for b in 0..self.bands {
                    let band = &mut self.channel_bands[offset + b];
                    band.output = cmul(band.output, rot);
                    band.prev_input = cmul(band.prev_input, rot);
                    rot = cmul(rot, rot_step);
                }
                return;
            }
            step -= self.channels;
        }

        if self.block_process.mapped_frequencies {
            if step < SMOOTH_ENERGY_STEPS {
                self.smooth_energy(step, smoothing_bins);
                return;
            }
            step -= SMOOTH_ENERGY_STEPS;
            if step == 0 {
                self.find_peaks();
                return;
            }
            step -= 1;
        }

        if step == 0 {
            if self.block_process.mapped_frequencies {
                self.update_output_map();
            } else {
                for c in 0..self.channels {
                    let offset = c * self.bands;
                    for b in 0..self.bands {
                        let energy = cnorm(self.channel_bands[offset + b].input);
                        self.channel_bands[offset + b].input_energy = energy;
                    }
                }
                for b in 0..self.bands {
                    self.output_map[b] = PitchMapPoint {
                        input_bin: su(b),
                        freq_grad: S::one(),
                    };
                }
            }
            return;
        }
        step -= 1;

        if self.block_process.process_formants {
            if step < 3 {
                self.update_formants(step);
                return;
            }
            step -= 3;
        }

        if step < self.channels {
            self.preliminary_prediction(step);
            return;
        }
        step -= self.channels;

        if step < SPLIT_MAIN_PREDICTION {
            self.main_prediction(
                step,
                time_factor,
                random_time_factor,
                distribute_min,
                long_vertical_step,
            );
            return;
        }
        step -= SPLIT_MAIN_PREDICTION;

        if self.block_process.new_spectrum && step == 0 {
            for band in &mut self.channel_bands {
                band.prev_input = band.input;
            }
        }
    }

    pub(super) fn smooth_energy(&mut self, step: usize, smoothing_bins: S) {
        let smoothing_slew = S::one() / (S::one() + smoothing_bins * sf(0.5));
        if step == 0 {
            self.energy.fill(S::zero());
            for c in 0..self.channels {
                let offset = c * self.bands;
                for b in 0..self.bands {
                    let energy = cnorm(self.channel_bands[offset + b].input);
                    self.channel_bands[offset + b].input_energy = energy;
                    self.energy[b] += energy;
                }
            }
            self.smoothed_energy.copy_from_slice(&self.energy);
            self.smooth_energy_state = S::zero();
            return;
        }

        let mut e = self.smooth_energy_state;
        for b in (0..self.bands).rev() {
            e += (self.smoothed_energy[b] - e) * smoothing_slew;
            self.smoothed_energy[b] = e;
        }
        for b in 0..self.bands {
            e += (self.smoothed_energy[b] - e) * smoothing_slew;
            self.smoothed_energy[b] = e;
        }
        self.smooth_energy_state = e;
    }

    pub(super) fn map_freq(&self, freq: S) -> S {
        if let Some(map) = &self.custom_freq_map {
            return map(freq);
        }
        if freq > self.freq_tonality_limit {
            freq + (self.freq_multiplier - S::one()) * self.freq_tonality_limit
        } else {
            freq * self.freq_multiplier
        }
    }

    pub(super) fn inv_map_formant(&self, freq: S) -> S {
        if freq * self.inv_formant_multiplier > self.freq_tonality_limit {
            freq + (S::one() - self.formant_multiplier) * self.freq_tonality_limit
        } else {
            freq * self.inv_formant_multiplier
        }
    }

    pub(super) fn find_peaks(&mut self) {
        self.peaks.clear();
        let mut start = 0;
        while start < self.bands {
            if self.energy[start] > self.smoothed_energy[start] {
                let mut end = start;
                let mut band_sum = S::zero();
                let mut energy_sum = S::zero();
                while end < self.bands && self.energy[end] > self.smoothed_energy[end] {
                    band_sum += su::<S>(end) * self.energy[end];
                    energy_sum += self.energy[end];
                    end += 1;
                }
                let avg_band = band_sum / energy_sum;
                let avg_freq = self.stft.bin_to_freq(avg_band);
                let input = avg_band;
                let output = self.stft.freq_to_bin(self.map_freq(avg_freq));
                self.peaks.push(Peak { input, output });
                start = end;
            }
            start += 1;
        }
    }

    pub(super) fn update_output_map(&mut self) {
        if self.peaks.is_empty() {
            for b in 0..self.bands {
                self.output_map[b] = PitchMapPoint {
                    input_bin: su(b),
                    freq_grad: S::one(),
                };
            }
            return;
        }

        let first = self.peaks[0];
        let bottom_offset = first.input - first.output;
        let first_end = ceil_isize(first.output).clamp(0, self.bands as isize) as usize;
        for b in 0..first_end {
            self.output_map[b] = PitchMapPoint {
                input_bin: su::<S>(b) + bottom_offset,
                freq_grad: S::one(),
            };
        }

        for p in 1..self.peaks.len() {
            let prev = self.peaks[p - 1];
            let next = self.peaks[p];
            let range_scale = S::one() / (next.output - prev.output);
            let out_offset = prev.input - prev.output;
            let out_scale = next.input - next.output - prev.input + prev.output;
            let grad_scale = out_scale * range_scale;
            let start_bin = ceil_isize(prev.output).max(0) as usize;
            let end_bin = ceil_isize(next.output).clamp(0, self.bands as isize) as usize;
            for b in start_bin..end_bin {
                let r = (su::<S>(b) - prev.output) * range_scale;
                let h = r * r * (sf::<S>(3.0) - sf::<S>(2.0) * r);
                let out_b = su::<S>(b) + out_offset + h * out_scale;
                let grad_h = sf::<S>(6.0) * r * (S::one() - r);
                let grad_b = S::one() + grad_h * grad_scale;
                self.output_map[b] = PitchMapPoint {
                    input_bin: out_b,
                    freq_grad: grad_b,
                };
            }
        }

        let last = *self.peaks.last().expect("non-empty peaks");
        let top_offset = last.input - last.output;
        let top_start = saturating_isize(last.output).max(0) as usize;
        for b in top_start..self.bands {
            self.output_map[b] = PitchMapPoint {
                input_bin: su::<S>(b) + top_offset,
                freq_grad: S::one(),
            };
        }
    }

    pub(super) fn get_band_complex(&self, channel: usize, index: isize) -> Complex<S> {
        if index < 0 || index >= self.bands as isize {
            return Complex::default();
        }
        self.channel_bands[channel * self.bands + index as usize].input
    }

    pub(super) fn get_band_prev_input(&self, channel: usize, index: isize) -> Complex<S> {
        if index < 0 || index >= self.bands as isize {
            return Complex::default();
        }
        self.channel_bands[channel * self.bands + index as usize].prev_input
    }

    pub(super) fn get_band_output(&self, channel: usize, index: isize) -> Complex<S> {
        if index < 0 || index >= self.bands as isize {
            return Complex::default();
        }
        self.channel_bands[channel * self.bands + index as usize].output
    }

    pub(super) fn get_band_energy(&self, channel: usize, index: isize) -> S {
        if index < 0 || index >= self.bands as isize {
            return S::zero();
        }
        self.channel_bands[channel * self.bands + index as usize].input_energy
    }

    pub(super) fn get_fractional_input(
        &self,
        channel: usize,
        low_index: isize,
        fractional: S,
    ) -> Complex<S> {
        let low = self.get_band_complex(channel, low_index);
        let high = self.get_band_complex(channel, low_index + 1);
        low + (high - low) * fractional
    }

    pub(super) fn get_fractional_prev_input(
        &self,
        channel: usize,
        low_index: isize,
        fractional: S,
    ) -> Complex<S> {
        let low = self.get_band_prev_input(channel, low_index);
        let high = self.get_band_prev_input(channel, low_index + 1);
        low + (high - low) * fractional
    }

    pub(super) fn get_fractional_energy(
        &self,
        channel: usize,
        low_index: isize,
        fractional: S,
    ) -> S {
        let low = self.get_band_energy(channel, low_index);
        let high = self.get_band_energy(channel, low_index + 1);
        low + (high - low) * fractional
    }
}
