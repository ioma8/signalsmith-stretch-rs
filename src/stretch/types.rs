//! Internal state types shared by the processing stages.

use crate::fft::Complex;
use crate::sample::{Sample, sf};

use super::NOISE_FLOOR;
use super::math::cnorm;

#[derive(Clone, Copy, Default)]
pub(super) struct Band<S> {
    pub(super) input: Complex<S>,
    pub(super) prev_input: Complex<S>,
    pub(super) output: Complex<S>,
    pub(super) input_energy: S,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Prediction<S> {
    pub(super) energy: S,
    pub(super) input: Complex<S>,
}

impl<S: Sample> Prediction<S> {
    pub(super) fn make_output(self, mut phase: Complex<S>) -> Complex<S> {
        let mut phase_norm = cnorm(phase);
        if phase_norm <= sf(NOISE_FLOOR) {
            phase = self.input;
            phase_norm = cnorm(self.input) + sf(NOISE_FLOOR);
        }
        phase * (self.energy / phase_norm).sqrt()
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Peak<S> {
    pub(super) input: S,
    pub(super) output: S,
}

#[derive(Clone, Copy, Default)]
pub(super) struct PitchMapPoint<S> {
    pub(super) input_bin: S,
    pub(super) freq_grad: S,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "direct processing state layout"
)]
pub(super) struct BlockProcess<S> {
    pub(super) samples_since_last: usize,
    pub(super) steps: usize,
    pub(super) step: usize,
    pub(super) new_spectrum: bool,
    pub(super) reanalyse_prev: bool,
    pub(super) mapped_frequencies: bool,
    pub(super) process_formants: bool,
    pub(super) time_factor: S,
}

impl<S: Sample> Default for BlockProcess<S> {
    fn default() -> Self {
        Self {
            samples_since_last: usize::MAX,
            steps: 0,
            step: 0,
            new_spectrum: false,
            reanalyse_prev: false,
            mapped_frequencies: false,
            process_formants: false,
            time_factor: S::one(),
        }
    }
}
