# ESP32 QEMU benchmark

A measured estimate of **Signalsmith Stretch on the original ESP32 (Xtensa LX6)**, running in Espressif's QEMU ESP32 machine.

## At a glance

| | |
|---|---|
| Board | ESP32, 240 MHz, 320 KB DRAM |
| Emulator | Espressif QEMU 9.2.2, `-machine esp32` |
| Audio | 48 kHz, stereo, 1.5× time stretch |
| Configuration | `configure(2, 1024, 256, false)` |
| Workload | 50 blocks of 512 input frames → 768 output frames |
| Median throughput | **≈1.98× real-time** |
| CPU load | **≈50% of one core** |
| Heap after setup | **≈152 KiB allocated**, ≈167 KiB free |

The short version: in this emulator, one ESP32 core can process roughly **two seconds of 48 kHz stereo audio per second** at 1.5× stretch.

## Results

Three runs of the same firmware:

| Run | Guest cycles | Cycles / stereo output frame | Cycles / audio second | Real-time factor |
|---|---:|---:|---:|---:|
| 1 | 107,009,040 | 2,786 | 133,728,000 | **1.79×** |
| 2 | 92,464,080 | 2,407 | 115,536,000 | **2.08×** |
| 3 | **96,777,360** | **2,520** | **120,960,000** | **1.98×** |

The third run is the median. For that run:

```text
audio processed  = 38,400 frames / 48,000 Hz = 0.800 s
CPU time          = 96,777,360 cycles / 240 MHz = 0.403 s
real-time factor  = 0.800 / 0.403 = 1.98x
```

Per processing block:

```text
cycles per block  = 96,777,360 / 50 = 1,935,547
time per block    = 1,935,547 / 240 MHz = 8.06 ms
audio per block   = 768 / 48,000 = 16.0 ms
core load         = 8.06 / 16.0 = 50.4%
```

## Memory

The firmware reports heap usage from `ESP.getFreeHeap()`:

```text
free heap before `ss_create`: 326,644 bytes
free heap after  `ss_create`: 171,148 bytes
allocated:                    155,496 bytes
```

That is about **152 KiB** for the stereo instance, preallocated working buffers, and the benchmark's input/output arrays. No PSRAM was used.

## What is being measured

The test calls only the streaming `process()` path:

```rust
stretch.configure(2, 1024, 256, false);
// 3 warm-up blocks omitted from timing
let mut out = [&mut output_l[..], &mut output_r[..]];
stretch.process(
    &[&input_l[..], &input_r[..]],
    512,
    &mut out,
    768,
);
```

- Input: 512 stereo frames, 48 kHz
- Output: 768 stereo frames, 1.5× longer
- 50 measured iterations after 3 warm-up iterations
- No `flush`, file I/O, or serial output inside the timed region
- Timing uses the ESP32 `CCOUNT` register

The checksum was identical across all three runs: `-759534`.

## Setup

The experiment harness was kept outside the crate. The relative paths below assume it is placed at `tools/esp32-qemu-bench/` if you want to reproduce it inside the repo.

### 1. Rust firmware library

The no_std library is built as a static library and called from a small Arduino C++ harness:

```toml
# Cargo.toml
[lib]
crate-type = ["staticlib"]

[dependencies]
signalsmith-stretch = {
    package = "signalsmith-stretch-rs",
    path = "../../../",
    default-features = false,
    features = ["no_std"],
}
```

The wrapper provides:

- a global allocator backed by the ESP32 C `malloc`/`free`
- a panic handler and alloc-error handler
- `ss_create`, `ss_process`, and `ss_drop` C ABI entry points

The wrapper uses `unsafe` for the allocator and FFI boundary. The library itself still has `#![forbid(unsafe_code)]`.

Build with the esp-rs toolchain:

```sh
cargo +esp build --release --ignore-rust-version \
    --target xtensa-esp32-none-elf \
    -Zbuild-std=core,alloc
```

> The installed esp-rs toolchain was Rust 1.95-nightly while the crate declares MSRV 1.97. The firmware compiles cleanly with `--ignore-rust-version`; this should be revisited when a matching esp-rs toolchain is available.

