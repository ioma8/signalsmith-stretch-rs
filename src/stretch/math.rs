//! Small complex-arithmetic helpers used by the DSP stages.

use crate::fft::Complex;
use crate::sample::Sample;

#[inline]
pub(super) fn cadd<S: Sample>(a: Complex<S>, b: Complex<S>) -> Complex<S> {
    Complex::new(a.re + b.re, a.im + b.im)
}

#[inline]
pub(super) fn cmul<S: Sample>(a: Complex<S>, b: Complex<S>) -> Complex<S> {
    Complex::new(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re)
}

/// Complex multiplication with the second operand conjugated.
#[inline]
pub(super) fn cmul_conj_second<S: Sample>(a: Complex<S>, b: Complex<S>) -> Complex<S> {
    Complex::new(b.re * a.re + b.im * a.im, b.re * a.im - b.im * a.re)
}

#[inline]
pub(super) fn cnorm<S: Sample>(a: Complex<S>) -> S {
    a.re * a.re + a.im * a.im
}
