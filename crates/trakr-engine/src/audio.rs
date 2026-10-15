//! cpal output backend (feature `audio`).

use crate::{create, Engine, EngineHandle, InputRecorder};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Keep this alive for as long as audio should play.
pub struct AudioOutput {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub device_name: String,
    pub status: AudioStatus,
}

/// Which host, devices, rate and buffer to use. Empty names and zeros mean "the default".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioConfig {
    /// Audio system, e.g. "ALSA" or "JACK" (empty = platform default).
    pub host: String,
    pub output: String,
    pub input: String,
    /// Engine sample rate in Hz (0 = the device's default).
    pub sample_rate: u32,
    /// Requested buffer size in frames (0 = the driver's default).
    pub buffer_frames: u32,
}

/// What the output ended up using.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioStatus {
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    /// Fixed buffer size in frames, or 0 when the driver chose.
    pub buffer_frames: u32,
    pub channels: u16,
}

impl AudioStatus {
    /// Output latency of the buffer alone, in milliseconds (0 when the driver chose the size).
    pub fn buffer_ms(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.buffer_frames as f32 * 1000.0 / self.sample_rate as f32
        }
    }
}

const COMMON_RATES: [u32; 9] = [22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 384_000];

fn host_by_name(name: &str) -> Result<cpal::Host> {
    if name.is_empty() {
        return Ok(cpal::default_host());
    }
    let id = cpal::available_hosts()
        .into_iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow!("audio system {name} is not available"))?;
    Ok(cpal::host_from_id(id)?)
}

fn device_name(d: &cpal::Device) -> String {
    d.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "unknown".into())
}

/// Audio systems this build can use (ALSA; JACK when built with the `jack` feature).
pub fn hosts() -> Vec<String> {
    cpal::available_hosts().into_iter().map(|h| h.name().to_string()).collect()
}

pub fn default_host_name() -> String {
    cpal::default_host().id().name().to_string()
}

