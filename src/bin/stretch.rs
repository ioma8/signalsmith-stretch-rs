//! Command-line WAV stretcher.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "WAV and fixture formats use explicit integer/float conversions"
)]

use std::env;
use std::error::Error;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use signalsmith_stretch::{MAX_BLOCK_SAMPLES, MAX_CHANNELS, SignalsmithStretch, VERSION};

type PlanarAudio = Vec<Vec<f32>>;

struct Options {
    input: PathBuf,
    output: PathBuf,
    time: f64,
    semitones: f64,
    tonality_hz: f64,
    formant: f64,
    formant_compensation: bool,
    formant_base_hz: f64,
    split_computation: bool,
    block_ms: f64,
    overlap: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output: PathBuf::new(),
            time: 1.0,
            semitones: 0.0,
            tonality_hz: 8000.0,
            formant: 0.0,
            formant_compensation: false,
            formant_base_hz: 100.0,
            split_computation: false,
            block_ms: 120.0,
            overlap: 4.0,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

const MAX_OUTPUT_FRAMES: usize = 100_000_000;
const MAX_OUTPUT_VALUES: usize = 100_000_000;
const MAX_INPUT_VALUES: usize = 100_000_000;

fn run() -> Result<(), Box<dyn Error>> {
    let Some(options) = parse_args()? else {
        return Ok(());
    };

    let mut reader = WavReader::open(&options.input)?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    if channels == 0 {
        return Err("WAV has no channels".into());
    }
    if channels > MAX_CHANNELS {
        return Err(format!(
            "WAV has {channels} channels; the supported maximum is {MAX_CHANNELS}"
        )
        .into());
    }
    let sample_rate = f64::from(spec.sample_rate);
    let (input, input_length) = read_planar(&mut reader, &spec)?;

    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err("WAV sample rate must be finite and positive".into());
    }
    let output_length_f = (input_length as f64 * options.time).round();
    if !output_length_f.is_finite()
        || output_length_f < 1.0
        || output_length_f > MAX_OUTPUT_FRAMES as f64
    {
        return Err(format!(
            "resulting output length must be between 1 and {MAX_OUTPUT_FRAMES} frames"
        )
        .into());
    }
    let output_length = output_length_f as usize;
    if output_length
        .checked_mul(channels)
        .is_none_or(|values| values > MAX_OUTPUT_VALUES)
    {
        return Err(format!("output would exceed {MAX_OUTPUT_VALUES} sample values").into());
    }

    // Match the upstream example: fractional block/interval sizes truncate.
    let max_block_samples = MAX_BLOCK_SAMPLES as f64;
    let block_length_f = sample_rate * 0.001 * options.block_ms;
    if !block_length_f.is_finite() || !(2.0..=max_block_samples).contains(&block_length_f) {
        return Err(format!(
            "WAV sample rate/--block-ms must produce 2..={MAX_BLOCK_SAMPLES} block samples"
        )
        .into());
    }
    let block_samples = block_length_f as usize;
    let interval_samples =
        ((block_samples as f64 / options.overlap) as usize).clamp(1, block_samples);

    let mut stretch = SignalsmithStretch::<f32>::with_seed(0x5eed_1234);
    stretch.configure(
        channels,
        block_samples,
        interval_samples,
        options.split_computation,
    );
    stretch.set_transpose_semitones(
        options.semitones as f32,
        (options.tonality_hz / sample_rate) as f32,
    );
    stretch.set_formant_semitones(options.formant as f32, options.formant_compensation);
    stretch.set_formant_base((options.formant_base_hz / sample_rate) as f32);

    let input_refs: Vec<&[f32]> = input.iter().map(Vec::as_slice).collect();
    let mut output = vec![vec![0.0_f32; output_length]; channels];
    {
        let mut output_refs: Vec<&mut [f32]> = output.iter_mut().map(Vec::as_mut_slice).collect();
        if !stretch.exact(&input_refs, input_length, &mut output_refs, output_length) {
            return Err(format!(
                "input too short for exact stretching: need at least {} frames, got {input_length}",
                stretch.output_seek_length((input_length as f32) / (output_length as f32))
            )
            .into());
        }
    }

