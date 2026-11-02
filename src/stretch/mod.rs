//! Pitch/time stretching algorithm.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "DSP indexing and numerical structure are intentional"
)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::sample::{Sample, sf};
use crate::stft::{DynamicStft, StftInput, StftOutput};

mod config;
mod formant;
mod math;
mod predict;
mod process;
mod random;
mod render;
mod spectrum;
mod types;

use random::Pcg32;
use types::{Band, BlockProcess, Peak, PitchMapPoint, Prediction};

/// Library version.
pub const VERSION: [usize; 3] = [1, 3, 2];

const NOISE_FLOOR: f64 = 1e-15;
const MAX_CLEAN_STRETCH: f64 = 2.0;
const SPLIT_MAIN_PREDICTION: usize = 8;
const SMOOTH_ENERGY_STEPS: usize = 3;

/// Largest accepted STFT block size.
///
/// This bounds the eager working-set allocation performed by [`SignalsmithStretch::configure`].
/// The WAV CLI applies the same limit.
pub const MAX_BLOCK_SAMPLES: usize = 1 << 20;

/// Largest accepted channel count.
pub const MAX_CHANNELS: usize = 256;

/// Largest accepted number of `channels * spectral bands` entries.
///
/// This keeps the per-channel band tables from growing without bound for
/// configurations that combine many channels with a very large block.
const MAX_BAND_CHANNEL_ENTRIES: usize = 4_000_000;

fn preset_samples<S: Sample>(sample_rate: S, seconds: f64, minimum: f64) -> usize {
    let rate = sample_rate.to_f64().expect("sample type");
    assert!(
        rate.is_finite() && rate > 0.0,
        "sample rate must be finite and positive"
    );
    let samples = rate * seconds;
    assert!(
        (minimum..=MAX_BLOCK_SAMPLES as f64).contains(&samples),
        "preset size must be between {minimum} and {MAX_BLOCK_SAMPLES} samples"
    );
    samples as usize
}

/// Pitch and time stretcher.
///
/// Input and output buffers are planar: one slice per channel.  They may be
/// different sizes, which is how time stretching is expressed.
#[allow(
    clippy::struct_excessive_bools,
    reason = "configuration and streaming flags mirror the processing state"
)]
pub struct SignalsmithStretch<S: Sample = f32> {
    seed: u64,
    split_computation: bool,
    block_process: BlockProcess<S>,
    silence_counter: usize,
    silence_first: bool,

    freq_multiplier: S,
    freq_tonality_limit: S,
    custom_freq_map: Option<Arc<dyn Fn(S) -> S + Send + Sync>>,

    formant_compensation: bool,
    formant_multiplier: S,
    inv_formant_multiplier: S,

    stft: DynamicStft<S>,
    stashed_input: StftInput<S>,
    stashed_output: StftOutput<S>,

    tmp_process_buffer: Vec<S>,
    tmp_pre_roll_buffer: Vec<S>,

    channels: usize,
    bands: usize,
    prev_input_offset: isize,
    did_seek: bool,
    seek_time_factor: S,

    channel_bands: Vec<Band<S>>,
    peaks: Vec<Peak<S>>,
    energy: Vec<S>,
    smoothed_energy: Vec<S>,
    output_map: Vec<PitchMapPoint<S>>,
    channel_predictions: Vec<Prediction<S>>,

    random_engine: Pcg32,
    process_spectrum_steps: usize,

    freq_estimate_weighted: S,
    freq_estimate_weight: S,
    freq_estimate: S,
    smooth_energy_state: S,
    formant_metric: Vec<S>,
    formant_base_freq: S,
}

impl<S: Sample> Default for SignalsmithStretch<S> {
    fn default() -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64);
        Self::with_seed(seed)
    }
}

impl<S: Sample> SignalsmithStretch<S> {
    pub(super) fn is_configured(&self) -> bool {
        self.channels != 0
    }

    /// Creates a stretcher with a time-derived random seed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stretcher with an explicit, reproducible random seed.
    #[must_use]
    pub fn with_seed(seed: u64) -> Self {
        let stft = DynamicStft::new(0, 0, 4, 0, 0, 0);
        let stashed_input = stft.input.clone();
        let stashed_output = stft.output.clone();
        Self {
            seed,
            split_computation: false,
            block_process: BlockProcess::default(),
            silence_counter: 0,
            silence_first: true,
            freq_multiplier: S::one(),
            freq_tonality_limit: sf(0.5),
            custom_freq_map: None,
            formant_compensation: false,
            formant_multiplier: S::one(),
            inv_formant_multiplier: S::one(),
            stft,
            stashed_input,
            stashed_output,
            tmp_process_buffer: Vec::new(),
            tmp_pre_roll_buffer: Vec::new(),
            channels: 0,
            bands: 0,
            prev_input_offset: -1,
            did_seek: false,
            seek_time_factor: S::one(),
            channel_bands: Vec::new(),
            peaks: Vec::new(),
            energy: Vec::new(),
            smoothed_energy: Vec::new(),
            output_map: Vec::new(),
            channel_predictions: Vec::new(),
            random_engine: Pcg32::new(seed),
            process_spectrum_steps: 0,
            freq_estimate_weighted: S::zero(),
            freq_estimate_weight: S::zero(),
            freq_estimate: S::zero(),
            smooth_energy_state: S::zero(),
            formant_metric: Vec::new(),
            formant_base_freq: S::zero(),
        }
    }
}

#[cfg(test)]
mod tests;