pub fn output_devices(host: &str) -> Vec<String> {
    host_by_name(host)
        .ok()
        .and_then(|h| h.output_devices().ok())
        .map(|d| d.map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

pub fn input_devices(host: &str) -> Vec<String> {
    host_by_name(host)
        .ok()
        .and_then(|h| h.input_devices().ok())
        .map(|d| d.map(|d| device_name(&d)).collect())
        .unwrap_or_default()
}

fn find_output(host: &cpal::Host, name: &str) -> Result<cpal::Device> {
    if name.is_empty() {
        return host.default_output_device().ok_or_else(|| anyhow!("no default audio output device"));
    }
    host.output_devices()?
        .find(|d| device_name(d) == name)
        .ok_or_else(|| anyhow!("output device {name} not found"))
}

fn find_input(host: &cpal::Host, name: &str) -> Result<cpal::Device> {
    if name.is_empty() {
        return host.default_input_device().ok_or_else(|| anyhow!("no default audio input device"));
    }
    host.input_devices()?
        .find(|d| device_name(d) == name)
        .ok_or_else(|| anyhow!("input device {name} not found"))
}

/// Sample rates (from the usual list) the output device accepts, plus its default.
pub fn output_rates(host: &str, device: &str) -> Vec<u32> {
    let Ok(h) = host_by_name(host) else { return Vec::new() };
    let Ok(d) = find_output(&h, device) else { return Vec::new() };
    let mut out: Vec<u32> = Vec::new();
    if let Ok(ranges) = d.supported_output_configs() {
        for r in ranges {
            out.extend(
                COMMON_RATES
                    .iter()
                    .copied()
                    .filter(|&c| c >= r.min_sample_rate() && c <= r.max_sample_rate()),
            );
        }
    }
    if let Ok(def) = d.default_output_config() {
        out.push(def.sample_rate());
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Open the default output device and start the engine on it.
pub fn start() -> Result<(AudioOutput, EngineHandle)> {
    start_with(&AudioConfig::default())
}

/// Open the configured output and start a new engine on it. A named device that has disappeared is an
/// error (the caller decides whether to fall back to the default).
pub fn start_with(cfg: &AudioConfig) -> Result<(AudioOutput, EngineHandle)> {
    let host = host_by_name(&cfg.host)?;
    let device = find_output(&host, &cfg.output)?;
    let default = device.default_output_config()?;
    if default.sample_format() != cpal::SampleFormat::F32 {
        // Prefer an f32 configuration when the default one is not.
        let alt = device
            .supported_output_configs()?
            .find(|r| r.sample_format() == cpal::SampleFormat::F32)
            .ok_or_else(|| anyhow!("{} offers no f32 output format", device_name(&device)))?;
        let rate = default.sample_rate().clamp(alt.min_sample_rate(), alt.max_sample_rate());
        return open_output(&device, &host, alt.with_sample_rate(rate).into(), cfg);
    }
    let mut config: cpal::StreamConfig = default.into();
    if cfg.sample_rate != 0 {
        let supported = device.supported_output_configs()?.any(|r| {
            r.sample_format() == cpal::SampleFormat::F32
                && cfg.sample_rate >= r.min_sample_rate()
                && cfg.sample_rate <= r.max_sample_rate()
        });
        if !supported {
            return Err(anyhow!("{} does not support {} Hz", device_name(&device), cfg.sample_rate));
        }
        config.sample_rate = cfg.sample_rate;
    }
    open_output(&device, &host, config, cfg)
}

fn open_output(
    device: &cpal::Device,
    host: &cpal::Host,
    mut config: cpal::StreamConfig,
    cfg: &AudioConfig,
) -> Result<(AudioOutput, EngineHandle)> {
    if cfg.buffer_frames != 0 {
        config.buffer_size = cpal::BufferSize::Fixed(cfg.buffer_frames);
    }
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    let (mut engine, handle) = create(rate);
    let mut scratch: Vec<f32> = vec![0.0; 8192 * 2];
    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _| fill(&mut engine, &mut scratch, data, channels),
        |err| eprintln!("audio stream error: {err}"),
        None,
    )?;
    stream.play()?;
    let status = AudioStatus {
        host: host.id().name().to_string(),
        device: device_name(device),
        sample_rate: rate,
        buffer_frames: if cfg.buffer_frames != 0 { cfg.buffer_frames } else { 0 },
        channels: config.channels,
    };
    Ok((
        AudioOutput { _stream: stream, sample_rate: rate, device_name: status.device.clone(), status },
        handle,
    ))
}

fn fill(engine: &mut Engine, scratch: &mut Vec<f32>, data: &mut [f32], channels: usize) {
    let frames = data.len() / channels;
    if scratch.len() < frames * 2 {
        scratch.resize(frames * 2, 0.0); // rare; only when the device buffer grows
    }
    engine.render(&mut scratch[..frames * 2]);
    for (i, out) in data.chunks_exact_mut(channels).enumerate() {
        let (l, r) = (scratch[i * 2], scratch[i * 2 + 1]);
        match channels {
            1 => out[0] = (l + r) * 0.5,
            _ => {
                out[0] = l;
                out[1] = r;
                out[2..].fill(0.0);
            }
        }
    }
}

/// Live capture from the default input device. Keep it alive while recording.
pub struct CpalRecorder {
    _stream: cpal::Stream,
    rx: rtrb::Consumer<f32>,
    rate: u32,
    channels: u8,
    pub device_name: String,
}

impl InputRecorder for CpalRecorder {
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn channels(&self) -> u8 {
        self.channels
    }
    fn drain(&mut self, out: &mut Vec<f32>) {
        while let Ok(v) = self.rx.pop() {
            out.push(v);
        }
    }
}

/// Open the default input device and start capturing (mono or stereo; extra channels are dropped).
pub fn start_input() -> Result<CpalRecorder> {
    start_input_with(&AudioConfig::default())
}

/// Like `start_input`, on the configured host and input device.
pub fn start_input_with(cfg: &AudioConfig) -> Result<CpalRecorder> {
    let host = host_by_name(&cfg.host)?;
    let device = find_input(&host, &cfg.input)?;
    let supported = device.default_input_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let src_ch = (config.channels as usize).max(1);
    let channels = src_ch.min(2);
    // Ten seconds of headroom; the UI drains every frame, so this only fills if the UI stalls.
    let (tx, rx) = rtrb::RingBuffer::new(rate as usize * channels * 10);

    let stream = match format {
        cpal::SampleFormat::F32 => input_stream::<f32>(&device, &config, tx, src_ch, channels, |s| s)?,
        cpal::SampleFormat::I16 => {
            input_stream::<i16>(&device, &config, tx, src_ch, channels, |s| s as f32 / 32768.0)?
        }
        cpal::SampleFormat::U16 => {
            input_stream::<u16>(&device, &config, tx, src_ch, channels, |s| (s as f32 - 32768.0) / 32768.0)?
        }
        cpal::SampleFormat::I32 => {
            input_stream::<i32>(&device, &config, tx, src_ch, channels, |s| s as f32 / 2_147_483_648.0)?
        }
        other => return Err(anyhow!("unsupported input sample format {other:?}")),
    };
    stream.play()?;
    let name = device_name(&device);
    Ok(CpalRecorder { _stream: stream, rx, rate, channels: channels as u8, device_name: name })
}

fn input_stream<T: cpal::SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut tx: rtrb::Producer<f32>,
    src_ch: usize,
    channels: usize,
    to_f32: fn(T) -> f32,
) -> Result<cpal::Stream> {
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            for frame in data.chunks_exact(src_ch) {
                for &s in &frame[..channels] {
                    let _ = tx.push(to_f32(s)); // dropping on overflow keeps the callback wait-free
                }
            }
        },
        |err| eprintln!("audio input error: {err}"),
        None,
    )?)
}
