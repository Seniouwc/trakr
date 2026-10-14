use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Reverse,
    PingPong,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub name: String,
    pub sample_rate: u32,
    pub channels: u8,
    /// Interleaved f32 PCM. Stored outside the song JSON by `trakr-format`.
    #[serde(skip)]
    pub data: Arc<Vec<f32>>,
    /// Internal note at which the sample plays at its original pitch (48 = C-4).
    pub base_note: u8,
    /// -128..=127 maps to -1..+1 semitone.
    pub fine_tune: i8,
    pub volume: f32,
    pub panning: f32,
    pub loop_mode: LoopMode,
    pub loop_start: usize,
    pub loop_end: usize,
    /// Inclusive key range this sample responds to.
    pub key_range: (u8, u8),
}

impl Sample {
    pub fn new(name: impl Into<String>, sample_rate: u32, channels: u8, data: Vec<f32>) -> Self {
        let frames = data.len() / channels.max(1) as usize;
        Self {
            name: name.into(),
            sample_rate,
            channels: channels.max(1),
            data: Arc::new(data),
            base_note: 48,
            fine_tune: 0,
            volume: 1.0,
            panning: 0.0,
            loop_mode: LoopMode::Off,
            loop_start: 0,
            loop_end: frames,
            key_range: (0, 119),
        }
    }

    pub fn frames(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Instrument {
    pub name: String,
    pub samples: Vec<Sample>,
    pub volume: f32,
}

impl Instrument {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), samples: Vec::new(), volume: 1.0 }
    }

    /// First sample whose key range contains `note`.
    pub fn sample_for(&self, note: u8) -> Option<&Sample> {
        self.samples.iter().find(|s| (s.key_range.0..=s.key_range.1).contains(&note))
    }
}
