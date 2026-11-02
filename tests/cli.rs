//! End-to-end WAV CLI tests.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "WAV test fixtures use explicit integer/float conversions"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_stretch")
}

fn temporary(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("stretch-cli-{prefix}-{unique}.wav"))
}

#[test]
fn stretches_a_wav_file() {
    let input_path = temporary("in");
    let output_path = temporary("out");
    write_input(&input_path, 4096);

    let result = Command::new(binary())
        .args(["--time", "1.5", "--semitones", "2", "--block-ms", "40"])
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run stretch binary");
    assert_success(&result);

    let mut reader = WavReader::open(&output_path).expect("output WAV");
    assert_eq!(reader.spec().channels, 1);
    assert_eq!(reader.spec().sample_rate, 1000);
    assert_eq!(reader.spec().bits_per_sample, 16);
    assert_eq!(reader.len(), 6144);
    let peak = reader
        .samples::<i16>()
        .map(Result::unwrap)
        .map(i16::unsigned_abs)
        .max()
        .unwrap_or(0);
    assert!(peak > 100, "output is unexpectedly quiet: peak={peak}");

    cleanup(&[&input_path, &output_path]);
}

#[test]
fn rejects_non_finite_values() {
    let input_path = temporary("nan-in");
    let output_path = temporary("nan-out");
    write_input(&input_path, 512);
    for option in ["NaN", "inf", "-inf"] {
        let result = Command::new(binary())
            .args(["--time", option])
            .arg(&input_path)
            .arg(&output_path)
            .output()
            .expect("run stretch binary");
        assert!(!result.status.success(), "--time {option} should fail");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("finite"),
            "--time {option} stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    cleanup(&[&input_path, &output_path]);
}

#[test]
fn rejects_excessive_finite_values() {
    let input_path = temporary("huge-in");
    let output_path = temporary("huge-out");
    write_input(&input_path, 512);
    for args in [["--time", "1e300"], ["--block-ms", "1e300"]] {
        let result = Command::new(binary())
            .args(args)
            .arg(&input_path)
            .arg(&output_path)
            .output()
            .expect("run stretch binary");
        assert!(!result.status.success(), "{args:?} should fail");
    }
    cleanup(&[&input_path, &output_path]);
}

#[test]
fn rejects_invalid_block_sizes() {
    let input_path = temporary("block-in");
    let output_path = temporary("block-out");
    write_input(&input_path, 512);
    for value in ["0", "-1"] {
        let result = Command::new(binary())
            .args(["--block-ms", value])
            .arg(&input_path)
            .arg(&output_path)
            .output()
            .expect("run stretch binary");
        assert!(!result.status.success(), "--block-ms {value} should fail");
    }
    cleanup(&[&input_path, &output_path]);
}

#[test]
fn preserves_float_sample_format() {
    let input_path = temporary("float-in");
    let output_path = temporary("float-out");
    let spec = WavSpec {
        channels: 1,
        sample_rate: 1000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&input_path, spec).unwrap();
    for index in 0..512 {
        let phase = std::f64::consts::TAU * 180.0 * index as f64 / 1000.0;
        writer.write_sample((phase.sin() * 0.4) as f32).unwrap();
    }
    writer.finalize().unwrap();

    let result = Command::new(binary())
        .args(["--time", "1", "--block-ms", "20"])
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run stretch binary");
    assert_success(&result);

    let mut reader = WavReader::open(&output_path).expect("output WAV");
    assert_eq!(reader.spec().sample_format, SampleFormat::Float);
    assert_eq!(reader.spec().bits_per_sample, 32);
    assert_eq!(reader.len(), 512);
    let peak = reader
        .samples::<f32>()
        .map(Result::unwrap)
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.01,
        "float output is unexpectedly quiet: peak={peak}"
    );
    cleanup(&[&input_path, &output_path]);
}

