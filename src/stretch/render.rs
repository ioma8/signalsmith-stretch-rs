//! Streaming, seeking, flushing, and fixed-length rendering.

use super::{NOISE_FLOOR, SignalsmithStretch};
use crate::fft::Complex;
use crate::sample::{Sample, saturating_usize, sf, su};

impl<S: Sample> SignalsmithStretch<S> {
    /// Number of input frames expected by [`Self::seek`], or zero before configuration.
    #[must_use]
    pub fn seek_length(&self) -> usize {
        if self.is_configured() {
            self.stft.block_samples() + self.stft.default_interval()
        } else {
            0
        }
    }

    /// Length of input needed for [`Self::output_seek`] at a playback rate.
    ///
    /// Returns zero before configuration.
    ///
    /// Returns zero for a negative or NaN playback rate, and
    /// [`usize::MAX`] when a positive finite or infinite product overflows.
    #[must_use]
    pub fn output_seek_length(&self, playback_rate: S) -> usize {
        if !self.is_configured() || playback_rate.is_nan() || playback_rate < S::zero() {
            return 0;
        }
        let length = su::<S>(self.input_latency()) + playback_rate * su::<S>(self.output_latency());
        saturating_usize(length.max(S::zero()))
    }

    /// Supplies pre-roll input without producing output.
    ///
    /// # Panics
    ///
    /// Panics if the stretcher is not configured, the channel count or a
    /// supplied channel length is too short, or `playback_rate` is not finite
    /// and non-negative.
    pub fn seek(&mut self, inputs: &[&[S]], input_samples: usize, playback_rate: S) {
        assert!(
            self.is_configured(),
            "configure() or a preset must be called first"
        );
        assert!(
            playback_rate.is_finite() && playback_rate >= S::zero(),
            "playback rate must be finite and non-negative"
        );
        assert_eq!(inputs.len(), self.channels, "channel count mismatch");
        for channel in inputs {
            assert!(
                channel.len() >= input_samples,
                "input buffer shorter than input_samples"
            );
        }
        let buffer_length = self.seek_length();
        self.tmp_process_buffer.resize(buffer_length, S::zero());
        self.tmp_process_buffer.fill(S::zero());

        let start_index = input_samples.saturating_sub(buffer_length);
        let pad_start = buffer_length + start_index - input_samples;

        let mut total_energy = S::zero();
        for c in 0..self.channels {
            let input_channel = inputs[c];
            for i in start_index..input_samples {
                let sample = input_channel[i];
                total_energy += sample * sample;
                self.tmp_process_buffer[i - start_index + pad_start] = sample;
            }
            self.stft.write_input(c, 0, &self.tmp_process_buffer);
        }
        self.stft.move_input(buffer_length);
        if total_energy >= sf(NOISE_FLOOR) {
            self.silence_counter = 0;
            self.silence_first = true;
        }
        self.did_seek = true;
        let interval = su::<S>(self.stft.default_interval());
        self.seek_time_factor = if playback_rate * interval > S::one() {
            S::one() / playback_rate
        } else {
            interval
        };
    }

    /// Moves playback to a new input position and pre-computes aligned output.
    ///
    /// # Panics
    ///
    /// Panics if the stretcher is not configured, the configured output latency
    /// is zero, or an input channel is shorter than the required seek range.
    pub fn output_seek(&mut self, inputs: &[&[S]], input_length: usize) {
        assert!(
            self.is_configured(),
            "configure() or a preset must be called first"
        );
        self.reset();
        let surplus_input = input_length.saturating_sub(self.input_latency());
        let output_latency = self.output_latency();
        assert!(output_latency > 0, "output latency must be positive");
        let playback_rate = su::<S>(surplus_input) / su::<S>(output_latency);
        let seek_samples = input_length - surplus_input;

        self.seek(inputs, seek_samples, playback_rate);

        let mut pre_roll = std::mem::take(&mut self.tmp_pre_roll_buffer);
        pre_roll.resize(output_latency * self.channels, S::zero());
        {
            let mut output_channels: Vec<&mut [S]> = pre_roll.chunks_mut(output_latency).collect();
            self.process_inner(
                Some(inputs),
                seek_samples,
                surplus_input,
                &mut output_channels,
                output_latency,
            );
        }

        for value in &mut pre_roll {
            *value = -*value;
        }
        for c in 0..self.channels {
            let channel = &mut pre_roll[c * output_latency..(c + 1) * output_latency];
            channel.reverse();
            self.stft.add_output(c, channel);
        }
        self.tmp_pre_roll_buffer = pre_roll;
    }

