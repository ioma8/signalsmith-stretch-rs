//! Phase-vocoder prediction stages.

use super::math::{cadd, cmul, cmul_conj_second};
use super::{NOISE_FLOOR, Prediction, SPLIT_MAIN_PREDICTION, Sample, SignalsmithStretch};
use crate::fft::Complex;
use crate::sample::{floor_isize, sf, si};

impl<S: Sample> SignalsmithStretch<S> {
    pub(super) fn preliminary_prediction(&mut self, channel: usize) {
        let offset = channel * self.bands;
        for b in 0..self.bands {
            let map_point = self.output_map[b];
            let low_index = floor_isize(map_point.input_bin);
            let fractional = map_point.input_bin - si::<S>(low_index);
            let prev_energy = self.channel_predictions[offset + b].energy;
            let mut energy = self.get_fractional_energy(channel, low_index, fractional);
            energy *= map_point.freq_grad.max(S::zero());
            let input = self.get_fractional_input(channel, low_index, fractional);
            let previous_input = self.get_fractional_prev_input(channel, low_index, fractional);
            let freq_twist = cmul_conj_second(input, previous_input);
            let old_output = self.channel_bands[offset + b].output;
            let phase = cmul(old_output, freq_twist);
            self.channel_bands[offset + b].output =
                phase / (prev_energy.max(energy) + sf(NOISE_FLOOR));
            self.channel_predictions[offset + b] = Prediction { energy, input };
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the prediction loop is kept together"
    )]
    pub(super) fn main_prediction(
        &mut self,
        chunk: usize,
        time_factor: S,
        random_time_factor: bool,
        distribute_min: S,
        long_vertical_step: isize,
    ) {
        let start_b = self.bands * chunk / SPLIT_MAIN_PREDICTION;
        let end_b = self.bands * (chunk + 1) / SPLIT_MAIN_PREDICTION;

        for b in start_b..end_b {
            let mut max_channel = 0;
            let mut max_energy = self.channel_predictions[b].energy;
            for c in 1..self.channels {
                let energy = self.channel_predictions[c * self.bands + b].energy;
                if energy > max_energy {
                    max_channel = c;
                    max_energy = energy;
                }
            }

            let prediction = self.channel_predictions[max_channel * self.bands + b];
            let mut phase = Complex::default();
            let map_point = self.output_map[b];

            if b > 0 {
                let bin_time_factor = if random_time_factor {
                    self.random_engine.range(distribute_min, time_factor)
                } else {
                    time_factor
                };
                let down_input = self.get_fractional_input(
                    max_channel,
                    floor_isize(map_point.input_bin - bin_time_factor),
                    map_point.input_bin
                        - bin_time_factor
                        - (map_point.input_bin - bin_time_factor).floor(),
                );
                let short_vertical_twist = cmul_conj_second(prediction.input, down_input);
                let down_bin = self.get_band_output(max_channel, b as isize - 1);
                phase = cadd(phase, cmul(down_bin, short_vertical_twist));

                if b as isize >= long_vertical_step {
                    let long_down_input = self.get_fractional_input(
                        max_channel,
                        floor_isize(
                            map_point.input_bin - si::<S>(long_vertical_step) * bin_time_factor,
                        ),
                        (map_point.input_bin - si::<S>(long_vertical_step) * bin_time_factor)
                            - (map_point.input_bin - si::<S>(long_vertical_step) * bin_time_factor)
                                .floor(),
                    );
                    let long_vertical_twist = cmul_conj_second(prediction.input, long_down_input);
                    let long_down_bin =
                        self.get_band_output(max_channel, b as isize - long_vertical_step);
                    phase = cadd(phase, cmul(long_down_bin, long_vertical_twist));
                }
            }

            if b < self.bands - 1 {
                let up_prediction = self.channel_predictions[max_channel * self.bands + b + 1];
                let up_map_point = self.output_map[b + 1];
                let bin_time_factor = if random_time_factor {
                    self.random_engine.range(distribute_min, time_factor)
                } else {
                    time_factor
                };
                let down_input = self.get_fractional_input(
                    max_channel,
                    floor_isize(up_map_point.input_bin - bin_time_factor),
                    up_map_point.input_bin
                        - bin_time_factor
                        - (up_map_point.input_bin - bin_time_factor).floor(),
                );
                let short_vertical_twist = cmul_conj_second(up_prediction.input, down_input);
                let up_bin = self.get_band_output(max_channel, b as isize + 1);
                phase = cadd(phase, cmul_conj_second(up_bin, short_vertical_twist));

                if (b as isize) < self.bands as isize - long_vertical_step {
                    let long_up_prediction = self.channel_predictions
                        [max_channel * self.bands + b + long_vertical_step as usize];
                    let long_up_map_point = self.output_map[b + long_vertical_step as usize];
                    let long_down_input = self.get_fractional_input(
                        max_channel,
                        floor_isize(
                            long_up_map_point.input_bin
                                - si::<S>(long_vertical_step) * bin_time_factor,
                        ),
                        (long_up_map_point.input_bin
                            - si::<S>(long_vertical_step) * bin_time_factor)
                            - (long_up_map_point.input_bin
                                - si::<S>(long_vertical_step) * bin_time_factor)
                                .floor(),
                    );
                    let long_vertical_twist =
                        cmul_conj_second(long_up_prediction.input, long_down_input);
                    let long_up_bin =
                        self.get_band_output(max_channel, b as isize + long_vertical_step);
                    phase = cadd(phase, cmul_conj_second(long_up_bin, long_vertical_twist));
                }
            }

            let output = prediction.make_output(phase);
            self.channel_bands[max_channel * self.bands + b].output = output;

            for c in 0..self.channels {
                if c != max_channel {
                    let channel_prediction = self.channel_predictions[c * self.bands + b];
                    let channel_twist =
                        cmul_conj_second(channel_prediction.input, prediction.input);
                    let channel_phase = cmul(output, channel_twist);
                    self.channel_bands[c * self.bands + b].output =
                        channel_prediction.make_output(channel_phase);
                }
            }
        }
    }
}
