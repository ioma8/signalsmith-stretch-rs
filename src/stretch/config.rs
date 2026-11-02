//! Configuration, presets, and parameter setters.

use alloc::sync::Arc;
#[cfg(not(feature = "std"))]
use alloc::vec;
use alloc::vec::Vec;

use super::random::Pcg32;
use super::{
    Band, BlockProcess, MAX_BAND_CHANNEL_ENTRIES, MAX_BLOCK_SAMPLES, MAX_CHANNELS, PitchMapPoint,
    Prediction, SignalsmithStretch, preset_samples,
};
use crate::fft::RealFft;
use crate::sample::{Sample, sf};
use crate::stft::{DynamicStft, WindowShape};

impl<S: Sample> SignalsmithStretch<S> {
    /// Input latency in samples.
    ///
    /// Returns zero before [`Self::configure`] or a preset has been called.
    #[must_use]
    pub fn input_latency(&self) -> usize {
        if self.is_configured() {
            self.stft.analysis_latency()
        } else {
            0
        }
    }

    /// Output latency in samples.
    ///
    /// Returns zero before [`Self::configure`] or a preset has been called.
    #[must_use]
    pub fn output_latency(&self) -> usize {
        if self.is_configured() {
            self.stft.synthesis_latency()
                + usize::from(self.split_computation) * self.stft.default_interval()
        } else {
            0
        }
    }

    /// Clears all streaming state and restores the seeded random stream.
    ///
    /// Does nothing except restore the random stream when the stretcher has not
    /// been configured yet.
    pub fn reset(&mut self) {
        self.random_engine = Pcg32::new(self.seed);
        if !self.is_configured() {
            return;
        }
        self.stft.reset(sf(0.1));
        self.stashed_input.clone_from(&self.stft.input);
        self.stashed_output.clone_from(&self.stft.output);
        self.prev_input_offset = -1;
        self.channel_bands.fill(Band::default());
        self.silence_counter = 0;
        self.silence_first = true;
        self.did_seek = false;
        self.seek_time_factor = S::one();
        self.block_process = BlockProcess::default();
        self.freq_estimate_weighted = S::zero();
        self.freq_estimate_weight = S::zero();
        self.freq_estimate = S::zero();
        self.smooth_energy_state = S::zero();
    }

    /// Configures the default 120 ms block / 30 ms interval preset.
    ///
    /// `sample_rate` is in Hz.
    ///
    /// # Panics
    ///
    /// Panics if `sample_rate` is not finite and positive, or if the resulting
    /// block size is outside `2..=`[`MAX_BLOCK_SAMPLES`].
    pub fn preset_default(&mut self, channels: usize, sample_rate: S, split_computation: bool) {
        self.configure(
            channels,
            preset_samples(sample_rate, 0.12, 2.0),
            preset_samples(sample_rate, 0.03, 1.0),
            split_computation,
        );
    }

    /// Configures the cheaper 100 ms block / 40 ms interval preset.
    ///
    /// `sample_rate` is in Hz.
    ///
    /// # Panics
    ///
    /// Panics if `sample_rate` is not finite and positive, or if the resulting
    /// block size is outside `2..=`[`MAX_BLOCK_SAMPLES`].
    pub fn preset_cheaper(&mut self, channels: usize, sample_rate: S, split_computation: bool) {
        self.configure(
            channels,
            preset_samples(sample_rate, 0.1, 2.0),
            preset_samples(sample_rate, 0.04, 1.0),
            split_computation,
        );
    }

    /// Configures block and interval sizes explicitly.
    ///
    /// Configuration is transactional: if an assertion fails, the previous
    /// configuration and streaming state are left unchanged.
    ///
    /// # Panics
    ///
    /// Panics if:
    ///
    /// - `channels` is zero or greater than [`MAX_CHANNELS`],
    /// - `block_samples` is outside `2..=`[`MAX_BLOCK_SAMPLES`],
    /// - `interval_samples` is zero or greater than `block_samples`,
    /// - or `channels * spectral_bands` exceeds the internal safety limit.
    pub fn configure(
        &mut self,
        channels: usize,
        block_samples: usize,
        interval_samples: usize,
        split_computation: bool,
    ) {
        assert!(channels > 0, "at least one channel is required");
        assert!(
            channels <= MAX_CHANNELS,
            "channel count exceeds the supported limit of {MAX_CHANNELS}"
        );
        assert!(block_samples >= 2, "block size must be at least 2");
        assert!(
            block_samples <= MAX_BLOCK_SAMPLES,
            "block size exceeds the supported limit of {MAX_BLOCK_SAMPLES}"
        );
        assert!(interval_samples > 0, "interval must be positive");
        assert!(interval_samples <= block_samples, "interval exceeds block");

        let bands = RealFft::<S>::real_fft_size(block_samples) / 2;
        assert!(bands > 0, "configuration produced no spectral bands");
        let band_entries = channels
            .checked_mul(bands)
            .expect("channel/band product overflow");
        assert!(
            band_entries <= MAX_BAND_CHANNEL_ENTRIES,
            "channel/band product exceeds the internal limit"
        );

        let mut stft = DynamicStft::new(
            channels,
            channels,
            block_samples,
            interval_samples + 1,
            0,
            0,
        );
        stft.set_interval(interval_samples, WindowShape::Kaiser);
        stft.reset(sf(0.1));
        debug_assert_eq!(bands, stft.bands());
        let stashed_input = stft.input.clone();
        let stashed_output = stft.output.clone();
        let output_latency =
            stft.synthesis_latency() + usize::from(split_computation) * interval_samples;

        let channel_bands = vec![Band::default(); band_entries];
        let peaks = Vec::with_capacity(bands / 2);
        let energy = vec![S::zero(); bands];
        let smoothed_energy = vec![S::zero(); bands];
        let output_map = vec![PitchMapPoint::default(); bands];
        let channel_predictions = vec![Prediction::default(); band_entries];
        let formant_metric = vec![S::zero(); bands + 2];
        let process_buffer_len = block_samples
            .checked_add(interval_samples)
            .expect("process buffer size overflow");
        let pre_roll_len = output_latency
            .checked_mul(channels)
            .expect("pre-roll buffer size overflow");
        let tmp_process_buffer = vec![S::zero(); process_buffer_len];
        let tmp_pre_roll_buffer = vec![S::zero(); pre_roll_len];

        self.split_computation = split_computation;
        self.stft = stft;
        self.stashed_input = stashed_input;
        self.stashed_output = stashed_output;
        self.channels = channels;
        self.bands = bands;
        self.channel_bands = channel_bands;
        self.peaks = peaks;
        self.energy = energy;
        self.smoothed_energy = smoothed_energy;
        self.output_map = output_map;
        self.channel_predictions = channel_predictions;
        self.formant_metric = formant_metric;
        self.tmp_process_buffer = tmp_process_buffer;
        self.tmp_pre_roll_buffer = tmp_pre_roll_buffer;
        self.block_process = BlockProcess::default();
        self.prev_input_offset = -1;
        self.silence_counter = 0;
        self.silence_first = true;
        self.did_seek = false;
        self.seek_time_factor = S::one();
        self.freq_estimate_weighted = S::zero();
        self.freq_estimate_weight = S::zero();
        self.freq_estimate = S::zero();
        self.smooth_energy_state = S::zero();
        self.random_engine = Pcg32::new(self.seed);
    }

