//! Formant estimation and formant-aware energy mapping.

use super::{Sample, SignalsmithStretch};
use crate::sample::{saturating_usize, sf, si, su};

impl<S: Sample> SignalsmithStretch<S> {
    fn get_formant(&self, band: S) -> S {
        if band < S::zero() {
            return S::zero();
        }
        let band = band.min(su::<S>(self.bands));
        let floor_band = saturating_usize(band.floor());
        let fractional = band - su::<S>(floor_band);
        let low = self.formant_metric[floor_band];
        let high = self.formant_metric[floor_band + 1];
        low + (high - low) * fractional
    }

    fn estimate_frequency(&mut self) -> S {
        let mut peak_indices = [0_usize; 3];
        for b in 1..self.bands.saturating_sub(1) {
            let energy = self.formant_metric[b];
            if energy < self.formant_metric[b - 1] || energy <= self.formant_metric[b + 1] {
                continue;
            }

            if energy > self.formant_metric[peak_indices[0]] {
                if energy > self.formant_metric[peak_indices[1]] {
                    if energy > self.formant_metric[peak_indices[2]] {
                        peak_indices = [peak_indices[1], peak_indices[2], b];
                    } else {
                        peak_indices = [peak_indices[1], b, peak_indices[2]];
                    }
                } else {
                    peak_indices[0] = b;
                }
            }
        }

        let mut peak_estimate = peak_indices[2] as i32;
        if self.formant_metric[peak_indices[1]] > self.formant_metric[peak_indices[2]] * sf(0.1) {
            let mut diff = (peak_estimate - peak_indices[1] as i32).abs();
            if diff > peak_estimate / 8 && diff < peak_estimate * 7 / 8 {
                peak_estimate %= diff;
            }
            if self.formant_metric[peak_indices[0]]
                > self.formant_metric[peak_indices[2]] * sf(0.01)
            {
                diff = (peak_estimate - peak_indices[0] as i32).abs();
                if diff > peak_estimate / 8 && diff < peak_estimate * 7 / 8 {
                    peak_estimate %= diff;
                }
            }
        }
        let weight = self.formant_metric[peak_indices[2]];
        let target = si::<S>(peak_estimate as isize) * weight;
        self.freq_estimate_weighted += (target - self.freq_estimate_weighted) * sf(0.25);
        self.freq_estimate_weight += (weight - self.freq_estimate_weight) * sf(0.25);
        self.freq_estimate_weighted / (self.freq_estimate_weight + sf(1e-30))
    }

    pub(super) fn update_formants(&mut self, step: usize) {
        if step == 0 {
            self.formant_metric.fill(S::zero());
            for c in 0..self.channels {
                let offset = c * self.bands;
                for b in 0..self.bands {
                    self.formant_metric[b] += self.channel_bands[offset + b].input_energy;
                }
            }
            self.freq_estimate = self.stft.freq_to_bin(self.formant_base_freq);
            if self.formant_base_freq <= S::zero() {
                let estimate = self.estimate_frequency();
                self.freq_estimate = estimate;
            }
            return;
        }

        if step == 1 {
            let decay = S::one() - S::one() / (self.freq_estimate * sf(0.5) + S::one());
            let mut e = S::zero();
            for _ in 0..2 {
                for b in (0..self.bands).rev() {
                    e = self.formant_metric[b].max(e * decay);
                    self.formant_metric[b] = e;
                }
                for b in 0..self.bands {
                    e = self.formant_metric[b].max(e * decay);
                    self.formant_metric[b] = e;
                }
            }
            let decay = S::one() / decay;
            for _ in 0..2 {
                for b in (0..self.bands).rev() {
                    e = self.formant_metric[b].min(e * decay);
                    self.formant_metric[b] = e;
                }
                for b in 0..self.bands {
                    e = self.formant_metric[b].min(e * decay);
                    self.formant_metric[b] = e;
                }
            }
            return;
        }

        for b in 0..self.bands {
            let input_f = self.stft.bin_to_freq(su(b));
            let mut output_f = if self.formant_compensation {
                self.map_freq(input_f)
            } else {
                input_f
            };
            output_f = self.inv_map_formant(output_f);
            let input_e = self.formant_metric[b];
            let target_e = self.get_formant(self.stft.freq_to_bin(output_f));
            let formant_ratio = target_e / (input_e + sf(1e-30));
            for c in 0..self.channels {
                self.channel_bands[c * self.bands + b].input_energy *= formant_ratio;
            }
        }
    }
}
