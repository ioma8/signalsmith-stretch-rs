//! Safe Rust audio pitch/time stretching.
//!
//! Construct a [`SignalsmithStretch`], configure it with a preset or explicit
//! block sizes, then feed planar input and output buffers to
//! [`SignalsmithStretch::process`].
//!
//! The implementation contains no `unsafe`, uses `microfft` for the FFT
//! primitives, and is deterministic for a given seed and configuration.
//! With the `no_std` feature it uses `alloc` and `libm`; the CLI requires
//! `std`.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

extern crate alloc;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme_doctests {}

mod fft;
mod sample;
mod stft;
mod stretch;

pub use sample::Sample;
pub use stretch::{MAX_BLOCK_SAMPLES, MAX_CHANNELS, SignalsmithStretch, VERSION};