    /// Processes planar input into planar output.
    ///
    /// # Panics
    ///
    /// Panics if the stretcher is not configured, the input/output channel
    /// counts do not match, or a buffer is shorter than its requested length.
    pub fn process(
        &mut self,
        inputs: &[&[S]],
        input_samples: usize,
        outputs: &mut [&mut [S]],
        output_samples: usize,
    ) {
        self.process_inner(Some(inputs), 0, input_samples, outputs, output_samples);
    }

    /// Reads the remaining output, assuming zero input for long flushes.
    ///
    /// # Panics
    ///
    /// Panics if the stretcher is not configured, the output channel count does
    /// not match, a buffer is shorter than `output_samples`, or `playback_rate`
    /// is not finite and non-negative.
    pub fn flush(&mut self, outputs: &mut [&mut [S]], output_samples: usize, playback_rate: S) {
        assert!(
            self.is_configured(),
            "configure() or a preset must be called first"
        );
        assert!(
            playback_rate.is_finite() && playback_rate >= S::zero(),
            "playback rate must be finite and non-negative"
        );
        assert_eq!(outputs.len(), self.channels, "channel count mismatch");
        for channel in outputs.iter() {
            assert!(
                channel.len() >= output_samples,
                "output buffer shorter than output_samples"
            );
        }

        let interval = self.stft.default_interval();
        let output_block = output_samples.saturating_sub(interval);
        if output_block > 0 {
            let input_samples = saturating_usize(su::<S>(output_block) * playback_rate);
            self.process_inner(None, 0, input_samples, outputs, output_block);
        }

        let tail_samples = output_samples - output_block;
        let mut tail = std::mem::take(&mut self.tmp_process_buffer);
        tail.resize(tail_samples, S::zero());
        self.stft.finish_output(S::one(), 0);
        for c in 0..self.channels {
            self.stft.read_output(c, 0, &mut tail[..tail_samples]);
            outputs[c][output_block..output_block + tail_samples]
                .copy_from_slice(&tail[..tail_samples]);
            self.stft
                .read_output(c, tail_samples, &mut tail[..tail_samples]);
            for i in 0..tail_samples {
                outputs[c][output_block + tail_samples - 1 - i] -= tail[i];
            }
        }
        self.tmp_process_buffer = tail;

        self.stft.reset(sf(0.1));
        for c in 0..self.channels {
            let offset = c * self.bands;
            for b in 0..self.bands {
                self.channel_bands[offset + b].prev_input = Complex::default();
                self.channel_bands[offset + b].output = Complex::default();
            }
        }
    }

    /// Processes an entire fixed-length buffer, returning false if too short.
    ///
    /// # Panics
    ///
    /// Panics if `output_samples == 0`, if the stretcher is not configured, or
    /// if any buffer is shorter than its requested length.
    #[must_use]
    pub fn exact(
        &mut self,
        inputs: &[&[S]],
        input_samples: usize,
        outputs: &mut [&mut [S]],
        output_samples: usize,
    ) -> bool {
        assert!(
            self.is_configured(),
            "configure() or a preset must be called first"
        );
        assert!(output_samples > 0, "output length must be positive");
        let playback_rate = su::<S>(input_samples) / su::<S>(output_samples);
        let seek_length = self.output_seek_length(playback_rate);
        if input_samples < seek_length {
            for channel in outputs.iter_mut() {
                channel[..output_samples].fill(S::zero());
            }
            return false;
        }

        self.output_seek(inputs, seek_length);
        let output_index =
            saturating_usize(su::<S>(output_samples) - su::<S>(seek_length) / playback_rate)
                .min(output_samples);
        self.process_inner(
            Some(inputs),
            seek_length,
            input_samples - seek_length,
            outputs,
            output_index,
        );

        let mut tail_outputs: Vec<&mut [S]> = outputs
            .iter_mut()
            .map(|channel| &mut channel[output_index..output_samples])
            .collect();
        self.flush(
            &mut tail_outputs,
            output_samples - output_index,
            playback_rate,
        );
        true
    }
}