    write_planar(&options.output, &spec, &output)?;
    Ok(())
}

fn parse_args() -> Result<Option<Options>, Box<dyn Error>> {
    let mut options = Options::default();
    let mut positional = Vec::new();
    let mut args = env::args().skip(1);

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(None);
            }
            "-v" | "--version" => {
                println!("{}.{}.{}", VERSION[0], VERSION[1], VERSION[2]);
                return Ok(None);
            }
            "--time" => options.time = next_value(&mut args, "--time")?.parse()?,
            "--semitones" => options.semitones = next_value(&mut args, "--semitones")?.parse()?,
            "--tonality" => options.tonality_hz = next_value(&mut args, "--tonality")?.parse()?,
            "--formant" => options.formant = next_value(&mut args, "--formant")?.parse()?,
            "--formant-base" => {
                options.formant_base_hz = next_value(&mut args, "--formant-base")?.parse()?;
            }
            "--block-ms" => options.block_ms = next_value(&mut args, "--block-ms")?.parse()?,
            "--overlap" => options.overlap = next_value(&mut args, "--overlap")?.parse()?,
            "--formant-comp" => options.formant_compensation = true,
            "--split-computation" => options.split_computation = true,
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}").into());
            }
            value => positional.push(value.to_owned()),
        }
    }

    if positional.len() != 2 {
        print_help();
        return Err("expected INPUT.wav and OUTPUT.wav".into());
    }
    options.input = PathBuf::from(&positional[0]);
    options.output = PathBuf::from(&positional[1]);
    for (name, value) in [
        ("--time", options.time),
        ("--semitones", options.semitones),
        ("--tonality", options.tonality_hz),
        ("--formant", options.formant),
        ("--formant-base", options.formant_base_hz),
        ("--block-ms", options.block_ms),
        ("--overlap", options.overlap),
    ] {
        if !value.is_finite() {
            return Err(format!("{name} must be finite").into());
        }
    }
    if options.time <= 0.0 {
        return Err("--time must be positive".into());
    }
    if options.semitones.abs() > 120.0 {
        return Err("--semitones must be within -120..=120".into());
    }
    if options.formant.abs() > 120.0 {
        return Err("--formant must be within -120..=120".into());
    }
    if options.tonality_hz < 0.0 || options.tonality_hz > 1.0e9 {
        return Err("--tonality must be within 0..=1e9 Hz".into());
    }
    if options.formant_base_hz < 0.0 || options.formant_base_hz > 1.0e9 {
        return Err("--formant-base must be within 0..=1e9 Hz".into());
    }
    if options.block_ms <= 0.0 {
        return Err("--block-ms must be positive".into());
    }
    if options.overlap <= 0.0 {
        return Err("--overlap must be positive".into());
    }
    Ok(Some(options))
}

fn next_value(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("missing value for {option}").into())
}

fn print_help() {
    println!(
        "signalsmith-stretch {}.{}.{}\n\
         \nUsage: stretch [OPTIONS] INPUT.wav OUTPUT.wav\n\
         \nOptions:\n\
         \t--time FACTOR             time-stretch factor (default 1)\n\
         \t--semitones N             pitch shift (default 0)\n\
         \t--tonality HZ             tonality limit (default 8000)\n\
         \t--formant N               formant shift in semitones (default 0)\n\
         \t--formant-comp            formant compensation\n\
         \t--formant-base HZ         formant base frequency, 0 = auto (default 100)\n\
         \t--split-computation       spread computation more evenly\n\
         \t--block-ms MS             STFT block length (default 120)\n\
         \t--overlap N               STFT overlap (default 4)\n\
         \t-v, --version             print version\n\
         \t-h, --help                print this help",
        VERSION[0], VERSION[1], VERSION[2]
    );
}

