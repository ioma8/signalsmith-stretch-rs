//! Analysis and synthesis window shapes.

use crate::sample::{Sample, sf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowShape {
    Acg,
    Kaiser,
}

pub(super) fn force_perfect_reconstruction<S: Sample>(
    data: &mut [S],
    window_length: usize,
    interval: usize,
) {
    for i in 0..interval {
        let mut sum2 = 0.0_f64;
        let mut index = i;
        while index < window_length {
            let product = data[index] * data[index];
            sum2 += product.to_f64().expect("sample type");
            index += interval;
        }
        let factor = 1.0 / sum2.sqrt();
        let mut index = i;
        while index < window_length {
            data[index] = sf(data[index].to_f64().expect("sample type") * factor);
            index += interval;
        }
    }
}

pub(super) struct Kaiser {
    beta: f64,
}

impl Kaiser {
    /// Scaled modified Bessel function of the first kind, `exp(-x) * I0(x)`.
    ///
    /// The unscaled series overflows for the high-bandwidth windows that a
    /// caller can request via an extreme block/interval ratio.  Scaling keeps
    /// the window ratio finite and makes the computation terminate.
    fn i0_scaled(x: f64) -> f64 {
        if x <= 20.0 {
            let mut result = 0.0;
            let mut term = 1.0;
            let mut m = 0.0;
            while term > 1e-4 && m < 10_000.0 {
                result += term;
                m += 1.0;
                term *= (x * x) / (4.0 * m * m);
                if !term.is_finite() {
                    break;
                }
            }
            result * (-x).exp()
        } else {
            let inv = 1.0 / x;
            let series = 1.0
                + inv
                    * (0.125
                        + inv
                            * (0.070_312_5 + inv * (0.073_242_187_5 + inv * 0.224_304_199_218_75)));
            series / (std::f64::consts::TAU * x).sqrt()
        }
    }

    fn heuristic_bandwidth(bandwidth: f64) -> f64 {
        bandwidth
            + 8.0 / ((bandwidth + 3.0) * (bandwidth + 3.0))
            + 0.25 * (3.0 - bandwidth).max(0.0)
    }

    pub(super) fn with_bandwidth(bandwidth: f64, heuristic_optimal: bool) -> Self {
        let bandwidth = if heuristic_optimal {
            Self::heuristic_bandwidth(bandwidth)
        } else {
            bandwidth
        };
        let bandwidth = bandwidth.max(2.0);
        let alpha = (bandwidth * bandwidth * 0.25 - 1.0).sqrt();
        let beta = alpha * std::f64::consts::PI;
        Self { beta }
    }

    pub(super) fn fill<S: Sample>(&self, data: &mut [S], is_for_synthesis: bool) {
        let size = data.len();
        let inv_size = 1.0 / size as f64;
        let offset_i = if size.is_multiple_of(2) {
            if is_for_synthesis { 0 } else { 2 }
        } else {
            1
        };
        let beta_scaled = Self::i0_scaled(self.beta);
        for (i, value) in data.iter_mut().enumerate() {
            let r = (2 * i + offset_i) as f64 * inv_size - 1.0;
            let arg = (1.0 - r * r).sqrt();
            let x = self.beta * arg;
            let ratio = Self::i0_scaled(x) / beta_scaled * (-(self.beta - x)).exp();
            *value = sf(ratio);
        }
    }
}

pub(super) struct ApproximateConfinedGaussian {
    gaussian_factor: f64,
}

impl ApproximateConfinedGaussian {
    fn bandwidth_to_sigma(bandwidth: f64) -> f64 {
        0.3 / bandwidth.sqrt()
    }

    pub(super) fn with_bandwidth(bandwidth: f64) -> Self {
        let sigma = Self::bandwidth_to_sigma(bandwidth);
        Self {
            gaussian_factor: 0.0625 / (sigma * sigma),
        }
    }

    fn gaussian(&self, x: f64) -> f64 {
        (-x * x * self.gaussian_factor).exp()
    }

    pub(super) fn fill<S: Sample>(&self, data: &mut [S], is_for_synthesis: bool) {
        let size = data.len();
        let inv_size = 1.0 / size as f64;
        let offset_scale = self.gaussian(1.0) / (self.gaussian(3.0) + self.gaussian(-1.0));
        let norm = 1.0 / (self.gaussian(0.0) - 2.0 * offset_scale * self.gaussian(2.0));
        let offset_i = if size.is_multiple_of(2) {
            if is_for_synthesis { 0 } else { 2 }
        } else {
            1
        };
        for (i, value) in data.iter_mut().enumerate() {
            let r = (2 * i + offset_i) as f64 * inv_size - 1.0;
            *value = sf(norm
                * (self.gaussian(r)
                    - offset_scale * (self.gaussian(r - 2.0) + self.gaussian(r + 2.0))));
        }
    }
}
