#![forbid(unsafe_code)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, SupportedStreamConfig};
use eframe::egui;
use signalsmith_stretch::SignalsmithStretch;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([560.0, 320.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Signalsmith Stretch — Mic Demo",
        options,
        Box::new(|_cc| Ok(Box::new(StretchApp::default()))),
    )
}

struct StretchApp {
    recorder: Option<Recorder>,
    recorded: Vec<f32>,
    sample_rate: u32,
    stretch: f32,
    pitch: f32,
    player: Option<Player>,
    status: String,
}

impl Default for StretchApp {
    fn default() -> Self {
        Self {
            recorder: None,
            recorded: Vec::new(),
            sample_rate: 48_000,
            stretch: 1.0,
            pitch: 0.0,
            player: None,
            status: "Ready to record.".into(),
        }
    }
}

impl StretchApp {
    fn start_recording(&mut self) {
        self.player = None;
        match Recorder::start() {
            Ok(recorder) => {
                self.sample_rate = recorder.sample_rate;
                self.recorded.clear();
                self.recorder = Some(recorder);
                self.status = format!("Recording at {} Hz.", self.sample_rate);
            }
            Err(error) => self.status = format!("Could not start microphone: {error:#}"),
        }
    }

    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let mut samples = recorder.stop();
        let seconds = samples.len() as f32 / self.sample_rate as f32;
        if samples.is_empty() {
            self.status = "No audio was captured.".into();
        } else {
            let peak = samples
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0_f32, f32::max);
            if peak > 1e-6 {
                let gain = 0.95 / peak;
                for sample in &mut samples {
                    *sample *= gain;
                }
                self.status = format!(
                    "Recorded {seconds:.2} s at {} Hz; normalized.",
                    self.sample_rate
                );
            } else {
                self.status = format!(
                    "Recorded {seconds:.2} s at {} Hz; the take is silent.",
                    self.sample_rate
                );
            }
            self.recorded = samples;
        }
    }

    fn play(&mut self) {
        if self.recorded.is_empty() {
            self.status = "Record something first.".into();
            return;
        }

        match self.render_processed() {
            Ok(processed) => {
                let duration = processed.len() as f32 / self.sample_rate as f32;
                match Player::start(processed, self.sample_rate) {
                    Ok(player) => {
                        self.status = format!("Playing {duration:.2} s.");
                        self.player = Some(player);
                    }
                    Err(error) => self.status = format!("Could not play audio: {error:#}"),
                }
            }
            Err(error) => self.status = format!("Could not process audio: {error:#}"),
        }
    }

    fn render_processed(&self) -> Result<Vec<f32>> {
        let output_len = (self.recorded.len() as f32 * self.stretch).round().max(1.0) as usize;
        let mut output = vec![0.0_f32; output_len];

        let mut stretch = SignalsmithStretch::<f32>::with_seed(12345);
        stretch.preset_default(1, self.sample_rate as f32, false);
        stretch.set_transpose_semitones(self.pitch, 8_000.0 / self.sample_rate as f32);

        let input: [&[f32]; 1] = [&self.recorded];
        let mut output_ref: [&mut [f32]; 1] = [&mut output];
        if !stretch.exact(&input, self.recorded.len(), &mut output_ref, output_len) {
            bail!("recording is too short for the configured stretcher");
        }
        Ok(output)
    }
}

impl eframe::App for StretchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let finished = self.player.as_ref().is_some_and(Player::finished);
        if finished {
            self.player = None;
            self.status = "Playback finished.".into();
        }

        let panel_fill = ui.visuals().panel_fill;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::symmetric(24, 20)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = 300.0;
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Signalsmith Stretch")
                                .size(28.0)
                                .strong(),
                        );
                        ui.label(egui::RichText::new("Record. Shape. Play.").size(14.0));
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let recording = self.recorder.is_some();
                        let record_label = if recording {
                            "Stop recording"
                        } else {
                            "Start recording"
                        };
                        if ui
                            .add_sized(
                                [154.0, 36.0],
                                egui::Button::new(egui::RichText::new(record_label).strong())
                                    .corner_radius(8.0),
                            )
                            .clicked()
                        {
                            if recording {
                                self.stop_recording();
                            } else {
                                self.start_recording();
                            }
                        }
                    });
                });

                ui.add_space(22.0);
                egui::Grid::new("shape-controls")
                    .num_columns(2)
                    .spacing([18.0, 14.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Stretch").strong());
                        ui.add(
                            egui::Slider::new(&mut self.stretch, 0.1..=10.0)
                                .logarithmic(true)
                                .custom_formatter(|value, _| format!("{value:.2}x")),
                        );
                        ui.end_row();

                        ui.label(egui::RichText::new("Pitch").strong());
                        ui.add(
                            egui::Slider::new(&mut self.pitch, -12.0..=12.0)
                                .custom_formatter(|value, _| format!("{value:+.1} st")),
                        );
                        ui.end_row();
                    });

                ui.add_space(22.0);
                ui.add_enabled_ui(!self.recorded.is_empty(), |ui| {
                    if ui
                        .add_sized(
                            [ui.available_width(), 38.0],
                            egui::Button::new(egui::RichText::new("Apply & play").strong())
                                .corner_radius(8.0),
                        )
                        .on_hover_text(
                            "Applies the current stretch and pitch, then plays the result.",
                        )
                        .clicked()
                    {
                        self.play();
                    }
                });

                let footer_height = if self.player.is_some() { 46.0 } else { 22.0 };
                ui.add_space((ui.available_height() - footer_height).max(0.0));
                if let Some(player) = &self.player {
                    ui.add(
                        egui::ProgressBar::new(player.progress())
                            .show_percentage()
                            .desired_width(ui.available_width()),
                    );
                }
                ui.label(egui::RichText::new(&self.status).weak());
            });

        ui.ctx().request_repaint();
    }
}

