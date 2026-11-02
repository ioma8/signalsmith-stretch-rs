# Signalsmith Stretch — Rust

> A safe, fast Rust library for pitch shifting, time stretching, formant shaping, and streaming audio.

- **No `unsafe`** — `#![forbid(unsafe_code)]`
- **`f32` and `f64`** samples
- **Planar I/O** — independent input/output lengths
- **Streaming** — `process`, `flush`, `seek`, and exact fixed-length rendering
- **Numerically regression-tested** against committed reference fixtures
- **Rust 2024**, MSRV `1.97`

Package: `signalsmith-stretch-rs` · Library crate: `signalsmith_stretch`

## Quick start

```toml
[dependencies]
signalsmith-stretch-rs = "1.3"
```

```rust
use signalsmith_stretch::SignalsmithStretch;

fn main() {
    let sample_rate = 48_000.0_f32;
    let output_frames = 7_200;
    let samples: Vec<f32> = (0..4_800)
        .map(|i| {
            let phase = std::f32::consts::TAU * 220.0 * i as f32 / sample_rate;
            phase.sin() * 0.3
        })
        .collect();

    let input: [&[f32]; 1] = [&samples];
    let mut rendered = vec![0.0_f32; output_frames];
    let mut output: [&mut [f32]; 1] = [&mut rendered];

    let mut stretch = SignalsmithStretch::<f32>::with_seed(12345);
    stretch.preset_default(1, sample_rate, false);
    stretch.set_transpose_semitones(4.0, 8_000.0 / sample_rate);

    assert!(stretch.exact(&input, samples.len(), &mut output, output_frames));
}
```

`exact()` performs seek + process + flush, and returns `false` if the input is too short. A larger compiled example is in [`examples/basic.rs`](examples/basic.rs).

## Streaming

```rust
use signalsmith_stretch::SignalsmithStretch;

fn main() {
    let input = vec![0.1_f32; 600];
    let mut output = vec![0.0_f32; 1_500];

    let mut stretch = SignalsmithStretch::<f32>::with_seed(12345);
    stretch.configure(1, 576, 144, false);

    {
        let mut first = [&mut output[..900]];
        stretch.process(&[&input], 600, &mut first, 900);
    }
    {
        let mut tail = [&mut output[900..]];
        stretch.flush(&mut tail, 600, 1.0);
    }
}
```

Feed `input_latency()` samples ahead of the processing point and read output `output_latency()` samples behind it.

## Configuration

| Call | Purpose |
| --- | --- |
| `preset_default` | 120 ms block / 30 ms interval |
| `preset_cheaper` | 100 ms block / 40 ms interval, lower CPU |
| `configure` | Explicit block and interval sizes |
| `set_transpose_semitones` | Pitch shift, with optional tonality limit |
| `set_formant_semitones` | Formant shift, with optional pitch compensation |
| `set_formant_base` | Rough fundamental, or `0.0` for automatic estimation |
| `set_freq_map` | Custom input-to-output frequency map |
| split computation | `true` argument to a preset or `configure`; adds one interval of latency |

Prefer preset defaults unless you are optimizing CPU or latency. Blocks are capped at 1,048,576 samples and 256 channels; oversized configurations panic with a clear message.

## WAV CLI

```sh
cargo run --release --features cli --bin stretch -- \
    --time 1.5 --semitones 4 --tonality 8000 \
    input.wav output.wav
```

The CLI preserves integer WAV bit depths and 32-bit float WAV. It rejects 64-bit float WAV with an explicit error.

## Numerical checks

The committed reference fixtures cover mono, stereo, three-channel audio, `f32`, `f64`, split computation, formants, custom maps, and streaming.

- `f32` exact renders: single-precision round-off
- `f64` exact renders: within `1e-10` absolute error
- Zero-input calls: finite output is guaranteed, sample-for-sample equality is not

Above `2×` stretch, phase randomization uses a deterministic PCG32 stream. The same seed reproduces the same output.

## Build and verify

```sh
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --doc --all-features
cargo fmt --all -- --check
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
```

A throughput probe is included:

```sh
cargo run --release --example bench
```

## License

MIT. See [`LICENSE.txt`](LICENSE.txt).