### 2. Avoid libc symbol collisions

The Rust `compiler_builtins` archive contains weak `memset`, `memcpy`, and libm symbols. If linked as-is, the ESP32 flash driver resolved `memset` to the Rust implementation and crashed during boot with `Cache disabled but cached memory region accessed`.

Copy the archive, then rename those definitions before linking:

```sh
cp target/xtensa-esp32-none-elf/release/libss_esp32_bench.a \
   libss_esp32_bench_renamed.a

objcopy --redefine-sym memset=__rust_memset \
        --redefine-sym memcpy=__rust_memcpy \
        --redefine-sym memmove=__rust_memmove \
        --redefine-sym memcmp=__rust_memcmp \
        --redefine-sym fmax=__rust_fmax \
        --redefine-sym fmin=__rust_fmin \
        --redefine-sym sinf=__rust_sinf \
        --redefine-sym cosf=__rust_cosf \
        --redefine-sym sqrtf=__rust_sqrtf \
        --redefine-sym floorf=__rust_floorf \
        --redefine-sym ceilf=__rust_ceilf \
        --redefine-sym roundf=__rust_roundf \
        --redefine-sym truncf=__rust_truncf \
        --redefine-sym fabsf=__rust_fabsf \
        libss_esp32_bench_renamed.a
```

After renaming, `memset` resolves to the ESP-IDF/newlib implementation and the firmware boots normally.

### 3. Arduino harness

The harness runs on the Arduino-ESP32 core and supplies UART, bootloader, partition table, and timing:

```ini
; platformio.ini
[env:esp32dev]
platform = espressif32
board = esp32dev
framework = arduino
monitor_speed = 115200
build_flags =
  -L../../rust
  -lss_esp32_bench_renamed
```

The loop task needs a larger stack than the Arduino default:

```cpp
SET_LOOP_TASK_STACK_SIZE(32 * 1024);
```

Timing is read directly from `CCOUNT`:

```cpp
static inline uint32_t ccount() {
    uint32_t value;
    asm volatile("rsr %0, ccount" : "=r"(value));
    return value;
}

uint32_t start = ccount();
volatile float checksum = ss_process(bench, 50);
uint32_t end = ccount();
```

### 4. Build the flash image

Build the Arduino firmware:

```sh
pio run -e esp32dev
```

Merge the bootloader, partition table, OTA data, and application into a 4 MB image:

```sh
python3 esptool.py --chip esp32 merge_bin \
    --fill-flash-size 4MB \
    -o flash.bin \
    0x1000  .pio/build/esp32dev/bootloader.bin \
    0x8000  .pio/build/esp32dev/partitions.bin \
    0xe000  boot_app0.bin \
    0x10000 .pio/build/esp32dev/firmware.bin
```

### 5. Run QEMU

Download the Espressif QEMU build for the host, then:

```sh
qemu-system-xtensa -nographic \
    -machine esp32 \
    -drive file=flash.bin,if=mtd,format=raw
```

## Emulator and toolchain

- [Espressif QEMU releases](https://github.com/espressif/qemu/releases)
- [Espressif QEMU ESP32 documentation](https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-guides/tools/qemu.html)
- [esp-rs toolchain installation](https://docs.espressif.com/projects/rust/book/getting-started/toolchain.html)
- Arduino-ESP32 core 3.3.10
- PlatformIO `espressif32` 7.0.1
- esptool 4.11.0

## Caveats

- **QEMU is not cycle-accurate.** It does not model ESP32 flash-cache waits, memory contention, or instruction timing exactly. Real hardware will likely be slower.
- `CCOUNT` is a guest cycle counter, not a silicon measurement.
- The run-to-run spread was about ±10% around the median.
- This is one core; the original ESP32 is dual-core, but the library itself is single-threaded.
- The benchmark excludes setup, allocation, `flush`, and I/O.
- The benchmark uses a 1024-sample block and 256-sample interval, not the default 120 ms / 30 ms preset. Larger blocks cost more memory and CPU.

Treat **≈2× real-time** as a useful emulator estimate and a starting point for on-hardware profiling, not a guaranteed production figure.