struct Recorder {
    _stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
}

impl Recorder {
    fn start() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no default microphone device")?;
        let config = choose_config(&device, true)?;
        let sample_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&samples);
        let max_samples = sample_rate as usize * 60;

        if config.sample_format() != SampleFormat::F32 {
            bail!(
                "microphone must support f32 samples (got {:?})",
                config.sample_format()
            );
        }

        let stream = device
            .build_input_stream(
                config.config(),
                move |data: &[f32], _| {
                    if channels == 0 {
                        return;
                    }
                    let mut buffer = capture.lock().expect("recording buffer poisoned");
                    let remaining = max_samples.saturating_sub(buffer.len());
                    buffer.extend(
                        data.chunks(channels)
                            .take(remaining)
                            .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32),
                    );
                },
                |error| eprintln!("microphone stream error: {error}"),
                None,
            )
            .context("build microphone stream")?;
        stream.play().context("start microphone stream")?;

        Ok(Self {
            _stream: stream,
            samples,
            sample_rate,
        })
    }

    fn stop(self) -> Vec<f32> {
        drop(self._stream);
        self.samples
            .lock()
            .expect("recording buffer poisoned")
            .clone()
    }
}

struct Player {
    _stream: Stream,
    cursor: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    frames: usize,
}

impl Player {
    fn start(samples: Vec<f32>, source_rate: u32) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no default output device")?;
        let config = choose_config(&device, false)?;
        if config.sample_format() != SampleFormat::F32 {
            bail!(
                "output must support f32 samples (got {:?})",
                config.sample_format()
            );
        }

        let output_rate = config.sample_rate();
        let channels = config.channels() as usize;
        let samples = resample_linear(&samples, source_rate, output_rate);
        let frames = samples.len();
        let buffer = Arc::new(samples);
        let cursor = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let cursor_cb = Arc::clone(&cursor);
        let done_cb = Arc::clone(&done);
        let stream = device
            .build_output_stream(
                config.config(),
                move |data: &mut [f32], _| {
                    if channels == 0 {
                        return;
                    }
                    let mut pos = cursor_cb.load(Ordering::Relaxed);
                    for frame in data.chunks_mut(channels) {
                        let value = buffer.get(pos).copied().unwrap_or(0.0);
                        pos += 1;
                        frame.fill(value);
                    }
                    cursor_cb.store(pos, Ordering::Relaxed);
                    if pos >= buffer.len() {
                        done_cb.store(true, Ordering::Relaxed);
                    }
                },
                |error| eprintln!("output stream error: {error}"),
                None,
            )
            .context("build output stream")?;
        stream.play().context("start playback stream")?;

        Ok(Self {
            _stream: stream,
            cursor,
            done,
            frames,
        })
    }

    fn finished(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }

    fn progress(&self) -> f32 {
        if self.frames == 0 {
            0.0
        } else {
            (self.cursor.load(Ordering::Relaxed) as f32 / self.frames as f32).min(1.0)
        }
    }
}

fn choose_config(device: &cpal::Device, input: bool) -> Result<SupportedStreamConfig> {
    let supported: Vec<_> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };

    let mut fallback = None;
    for range in supported {
        if range.sample_format() != SampleFormat::F32 {
            continue;
        }
        fallback.get_or_insert_with(|| range.with_max_sample_rate());
        if range.min_sample_rate() <= 48_000 && 48_000 <= range.max_sample_rate() {
            return Ok(range.with_sample_rate(48_000));
        }
    }
    fallback.context("no f32 audio configuration found")
}

fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let output_len = ((input.len() as f64) * ratio).round().max(1.0) as usize;
    let mut output = Vec::with_capacity(output_len);
    for i in 0..output_len {
        let source = i as f64 / ratio;
        let left = source.floor() as usize;
        let right = (left + 1).min(input.len() - 1);
        let fraction = (source - left as f64) as f32;
        output.push(input[left] + (input[right] - input[left]) * fraction);
    }
    output
}
