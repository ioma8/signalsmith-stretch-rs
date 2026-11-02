//! Sample-type abstraction used by the public API.

use std::fmt::Debug;
use std::ops::{AddAssign, DivAssign, MulAssign, SubAssign};

use num_traits::{Float, FromPrimitive, ToPrimitive};

mod sealed {
    pub trait Sealed {}
}

/// Floating-point sample type accepted by [`crate::SignalsmithStretch`].
///
/// Only [`f32`] is supported by the `microfft` backend.
pub trait Sample:
    sealed::Sealed
    + Float
    + FromPrimitive
    + ToPrimitive
    + Debug
    + Default
    + AddAssign
    + SubAssign
    + MulAssign
    + DivAssign
    + Send
    + Sync
    + 'static
{
}

impl sealed::Sealed for f32 {}

impl Sample for f32 {}

#[inline]
pub(crate) fn sf<S: Sample>(value: f64) -> S {
    S::from_f64(value).expect("sample type cannot represent f64")
}

#[inline]
pub(crate) fn su<S: Sample>(value: usize) -> S {
    S::from_usize(value).expect("sample type cannot represent usize")
}

#[inline]
pub(crate) fn si<S: Sample>(value: isize) -> S {
    S::from_isize(value).expect("sample type cannot represent isize")
}

#[inline]
pub(crate) fn saturating_usize<S: Sample>(value: S) -> usize {
    if value.is_nan() || value <= S::zero() {
        0
    } else if value >= S::from_usize(usize::MAX).expect("sample type") {
        usize::MAX
    } else {
        value.to_usize().unwrap_or(usize::MAX)
    }
}

#[inline]
pub(crate) fn saturating_isize<S: Sample>(value: S) -> isize {
    if value.is_nan() {
        0
    } else if value <= S::from_isize(isize::MIN).expect("sample type") {
        isize::MIN
    } else if value >= S::from_isize(isize::MAX).expect("sample type") {
        isize::MAX
    } else {
        value.to_isize().unwrap_or_else(|| {
            if value < S::zero() {
                isize::MIN
            } else {
                isize::MAX
            }
        })
    }
}

#[inline]
pub(crate) fn round_usize<S: Sample>(value: S) -> usize {
    saturating_usize(value.round())
}

#[inline]
pub(crate) fn round_isize<S: Sample>(value: S) -> isize {
    saturating_isize(value.round())
}

#[inline]
pub(crate) fn ceil_isize<S: Sample>(value: S) -> isize {
    saturating_isize(value.ceil())
}

#[inline]
pub(crate) fn floor_isize<S: Sample>(value: S) -> isize {
    saturating_isize(value.floor())
}
