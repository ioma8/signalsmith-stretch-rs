//! Complete fixed-length example, also compiled by `cargo test --all-targets`.
//!
//! Run with:
//! `cargo run --release --example basic`

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    reason = "example converts sample indices to time"
)]

use signalsmith_stretch::SignalsmithStretch;

fn main() {
    let sample_rate = 48_000.0_f32;
    let input_frames = 4_800;
    let output_frames = 7_200;

    let mut left = vec![0.0_f32; input_frames];
    let mut right = vec![0.0_f32; input_frames];
    for (index, (left, right)) in left.iter_mut().zip(&mut right).enumerate() {
        let phase = std::f32::consts::TAU * 220.0 * index as f32 / sample_rate;
        *left = phase.sin() * 0.3;
        *right = (phase * 1.01).cos() * 0.3;
    }

    let mut left_out = vec![0.0_f32; output_frames];
    let mut right_out = vec![0.0_f32; output_frames];
    let input: [&[f32]; 2] = [&left, &right];
    let mut output: [&mut [f32]; 2] = [&mut left_out, &mut right_out];

    let mut stretch = SignalsmithStretch::<f32>::with_seed(12345);
    stretch.preset_default(2, sample_rate, false);
    stretch.set_transpose_semitones(4.0, 8_000.0 / sample_rate);
    stretch.set_formant_semitones(2.0, false);
    stretch.set_formant_base(120.0 / sample_rate);

    let rendered = stretch.exact(&input, input_frames, &mut output, output_frames);
    assert!(rendered, "the example input is long enough");
    assert!(
        left_out
            .iter()
            .chain(&right_out)
            .all(|sample| sample.is_finite())
    );

    println!(
        "configured block={} interval={} input_latency={} output_latency={}",
        stretch.block_samples(),
        stretch.interval_samples(),
        stretch.input_latency(),
        stretch.output_latency()
    );
}
