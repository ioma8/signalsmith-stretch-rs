//! Numerical regression tests against committed reference fixtures.
//!
//! Each file stores the exact input and reference output as little-endian
//! samples. The tests allow single-precision round-off.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "WAV and fixture formats use explicit integer/float conversions"
)]

use std::fs;
use std::path::PathBuf;

use signalsmith_stretch::SignalsmithStretch;

#[derive(Clone, Copy)]
struct Scenario {
    channels: usize,
    block: usize,
    interval: usize,
    input_len: usize,
    output_len: usize,
    split: bool,
    semitones: f32,
    tonality: f32,
    formant: f32,
    base: f32,
    formant_comp: bool,
    custom_map: bool,
}

const SCENARIOS: [Scenario; 13] = [
    Scenario {
        channels: 1,
        block: 64,
        interval: 16,
        input_len: 1024,
        output_len: 1536,
        split: false,
        semitones: 4.0,
        tonality: 0.8,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 2,
        block: 128,
        interval: 32,
        input_len: 2048,
        output_len: 1536,
        split: false,
        semitones: -5.0,
        tonality: 0.7,
        formant: 3.0,
        base: 0.1,
        formant_comp: true,
        custom_map: false,
    },
    Scenario {
        channels: 2,
        block: 96,
        interval: 24,
        input_len: 1500,
        output_len: 2200,
        split: true,
        semitones: 0.0,
        tonality: 0.5,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 1,
        block: 80,
        interval: 20,
        input_len: 2000,
        output_len: 2000,
        split: false,
        semitones: 2.0,
        tonality: 0.9,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: true,
    },
    Scenario {
        channels: 1,
        block: 64,
        interval: 16,
        input_len: 1000,
        output_len: 1000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 2,
        block: 96,
        interval: 24,
        input_len: 1500,
        output_len: 2200,
        split: false,
        semitones: 0.0,
        tonality: 0.5,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 1,
        block: 64,
        interval: 16,
        input_len: 1000,
        output_len: 1000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: true,
    },
    Scenario {
        channels: 1,
        block: 64,
        interval: 16,
        input_len: 1024,
        output_len: 1536,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 1,
        block: 80,
        interval: 20,
        input_len: 1000,
        output_len: 1000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 1,
        block: 96,
        interval: 24,
        input_len: 1000,
        output_len: 1000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 1,
        block: 80,
        interval: 20,
        input_len: 2000,
        output_len: 2000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: true,
    },
    Scenario {
        channels: 1,
        block: 80,
        interval: 20,
        input_len: 2000,
        output_len: 2000,
        split: false,
        semitones: 0.0,
        tonality: 0.0,
        formant: 0.0,
        base: 0.0,
        formant_comp: false,
        custom_map: false,
    },
    Scenario {
        channels: 3,
        block: 96,
        interval: 24,
        input_len: 1200,
        output_len: 1800,
        split: false,
        semitones: 0.0,
        tonality: 0.5,
        formant: 3.0,
        base: 0.0,
        formant_comp: true,
        custom_map: false,
    },
];

fn read_i32(data: &[u8], offset: &mut usize) -> i32 {
    let value = i32::from_le_bytes(data[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    value
}

#[test]
fn matches_streaming_reference() {
    let data = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parity-stream.bin"),
    )
    .expect("streaming reference fixture");
    assert_eq!(&data[..4], b"SSP3");
    let mut offset = 4;
    let channels = read_i32(&data, &mut offset) as usize;
    let input_len = read_i32(&data, &mut offset) as usize;
    let output_len = read_i32(&data, &mut offset) as usize;
    assert_eq!((channels, input_len, output_len), (1, 2000, 3000));

    let mut input = vec![vec![0.0_f32; input_len]; channels];
    for channel in &mut input {
        for sample in channel {
            *sample = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
        }
    }
    let mut reference = vec![vec![0.0_f32; output_len]; channels];
    for channel in &mut reference {
        for sample in channel {
            *sample = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            offset += 4;
        }
    }
    assert_eq!(offset, data.len(), "trailing fixture bytes");

    let mut stretch = SignalsmithStretch::<f32>::with_seed(7);
    stretch.configure(channels, 128, 32, false);
    stretch.set_transpose_semitones(3.0, 0.8);
    let mut output = vec![vec![0.0_f32; output_len]; channels];
    for call in 0..20 {
        let input_start = call * 100;
        let output_start = call * 150;
        let input_refs: Vec<&[f32]> = input
            .iter()
            .map(|channel| &channel[input_start..input_start + 100])
            .collect();
        let mut output_refs: Vec<&mut [f32]> = output
            .iter_mut()
            .map(|channel| &mut channel[output_start..output_start + 150])
            .collect();
        stretch.process(&input_refs, 100, &mut output_refs, 150);
    }

    let mut max_abs = 0.0_f32;
    for c in 0..channels {
        for i in 0..output_len {
            max_abs = max_abs.max((output[c][i] - reference[c][i]).abs());
        }
    }
    assert!(max_abs < 2e-4, "streaming parity max_abs={max_abs:e}");
}

#[test]
fn matches_reference() {
    for (index, scenario) in SCENARIOS.iter().enumerate() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("parity-{index}.bin"));
        let data = fs::read(&path).expect("reference fixture");
        assert_eq!(&data[..4], b"SSP1");
        let mut offset = 4;
        let channels = read_i32(&data, &mut offset) as usize;
        let input_len = read_i32(&data, &mut offset) as usize;
        let output_len = read_i32(&data, &mut offset) as usize;
        assert_eq!(channels, scenario.channels);
        assert_eq!(input_len, scenario.input_len);
        assert_eq!(output_len, scenario.output_len);

        let mut input = vec![vec![0.0_f32; input_len]; channels];
        for channel in &mut input {
            for sample in channel {
                *sample = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                offset += 4;
            }
        }
        let mut reference = vec![vec![0.0_f32; output_len]; channels];
        for channel in &mut reference {
            for sample in channel {
                *sample = f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                offset += 4;
            }
        }

        assert_eq!(offset, data.len(), "trailing fixture bytes");

        let mut stretch = SignalsmithStretch::<f32>::with_seed(12345);
        stretch.configure(channels, scenario.block, scenario.interval, scenario.split);
        stretch.set_transpose_semitones(scenario.semitones, scenario.tonality);
        stretch.set_formant_semitones(scenario.formant, scenario.formant_comp);
        stretch.set_formant_base(scenario.base);
        if scenario.custom_map {
            stretch.set_freq_map(|frequency| frequency * 1.25);
        }

        let input_refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
        let mut output = vec![vec![0.0_f32; output_len]; channels];
        {
            let mut output_refs: Vec<&mut [f32]> =
                output.iter_mut().map(Vec::as_mut_slice).collect();
            assert!(
                stretch.exact(&input_refs, input_len, &mut output_refs, output_len),
                "exact() rejected scenario {index}"
            );
        }

        let mut max_abs = 0.0_f32;
        let mut error_energy = 0.0_f64;
        let mut reference_energy = 0.0_f64;
        for c in 0..channels {
            for i in 0..output_len {
                let difference = output[c][i] - reference[c][i];
                max_abs = max_abs.max(difference.abs());
                error_energy += f64::from(difference) * f64::from(difference);
                reference_energy += f64::from(reference[c][i]) * f64::from(reference[c][i]);
            }
        }
        let relative_rms = (error_energy / reference_energy.max(1e-30)).sqrt();
        assert!(
            max_abs < 1e-3 && relative_rms < 2e-4,
            "scenario {index}: max_abs={max_abs:e}, relative_rms={relative_rms:e}"
        );
    }
}