fn read_planar<R: Read + Seek>(
    reader: &mut WavReader<R>,
    spec: &WavSpec,
) -> Result<(PlanarAudio, usize), Box<dyn Error>> {
    let channels = usize::from(spec.channels);
    let total_values = reader.len() as usize;
    if total_values > MAX_INPUT_VALUES {
        return Err(format!(
            "input has {total_values} sample values; the supported maximum is {MAX_INPUT_VALUES}"
        )
        .into());
    }
    let mut output = vec![Vec::new(); channels];
    let frames = total_values / channels;
    for channel in &mut output {
        channel.reserve_exact(frames);
    }

    match spec.sample_format {
        SampleFormat::Float => {
            if spec.bits_per_sample != 32 {
                return Err(format!(
                    "only 32-bit floating-point WAV is supported (input has {} bits)",
                    spec.bits_per_sample
                )
                .into());
            }
            for (index, sample) in reader.samples::<f32>().enumerate() {
                output[index % channels].push(sample?);
            }
        }
        SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample.saturating_sub(1))) as f32;
            if spec.bits_per_sample <= 16 {
                for (index, sample) in reader.samples::<i16>().enumerate() {
                    output[index % channels].push(f32::from(sample?) / scale);
                }
            } else {
                for (index, sample) in reader.samples::<i32>().enumerate() {
                    output[index % channels].push(sample? as f32 / scale);
                }
            }
        }
    }

    let length = output.first().map_or(0, Vec::len);
    if output.iter().any(|channel| channel.len() != length) {
        return Err("WAV has unequal channel lengths".into());
    }
    Ok((output, length))
}

fn write_planar(
    path: &Path,
    input_spec: &WavSpec,
    channels: &[Vec<f32>],
) -> Result<(), Box<dyn Error>> {
    let frames = channels.first().map_or(0, Vec::len);
    match input_spec.sample_format {
        SampleFormat::Float => {
            let spec = WavSpec {
                channels: input_spec.channels,
                sample_rate: input_spec.sample_rate,
                bits_per_sample: 32,
                sample_format: SampleFormat::Float,
            };
            let mut writer = WavWriter::create(path, spec)?;
            for frame in 0..frames {
                for channel in channels {
                    writer.write_sample(channel[frame].clamp(-1.0, 1.0))?;
                }
            }
            writer.finalize()?;
        }
        SampleFormat::Int if input_spec.bits_per_sample <= 16 => {
            let spec = WavSpec {
                channels: input_spec.channels,
                sample_rate: input_spec.sample_rate,
                bits_per_sample: input_spec.bits_per_sample,
                sample_format: SampleFormat::Int,
            };
            let peak = ((1_i32 << (input_spec.bits_per_sample.saturating_sub(1))) - 1) as f32;
            let mut writer = WavWriter::create(path, spec)?;
            for frame in 0..frames {
                for channel in channels {
                    let sample = channel[frame].clamp(-1.0, 1.0);
                    writer.write_sample((sample * peak).round() as i16)?;
                }
            }
            writer.finalize()?;
        }
        SampleFormat::Int => {
            let spec = WavSpec {
                channels: input_spec.channels,
                sample_rate: input_spec.sample_rate,
                bits_per_sample: input_spec.bits_per_sample,
                sample_format: SampleFormat::Int,
            };
            let peak = ((1_i64 << (input_spec.bits_per_sample.saturating_sub(1))) - 1) as f32;
            let mut writer = WavWriter::create(path, spec)?;
            for frame in 0..frames {
                for channel in channels {
                    let sample = channel[frame].clamp(-1.0, 1.0);
                    writer.write_sample((sample * peak).round() as i32)?;
                }
            }
            writer.finalize()?;
        }
    }
    Ok(())
}
