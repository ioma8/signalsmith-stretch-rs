//! Sample-by-sample streaming and processing-step scheduling.

use super::{
    BlockProcess, NOISE_FLOOR, SMOOTH_ENERGY_STEPS, SPLIT_MAIN_PREDICTION, Sample,
    SignalsmithStretch,
};
use crate::fft::Complex;
use crate::sample::{round_usize, saturating_usize, sf, su};

impl<S: Sample> SignalsmithStretch<S> {
    #[allow(
        clippy::float_cmp,
        clippy::too_many_lines,
        reason = "exact identity checks and ordered DSP steps mirror the processing branches"
    )]
    pub(super) fn process_inner(
        &mut self,
        inputs: Option<&[&[S]]>,
        input_offset: usize,
        input_samples: usize,
        outputs: &mut [&mut [S]],
        output_samples: usize,
    ) {
        assert!(
            self.is_configured(),
            "configure() or a preset must be called first"
        );
        assert_eq!(outputs.len(), self.channels, "channel count mismatch");
        for channel in outputs.iter() {
            assert!(
                channel.len() >= output_samples,
                "output buffer shorter than output_samples"
            );
        }
        if let Some(inputs) = inputs {
            assert_eq!(inputs.len(), self.channels, "channel count mismatch");
            for channel in inputs {
                assert!(
                    channel.len() >= input_offset + input_samples,
                    "input buffer shorter than requested range"
                );
            }
        }

        let mut prev_copied_input = 0;
        let mut total_energy = S::zero();
        if let Some(inputs) = inputs {
            for c in 0..self.channels {
                let input_channel = inputs[c];
                for i in 0..input_samples {
                    let sample = input_channel[input_offset + i];
                    total_energy += sample * sample;
                }
            }
        }

        if total_energy < sf(NOISE_FLOOR) {
            if self.silence_counter >= 2 * self.stft.block_samples() {
                if self.silence_first {
                    self.silence_first = false;
                    self.block_process = BlockProcess::default();
                    for band in &mut self.channel_bands {
                        band.input = Complex::default();
                        band.prev_input = Complex::default();
                        band.output = Complex::default();
                        band.input_energy = S::zero();
                    }
                }

                if input_samples > 0 {
                    for output_index in 0..output_samples {
                        let input_index = output_index % input_samples;
                        for c in 0..self.channels {
                            outputs[c][output_index] = inputs.map_or(S::zero(), |buffers| {
                                buffers[c][input_offset + input_index]
                            });
                        }
                    }
                } else {
                    for channel in outputs.iter_mut() {
                        channel[..output_samples].fill(S::zero());
                    }
                }

                self.copy_input(inputs, input_offset, input_samples, &mut prev_copied_input);
                return;
            }
            self.silence_counter += input_samples;
        } else {
            self.silence_counter = 0;
            self.silence_first = true;
        }

        for output_index in 0..output_samples {
            let new_block = self.block_process.samples_since_last >= self.stft.default_interval();
            if new_block {
                self.block_process.step = 0;
                self.block_process.steps = 0;
                self.block_process.samples_since_last = 0;

                let input_offset_in_block = if output_samples == 0 {
                    0
                } else {
                    round_usize(
                        su::<S>(output_index) * su::<S>(input_samples) / su::<S>(output_samples),
                    )
                };
                let input_interval = input_offset_in_block as isize - self.prev_input_offset;
                self.prev_input_offset = input_offset_in_block as isize;

                self.copy_input(
                    inputs,
                    input_offset,
                    input_offset_in_block,
                    &mut prev_copied_input,
                );
                self.stashed_input.clone_from(&self.stft.input);
                if self.split_computation {
                    self.stashed_output.clone_from(&self.stft.output);
                    self.stft.move_output(self.stft.default_interval());
                }

                self.block_process.new_spectrum = self.did_seek || input_interval > 0;
                self.block_process.mapped_frequencies =
                    self.custom_freq_map.is_some() || self.freq_multiplier != S::one();
                if self.block_process.new_spectrum {
                    self.block_process.reanalyse_prev = self.did_seek
                        || (input_interval - self.stft.default_interval() as isize).abs() > 1;
                    if self.block_process.reanalyse_prev {
                        self.block_process.steps += self.stft.analyse_steps() + 1;
                    }
                    self.block_process.steps += self.stft.analyse_steps() + 1;
                }

                self.block_process.process_formants = self.formant_multiplier != S::one()
                    || (self.formant_compensation && self.block_process.mapped_frequencies);

                self.block_process.time_factor = if self.did_seek {
                    self.seek_time_factor
                } else {
                    su::<S>(self.stft.default_interval()) / su::<S>(input_interval.max(1) as usize)
                };
                self.did_seek = false;

                self.update_process_spectrum_steps();
                self.block_process.steps += self.process_spectrum_steps;
                self.block_process.steps += self.stft.synthesise_steps() + 1;
            }

            let mut process_to_step = if new_block {
                self.block_process.steps
            } else {
                0
            };
            if self.split_computation {
                let process_ratio = su::<S>(self.block_process.samples_since_last + 1)
                    / su::<S>(self.stft.default_interval());
                let scaled = (su::<S>(self.block_process.steps) + sf(0.999)) * process_ratio;
                process_to_step = self.block_process.steps.min(saturating_usize(scaled));
            }

            while self.block_process.step < process_to_step {
                let step = self.block_process.step;
                self.block_process.step += 1;
                self.process_step(step);
            }

            self.block_process.samples_since_last += 1;
            if self.split_computation {
                self.stashed_output.swap(&mut self.stft.output);
            }
            for c in 0..self.channels {
                let mut sample = S::zero();
                self.stft
                    .read_output(c, 0, core::slice::from_mut(&mut sample));
                outputs[c][output_index] = sample;
            }
            self.stft.move_output(1);
            if self.split_computation {
                self.stashed_output.swap(&mut self.stft.output);
            }
        }

        self.copy_input(inputs, input_offset, input_samples, &mut prev_copied_input);
        self.prev_input_offset -= input_samples as isize;
    }

    fn copy_input(
        &mut self,
        inputs: Option<&[&[S]]>,
        input_offset: usize,
        to_index: usize,
        prev_copied_input: &mut usize,
    ) {
        let max_length = self.stft.block_samples() + self.stft.default_interval();
        let length = max_length.min(to_index.saturating_sub(*prev_copied_input));
        self.tmp_process_buffer.resize(length, S::zero());
        let offset = to_index - length;
        for c in 0..self.channels {
            for i in 0..length {
                self.tmp_process_buffer[i] =
                    inputs.map_or(S::zero(), |buffers| buffers[c][input_offset + offset + i]);
            }
            self.stft.write_input(c, 0, &self.tmp_process_buffer);
        }
        self.stft.move_input(length);
        *prev_copied_input = to_index;
    }

    fn process_step(&mut self, mut step: usize) {
        let interval = self.stft.default_interval();
        if self.block_process.new_spectrum {
            if self.block_process.reanalyse_prev {
                let analyse_steps = self.stft.analyse_steps();
                if step < analyse_steps {
                    self.stashed_input.swap(&mut self.stft.input);
                    self.stft.analyse_step(step, interval);
                    self.stashed_input.swap(&mut self.stft.input);
                    return;
                }
                step -= analyse_steps;
                if step < 1 {
                    for c in 0..self.channels {
                        let offset = c * self.bands;
                        let spectrum = self.stft.spectrum(c);
                        for b in 0..self.bands {
                            self.channel_bands[offset + b].prev_input = spectrum[b];
                        }
                    }
                    return;
                }
                step -= 1;
            }

            let analyse_steps = self.stft.analyse_steps();
            if step < analyse_steps {
                self.stashed_input.swap(&mut self.stft.input);
                self.stft.analyse_step(step, 0);
                self.stashed_input.swap(&mut self.stft.input);
                return;
            }
            step -= analyse_steps;
            if step < 1 {
                for c in 0..self.channels {
                    let offset = c * self.bands;
                    let spectrum = self.stft.spectrum(c);
                    for b in 0..self.bands {
                        self.channel_bands[offset + b].input = spectrum[b];
                    }
                }
                return;
            }
            step -= 1;
        }

        if step < self.process_spectrum_steps {
            self.process_spectrum(step);
            return;
        }
        step -= self.process_spectrum_steps;

        if step < 1 {
            for c in 0..self.channels {
                let offset = c * self.bands;
                let spectrum = self.stft.spectrum(c);
                for b in 0..self.bands {
                    spectrum[b] = self.channel_bands[offset + b].output;
                }
            }
            return;
        }
        step -= 1;

        if step < self.stft.synthesise_steps() {
            self.stft.synthesise_step(step);
        }
    }

    fn update_process_spectrum_steps(&mut self) {
        self.process_spectrum_steps = 0;
        if self.block_process.new_spectrum {
            self.process_spectrum_steps += self.channels;
        }
        if self.block_process.mapped_frequencies {
            self.process_spectrum_steps += SMOOTH_ENERGY_STEPS;
            self.process_spectrum_steps += 1;
        }
        self.process_spectrum_steps += 1;
        self.process_spectrum_steps += self.channels;
        self.process_spectrum_steps += SPLIT_MAIN_PREDICTION;
        if self.block_process.new_spectrum {
            self.process_spectrum_steps += 1;
        }
        if self.block_process.process_formants {
            self.process_spectrum_steps += 3;
        }
    }
}
