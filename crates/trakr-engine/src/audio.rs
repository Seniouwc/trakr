//! cpal output backend (feature `audio`).

use crate::{create, Engine, EngineHandle};
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