fn assert_success(result: &Output) {
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn cleanup(paths: &[&Path]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn write_input(path: &Path, frames: usize) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 1000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    for index in 0..frames {
        let phase = std::f64::consts::TAU * 180.0 * index as f64 / 1000.0;
        writer
            .write_sample((phase.sin() * 12_000.0) as i16)
            .unwrap();
    }
    writer.finalize().unwrap();
}

#[test]
fn preserves_integer_bit_depths() {
    for bits in [8_u16, 16, 24, 32] {
        let input_path = temporary(&format!("int{bits}-in"));
        let output_path = temporary(&format!("int{bits}-out"));
        write_int_input(&input_path, 512, bits);
        let result = Command::new(binary())
            .args(["--time", "1", "--block-ms", "20"])
            .arg(&input_path)
            .arg(&output_path)
            .output()
            .expect("run stretch binary");
        assert_success(&result);
        let mut reader = WavReader::open(&output_path).expect("output WAV");
        assert_eq!(reader.spec().sample_format, SampleFormat::Int);
        assert_eq!(reader.spec().bits_per_sample, bits);
        assert_eq!(reader.len(), 512);
        let peak = reader
            .samples::<i32>()
            .map(Result::unwrap)
            .map(i32::unsigned_abs)
            .max()
            .unwrap_or(0);
        assert!(peak > 0, "{bits}-bit output is silent");
        cleanup(&[&input_path, &output_path]);
    }
}

#[test]
fn rejects_64_bit_float_wav() {
    let input_path = temporary("f64-in");
    let output_path = temporary("f64-out");
    write_f64_wav(&input_path, 256);
    let result = Command::new(binary())
        .args(["--time", "1"])
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run stretch binary");
    assert!(!result.status.success(), "64-bit float should be rejected");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("32-bit floating") || stderr.contains("bits per sample is not 32"),
        "unexpected stderr: {stderr}"
    );
    cleanup(&[&input_path, &output_path]);
}

fn write_int_input(path: &Path, frames: usize, bits: u16) {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 1000,
        bits_per_sample: bits,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec).unwrap();
    for index in 0..frames {
        let phase = std::f64::consts::TAU * 180.0 * index as f64 / 1000.0;
        let value = phase.sin();
        match bits {
            8 => writer.write_sample((value * 100.0) as i8).unwrap(),
            16 => writer.write_sample((value * 12_000.0) as i16).unwrap(),
            _ => writer.write_sample((value * 1_000_000.0) as i32).unwrap(),
        }
    }
    writer.finalize().unwrap();
}

fn write_f64_wav(path: &Path, frames: usize) {
    let data_len = (frames * 8) as u32;
    let mut bytes = Vec::with_capacity(44 + frames * 8);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1000_u32.to_le_bytes());
    bytes.extend_from_slice(&8000_u32.to_le_bytes());
    bytes.extend_from_slice(&8_u16.to_le_bytes());
    bytes.extend_from_slice(&64_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for index in 0..frames {
        let value = (index as f64 * 0.031).sin() * 0.2;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn rejects_too_many_channels() {
    let input_path = temporary("channels-in");
    let output_path = temporary("channels-out");
    let channels = 257_u16;
    let spec = WavSpec {
        channels,
        sample_rate: 1000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&input_path, spec).unwrap();
    for index in 0..256 {
        let sample = (index as i16 % 64) - 32;
        for _ in 0..channels {
            writer.write_sample(sample).unwrap();
        }
    }
    writer.finalize().unwrap();

    let result = Command::new(binary())
        .args(["--time", "1"])
        .arg(&input_path)
        .arg(&output_path)
        .output()
        .expect("run stretch binary");
    assert!(!result.status.success(), "too many channels should fail");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("supported maximum"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    cleanup(&[&input_path, &output_path]);
}

#[test]
fn rejects_invalid_numeric_ranges() {
    let input_path = temporary("ranges-in");
    let output_path = temporary("ranges-out");
    write_input(&input_path, 256);
    for args in [
        ["--semitones", "121"],
        ["--formant", "-121"],
        ["--tonality", "-1"],
        ["--formant-base", "-1"],
        ["--overlap", "0"],
    ] {
        let result = Command::new(binary())
            .args(args)
            .arg(&input_path)
            .arg(&output_path)
            .output()
            .expect("run stretch binary");
        assert!(!result.status.success(), "{args:?} should fail");
    }
    cleanup(&[&input_path, &output_path]);
}
