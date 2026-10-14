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

    fn set_data(&mut self, data: Vec<f32>) {
        let frames = data.len() / self.channels.max(1) as usize;
        self.data = Arc::new(data);
        self.loop_start = self.loop_start.min(frames);
        self.loop_end = self.loop_end.min(frames).max(self.loop_start);
    }

    /// Scale so the loudest sample hits full scale. No-op for silence.
    pub fn normalize(&mut self) {
        let peak = self.data.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > 0.0 {
            self.set_data(self.data.iter().map(|s| s / peak).collect());
        }
    }

    pub fn reverse(&mut self) {
        let ch = self.channels as usize;
        let mut out = Vec::with_capacity(self.data.len());
        for f in self.data.chunks_exact(ch).rev() {
            out.extend_from_slice(f);
        }
        self.set_data(out);
    }

    /// Linear fade over the first `frames` frames.
    pub fn fade_in(&mut self, frames: usize) {
        let ch = self.channels as usize;
        let mut out = (*self.data).clone();
        for (i, f) in out.chunks_exact_mut(ch).take(frames).enumerate() {
            f.iter_mut().for_each(|s| *s *= i as f32 / frames as f32);
        }
        self.set_data(out);
    }

    /// Linear fade over the last `frames` frames.
    pub fn fade_out(&mut self, frames: usize) {
        let ch = self.channels as usize;
        let mut out = (*self.data).clone();
        let total = out.len() / ch;
        for (i, f) in out.chunks_exact_mut(ch).enumerate().skip(total.saturating_sub(frames)) {
            let left = (total - i) as f32 / frames as f32;
            f.iter_mut().for_each(|s| *s *= left);
        }
        self.set_data(out);
    }

    /// Keep only frames `start..end`.
    pub fn trim(&mut self, start: usize, end: usize) {
        let ch = self.channels as usize;
        let end = end.min(self.frames());
        let start = start.min(end);
        self.set_data(self.data[start * ch..end * ch].to_vec());
    }

    pub fn to_mono(&mut self) {
        if self.channels == 2 {
            let out = self.data.chunks_exact(2).map(|f| (f[0] + f[1]) * 0.5).collect();
            self.channels = 1;
            self.set_data(out);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn s(data: Vec<f32>) -> Sample {
        Sample::new("t", 44_100, 1, data)
    }

    #[test]
    fn normalize_reverse_trim() {
        let mut a = s(vec![0.1, -0.5, 0.25]);
        a.normalize();
        assert_eq!(*a.data, vec![0.2, -1.0, 0.5]);
        a.reverse();
        assert_eq!(*a.data, vec![0.5, -1.0, 0.2]);
        a.trim(1, 3);
        assert_eq!(*a.data, vec![-1.0, 0.2]);
        assert_eq!(a.loop_end, 2);
    }

    #[test]
    fn fades_and_mono() {
        let mut a = s(vec![1.0; 4]);
        a.fade_in(4);
        assert_eq!(*a.data, vec![0.0, 0.25, 0.5, 0.75]);
        let mut b = s(vec![1.0; 4]);
        b.fade_out(2);
        assert_eq!(*b.data, vec![1.0, 1.0, 1.0, 0.5]);
        let mut c = Sample::new("t", 44_100, 2, vec![1.0, 0.0, 0.5, 0.5]);
        c.to_mono();
        assert_eq!((c.channels, &*c.data), (1, &vec![0.5, 0.5]));
    }
}
