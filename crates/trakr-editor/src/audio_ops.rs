//! Audio system selection: host, devices, sample rate and buffer size, applied at run time.

use crate::{Editor, EngineHandle};
use serde::{Deserialize, Serialize};

/// Saved audio choices. Empty names and zeros mean "the default".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub host: String,
    pub output: String,
    pub input: String,
    pub sample_rate: u32,
    pub buffer_frames: u32,
}

/// What the running output is using.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioInfo {
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub buffer_frames: u32,
    pub channels: u16,
}

impl AudioInfo {
    pub fn buffer_ms(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.buffer_frames as f32 * 1000.0 / self.sample_rate as f32
        }
    }
}

/// Implemented by the binary on top of the audio backend; the editor never touches devices itself.
pub trait AudioControl {
    fn hosts(&self) -> Vec<String>;
    fn outputs(&self, host: &str) -> Vec<String>;
    fn inputs(&self, host: &str) -> Vec<String>;
    /// Sample rates the output device accepts.
    fn rates(&self, host: &str, output: &str) -> Vec<u32>;
    /// Stop the current output and start a new one; the old engine is dropped on success only.
    fn restart(&mut self, settings: &AudioSettings) -> Result<(EngineHandle, AudioInfo), String>;
}

impl Editor {
    pub fn set_audio_control(&mut self, control: Box<dyn AudioControl>, info: Option<AudioInfo>) {
        self.audio_ctl = Some(control);
        self.audio_info = info;
    }

    pub fn audio_control(&self) -> Option<&dyn AudioControl> {
        self.audio_ctl.as_deref()
    }

    pub fn audio_info(&self) -> Option<&AudioInfo> {
        self.audio_info.as_ref()
    }

    /// Switch to the output described by `prefs.audio`. Playback stops; the song, device chains and MIDI
    /// output carry over to the new engine. On failure the previous output keeps running.
    pub fn audio_apply(&mut self) {
        let settings = self.prefs.audio.clone();
        if self.audio_ctl.is_none() {
            self.status = "audio selection is not available in this build".into();
            return;
        }
        if self.recorder.is_some() {
            self.record_stop();
        }
        let midi_port = self.midi_out_port_name().map(str::to_string);
        if midi_port.is_some() {
            self.midi_out_disconnect();
        }
        // `midi_out_disconnect` needs the old engine, so only now hand over to the audio control.
        let Some(ctl) = &mut self.audio_ctl else { return };
        match ctl.restart(&settings) {
            Ok((handle, info)) => {
                self.status = format!(
                    "audio: {} / {} at {} Hz{}",
                    info.host,
                    info.device,
                    info.sample_rate,
                    if info.buffer_frames > 0 {
                        format!(", {} frames ({:.1} ms)", info.buffer_frames, info.buffer_ms())
                    } else {
                        String::new()
                    }
                );
                self.engine = Some(handle);
                self.audio_info = Some(info);
                self.sent_chains.clear();
                self.sent_inst_plugins.clear();
                self.sent_inst_fx.clear();
                self.needs_sync = true;
                self.flush_sync();
                self.save_prefs();
            }
            Err(e) => self.status = format!("audio not changed: {e}"),
        }
        if let Some(name) = midi_port {
            if let Some(i) = self.midi_out_ports().iter().position(|p| *p == name) {
                self.midi_out_connect(i);
            }
        }
    }
}
