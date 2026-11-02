use super::*;

fn sine_input(length: usize, frequency: f32) -> Vec<f32> {
    (0..length)
        .map(|i| (std::f32::consts::TAU * frequency * i as f32 / 1000.0).sin() * 0.4)
        .collect()
}

#[test]
fn unity_exact_matches_input() {
    let input = sine_input(2048, 173.0);
    let mut output = vec![0.0_f32; input.len()];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(1, 64, 16, false);
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut output], input.len()));

    let mut error = 0.0_f64;
    let mut energy = 0.0_f64;
    for (actual, expected) in output[100..input.len() - 100]
        .iter()
        .zip(&input[100..input.len() - 100])
    {
        let difference = f64::from(actual - expected);
        error += difference * difference;
        energy += f64::from(*expected) * f64::from(*expected);
    }
    assert!((error / energy).sqrt() < 1e-5);
}

#[test]
fn reset_reproduces_time_stretch() {
    let input = sine_input(2048, 211.0);
    let mut first = vec![0.0_f32; 3072];
    let mut second = vec![0.0_f32; 3072];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(11);
    stretch.configure(1, 64, 16, false);
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut first], 3072));
    stretch.reset();
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut second], 3072));
    assert_eq!(first, second);
}

#[test]
fn f64_processing_is_supported() {
    let input: Vec<f64> = (0..1024)
        .map(|i| (std::f64::consts::TAU * 190.0 * f64::from(i) / 1000.0).sin() * 0.3)
        .collect();
    let mut output = vec![0.0_f64; 1536];
    let mut stretch = SignalsmithStretch::<f64>::with_seed(13);
    stretch.configure(1, 64, 16, false);
    stretch.set_transpose_semitones(3.0, 0.8);
    let output_len = output.len();
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut output], output_len));
    assert!(output.iter().all(|sample| sample.is_finite()));
    assert!(output.iter().any(|sample| sample.abs() > 1e-4));
}

#[test]
fn extreme_stretch_is_seeded() {
    let input = sine_input(2048, 137.0);
    let mut first = vec![0.0_f32; 4096];
    let mut second = vec![0.0_f32; 4096];
    let mut a = SignalsmithStretch::<f32>::with_seed(99);
    a.configure(1, 64, 16, false);
    assert!(a.exact(&[&input], input.len(), &mut [&mut first], 4096));
    let mut b = SignalsmithStretch::<f32>::with_seed(99);
    b.configure(1, 64, 16, false);
    assert!(b.exact(&[&input], input.len(), &mut [&mut second], 4096));
    assert_eq!(first, second);
    assert!(first.iter().any(|sample| sample.abs() > 1e-4));
}

#[test]
fn tiny_blocks_configure() {
    for block in 2..=4 {
        let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
        stretch.configure(1, block, 1, false);
        assert_eq!(stretch.block_samples(), block);
        let input = sine_input(64, 200.0);
        let mut output = vec![0.0_f32; 64];
        assert!(stretch.exact(&[&input], input.len(), &mut [&mut output], 64));
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
}

#[test]
fn extreme_bandwidth_terminates() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(1, 512, 1, false);
    let input = sine_input(1024, 220.0);
    let mut output = vec![0.0_f32; 1024];
    let output_len = output.len();
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut output], output_len));
    assert!(output.iter().all(|sample| sample.is_finite()));
}

#[test]
fn zero_input_from_reset_is_silent() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(1, 128, 32, false);
    let mut output = vec![0.0_f32; 100];
    stretch.process(&[&[]], 0, &mut [&mut output], 100);
    assert!(output.iter().all(|sample| *sample == 0.0));
}

#[test]
fn zero_input_midstream_stays_finite() {
    // Finite is the guarantee for this path; see README numerical limits.
    let input = sine_input(2000, 180.0);
    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(1, 128, 32, false);
    stretch.set_transpose_semitones(3.0, 0.8);
    let mut output = vec![0.0_f32; 250];
    stretch.process(&[&input[..100]], 100, &mut [&mut output[..150]], 150);
    stretch.process(&[&[]], 0, &mut [&mut output[150..]], 100);
    assert!(output.iter().all(|sample| sample.is_finite()));
}

#[test]
fn exact_rejects_short_input() {
    let input = sine_input(32, 200.0);
    let mut output = vec![0.0_f32; 64];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(17);
    stretch.configure(1, 64, 16, false);
    assert!(!stretch.exact(&[&input], input.len(), &mut [&mut output], 64));
    assert!(output.iter().all(|sample| *sample == 0.0));
}

