//! Small throughput probe for the release build.
//!
//! Run with:
//! `cargo run --release --example bench`

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "benchmark setup uses explicit frame-count conversions"
)]

use std::hint::black_box;
use std::time::Instant;

use signalsmith_stretch::SignalsmithStretch;

fn main() {
    let sample_rate = 48_000.0_f32;
    let input_frames = 48_000 * 5;
    let output_frames = input_frames * 3 / 2;
    let mut left = vec![0.0_f32; input_frames];
    let mut right = vec![0.0_f32; input_frames];
    for (index, (left, right)) in left.iter_mut().zip(&mut right).enumerate() {
        let phase = std::f32::consts::TAU * 220.0 * index as f32 / sample_rate;
        *left = phase.sin() * 0.3;
        *right = (phase * 1.01).cos() * 0.3;
    }

    let mut left_out = vec![0.0_f32; output_frames];
    let mut right_out = vec![0.0_f32; output_frames];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.preset_default(2, sample_rate, false);
    stretch.set_transpose_semitones(3.0, 8_000.0 / sample_rate);

    let start = Instant::now();
    let rendered = stretch.exact(
        &[&left, &right],
        input_frames,
        &mut [&mut left_out, &mut right_out],
        output_frames,
    );
    let elapsed = start.elapsed();
    black_box((&left_out, &right_out));
    assert!(rendered);
    let audio_seconds = output_frames as f64 / f64::from(sample_rate);
    println!(
        "rendered {audio_seconds:.2}s of stereo audio in {:.3}s ({:.1}x realtime)",
        elapsed.as_secs_f64(),
        audio_seconds / elapsed.as_secs_f64()
    );
}