    /// Configured STFT block size, or zero before configuration.
    #[must_use]
    pub fn block_samples(&self) -> usize {
        if self.is_configured() {
            self.stft.block_samples()
        } else {
            0
        }
    }

    /// Configured STFT interval size, or zero before configuration.
    #[must_use]
    pub fn interval_samples(&self) -> usize {
        if self.is_configured() {
            self.stft.default_interval()
        } else {
            0
        }
    }

    /// Whether split computation is enabled.
    #[must_use]
    pub fn split_computation(&self) -> bool {
        self.is_configured() && self.split_computation
    }

    /// Sets a frequency multiplier and optional tonality limit.
    ///
    /// `tonality_limit` is normalized to the sample rate; `0` disables the
    /// limit.
    ///
    /// # Panics
    ///
    /// Panics if `multiplier` is not finite and positive, or if
    /// `tonality_limit` is not finite or is negative.
    pub fn set_transpose_factor(&mut self, multiplier: S, tonality_limit: S) {
        assert!(
            multiplier.is_finite() && multiplier > S::zero(),
            "transpose multiplier must be finite and positive"
        );
        assert!(
            tonality_limit.is_finite() && tonality_limit >= S::zero(),
            "tonality limit must be finite and non-negative"
        );
        self.freq_multiplier = multiplier;
        self.freq_tonality_limit = if tonality_limit > S::zero() {
            tonality_limit / multiplier.sqrt()
        } else {
            S::one()
        };
        self.custom_freq_map = None;
    }

    /// Sets a pitch shift in semitones and an optional tonality limit.
    ///
    /// # Panics
    ///
    /// Panics if `semitones` or `tonality_limit` is not finite.
    pub fn set_transpose_semitones(&mut self, semitones: S, tonality_limit: S) {
        assert!(semitones.is_finite(), "semitones must be finite");
        self.set_transpose_factor(sf::<S>(2.0).powf(semitones / sf(12.0)), tonality_limit);
    }

    /// Sets a custom monotonically increasing input-to-output frequency map.
    pub fn set_freq_map<F>(&mut self, input_to_output: F)
    where
        F: Fn(S) -> S + Send + Sync + 'static,
    {
        self.custom_freq_map = Some(Arc::new(input_to_output));
    }

    /// Sets a formant multiplier.
    ///
    /// # Panics
    ///
    /// Panics if `multiplier` is not finite and positive.
    pub fn set_formant_factor(&mut self, multiplier: S, compensate_pitch: bool) {
        assert!(
            multiplier.is_finite() && multiplier > S::zero(),
            "formant multiplier must be finite and positive"
        );
        self.formant_multiplier = multiplier;
        self.inv_formant_multiplier = S::one() / multiplier;
        self.formant_compensation = compensate_pitch;
    }

    /// Sets a formant shift in semitones.
    ///
    /// # Panics
    ///
    /// Panics if `semitones` is not finite.
    pub fn set_formant_semitones(&mut self, semitones: S, compensate_pitch: bool) {
        assert!(semitones.is_finite(), "semitones must be finite");
        self.set_formant_factor(sf::<S>(2.0).powf(semitones / sf(12.0)), compensate_pitch);
    }

    /// Sets a rough fundamental frequency, or zero for automatic estimation.
    ///
    /// `base_freq` is normalized to the sample rate.
    ///
    /// # Panics
    ///
    /// Panics if `base_freq` is not finite or is negative.
    pub fn set_formant_base(&mut self, base_freq: S) {
        assert!(
            base_freq.is_finite() && base_freq >= S::zero(),
            "formant base frequency must be finite and non-negative"
        );
        self.formant_base_freq = base_freq;
    }
}
