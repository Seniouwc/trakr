use std::sync::Arc;
use trakr_core::{LoopMode, Sample};

/// One playing sample. Linear interpolation; forward/off loops only for now.
/// TODO: reverse/ping-pong loops, sinc interpolation, envelopes, NNA, filters.
pub struct Voice {
    pub track: usize,
    pub column: usize,
    data: Arc<Vec<f32>>,
    channels: usize,
    frames: usize,
    pos: f64,
    step_base: f64,
    gain: f32,
    pan: f32,
    loop_mode: LoopMode,
    loop_start: usize,
    loop_end: usize,
    finished: bool,
    releasing: bool,
    fade: f32,
}

impl Voice {
    pub fn new(
        track: usize,
        column: usize,
        s: &Sample,
        inst_volume: f32,
        note: u8,
        vol: f32,
        pan: f32,
    ) -> Self {
        let semis = note as f64 - s.base_note as f64 + s.fine_tune as f64 / 128.0;
        Self {
            track,
            column,
            data: s.data.clone(),
            channels: s.channels as usize,
            frames: s.frames(),
            pos: 0.0,
            step_base: s.sample_rate as f64 * 2f64.powf(semis / 12.0),
            gain: vol * s.volume * inst_volume,
            pan: (pan + s.panning).clamp(-1.0, 1.0),
            loop_mode: s.loop_mode,
            loop_start: s.loop_start,
            loop_end: s.loop_end.min(s.frames()),
            finished: false,
            releasing: false,
            fade: 1.0,
        }
    }

    pub fn release(&mut self) {
        self.releasing = true;
    }

    pub fn alive(&self) -> bool {
        !self.finished
    }

    #[inline]
    fn frame(&self, i: usize, ch: usize) -> f32 {
        self.data.get(i * self.channels + ch.min(self.channels - 1)).copied().unwrap_or(0.0)
    }

    /// Add this voice into interleaved stereo `buf`.
    pub fn mix(&mut self, buf: &mut [f32], engine_rate: u32, track_vol: f32, track_pan: f32) {
        let step = self.step_base / engine_rate as f64;
        let pan = (self.pan + track_pan).clamp(-1.0, 1.0);
        let (gl, gr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
        let g = self.gain * track_vol;
        let fade_step = 1.0 / (engine_rate as f32 * 0.005); // 5 ms release
        for out in buf.chunks_exact_mut(2) {
            if self.finished {
                return;
            }
            let i = self.pos as usize;
            if i >= self.frames {
                self.finished = true;
                return;
            }
            let frac = (self.pos - i as f64) as f32;
            let j = (i + 1).min(self.frames - 1);
            let l = self.frame(i, 0) * (1.0 - frac) + self.frame(j, 0) * frac;
            let r =
                if self.channels > 1 { self.frame(i, 1) * (1.0 - frac) + self.frame(j, 1) * frac } else { l };
            if self.releasing {
                self.fade -= fade_step;
                if self.fade <= 0.0 {
                    self.finished = true;
                    return;
                }
            }
            let a = g * self.fade;
            out[0] += l * a * gl;
            out[1] += r * a * gr;

            self.pos += step;
            if self.loop_mode == LoopMode::Forward
                && !self.releasing
                && self.loop_end > self.loop_start
                && self.pos >= self.loop_end as f64
            {
                self.pos -= (self.loop_end - self.loop_start) as f64;
            }
        }
    }
}
