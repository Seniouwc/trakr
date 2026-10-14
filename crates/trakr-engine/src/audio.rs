//! cpal output backend (feature `audio`).

use crate::{create, Engine, EngineHandle, InputRecorder};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Keep this alive for as long as audio should play.
pub struct AudioOutput {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub device_name: String,
}

/// Open the default output device and start the engine on it.
pub fn start() -> Result<(AudioOutput, EngineHandle)> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow!("no default audio output device"))?;
    let supported = device.default_output_config()?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(anyhow!(
            "default output format is {:?}, only f32 is supported",
            supported.sample_format()
        ));
    }
    let config: cpal::StreamConfig = supported.into();
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
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "unknown".into());
    Ok((AudioOutput { _stream: stream, sample_rate: rate, device_name: name }, handle))
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
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no default audio input device"))?;
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
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "unknown".into());
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