#[test]
fn unconfigured_queries_are_zero() {
    let stretch = SignalsmithStretch::<f32>::with_seed(1);
    assert_eq!(stretch.block_samples(), 0);
    assert_eq!(stretch.interval_samples(), 0);
    assert_eq!(stretch.input_latency(), 0);
    assert_eq!(stretch.output_latency(), 0);
    assert_eq!(stretch.seek_length(), 0);
    assert_eq!(stretch.output_seek_length(1.0), 0);
    assert!(!stretch.split_computation());
}

#[test]
fn reset_reproduces_extreme_stretch() {
    let input = sine_input(2_048, 137.0);
    let mut first = vec![0.0_f32; 8_192];
    let mut second = vec![0.0_f32; 8_192];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(99);
    stretch.configure(1, 64, 16, false);
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut first], 8_192));
    stretch.reset();
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut second], 8_192));
    assert_eq!(first, second);
}

#[test]
fn configure_restores_extreme_stretch() {
    let input = sine_input(2_048, 137.0);
    let mut first = vec![0.0_f32; 8_192];
    let mut second = vec![0.0_f32; 8_192];
    let mut stretch = SignalsmithStretch::<f32>::with_seed(99);
    stretch.configure(1, 64, 16, false);
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut first], 8_192));
    stretch.configure(1, 64, 16, false);
    assert!(stretch.exact(&[&input], input.len(), &mut [&mut second], 8_192));
    assert_eq!(first, second);
}

#[test]
fn output_seek_length_saturates() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(1, 64, 16, false);
    assert_eq!(stretch.output_seek_length(f32::INFINITY), usize::MAX);
    assert_eq!(stretch.output_seek_length(f32::MAX), usize::MAX);
    assert_eq!(stretch.output_seek_length(-1.0), 0);
    assert_eq!(stretch.output_seek_length(f32::NAN), 0);
}

#[test]
fn seek_and_output_seek_stay_finite() {
    let input: Vec<f32> = (0..2_000).map(|i| (i as f32 * 0.071).sin() * 0.2).collect();
    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(1, 128, 32, false);

    let seek_length = stretch.seek_length();
    assert!(seek_length > 0);
    stretch.seek(&[&input], seek_length, 0.8);
    let mut seeked = vec![0.0_f32; 128];
    stretch.process(&[&input], 128, &mut [&mut seeked], 128);
    assert!(seeked.iter().all(|sample| sample.is_finite()));

    let output_length = stretch.output_latency();
    assert!(output_length > 0);
    stretch.output_seek(&[&input], 1_000);
    let mut output = vec![0.0_f32; 128];
    stretch.process(&[&input], 128, &mut [&mut output], 128);
    assert!(output.iter().all(|sample| sample.is_finite()));
}

#[test]
fn irregular_flush_stays_finite() {
    let input: Vec<f32> = (0..2_000).map(|i| (i as f32 * 0.071).sin() * 0.2).collect();
    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(1, 128, 32, false);
    let mut output = vec![0.0_f32; 560];
    stretch.process(&[&input[..300]], 300, &mut [&mut output[..240]], 240);
    stretch.process(&[&input[300..480]], 180, &mut [&mut output[240..400]], 160);
    stretch.process(&[&[]], 0, &mut [&mut output[400..560]], 160);
    let mut tail = vec![0.0_f32; 300];
    stretch.flush(&mut [&mut tail], 300, 0.8);
    assert!(output.iter().chain(&tail).all(|sample| sample.is_finite()));
}

#[test]
#[should_panic(expected = "block size exceeds")]
fn rejects_oversized_block() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(1, MAX_BLOCK_SAMPLES + 1, 1, false);
}

#[test]
#[should_panic(expected = "channel count exceeds")]
fn rejects_too_many_channels() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(MAX_CHANNELS + 1, 64, 16, false);
}

#[test]
#[should_panic(expected = "channel/band product exceeds")]
fn rejects_oversized_band_product() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(MAX_CHANNELS, MAX_BLOCK_SAMPLES, 1, false);
}

#[test]
#[should_panic(expected = "sample rate must be finite")]
fn rejects_non_finite_preset_rate() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.preset_default(1, f32::NAN, false);
}

#[test]
#[should_panic(expected = "transpose multiplier")]
fn rejects_zero_transpose_multiplier() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(1, 64, 16, false);
    stretch.set_transpose_factor(0.0, 0.5);
}

#[test]
#[should_panic(expected = "formant multiplier")]
fn rejects_zero_formant_multiplier() {
    let mut stretch = SignalsmithStretch::<f32>::with_seed(1);
    stretch.configure(1, 64, 16, false);
    stretch.set_formant_factor(0.0, false);
}
