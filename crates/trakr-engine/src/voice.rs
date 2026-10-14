use std::sync::Arc;
use trakr_core::{LoopMode, Sample};

/// One playing sample. Linear interpolation; forward, reverse and ping-pong loops.
/// TODO: sinc interpolation, envelopes, NNA, filters.
pub struct Voice {
    pub track: usize,
    pub column: usize,
    data: Arc<Vec<f32>>,
    channels: usize,
    frames: usize,
    pos: f64,
    /// +1 forward, -1 backward.
    dir: f64,
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

/// Per-trigger options from effect commands.
#[derive(Clone, Copy, Default)]
pub struct TriggerOpts {
    /// Start offset in 1/256 of the sample length.
    pub offset: u8,
    pub backwards: bool,
}

impl Voice {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track: usize,
        column: usize,
        s: &Sample,
        inst_volume: f32,
        note: u8,
        vol: f32,
        pan: f32,
        opts: TriggerOpts,
    ) -> Self {
        let semis = note as f64 - s.base_note as f64 + s.fine_tune as f64 / 128.0;
        let frames = s.frames();
        let loop_end = s.loop_end.min(frames);
        let has_loop = s.loop_mode != LoopMode::Off && loop_end > s.loop_start;
        let reverse = opts.backwards || (s.loop_mode == LoopMode::Reverse && has_loop);
        let mut pos = frames as f64 * opts.offset as f64 / 256.0;
        if reverse {
            pos = if has_loop && s.loop_mode == LoopMode::Reverse {
                loop_end as f64 - 1.0
            } else {
                frames as f64 - 1.0
            };
        }
        Self {
            track,
            column,
            data: s.data.clone(),
            channels: s.channels as usize,
            frames,
            pos: pos.max(0.0),
            dir: if reverse { -1.0 } else { 1.0 },
            step_base: s.sample_rate as f64 * 2f64.powf(semis / 12.0),
            gain: vol * s.volume * inst_volume,
            pan: (pan + s.panning).clamp(-1.0, 1.0),
            loop_mode: s.loop_mode,
            loop_start: s.loop_start,
            loop_end,
            finished: frames == 0,
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
        let looping = self.loop_mode != LoopMode::Off && self.loop_end > self.loop_start;
        let len = (self.loop_end - self.loop_start.min(self.loop_end)) as f64;
        for out in buf.chunks_exact_mut(2) {
            if self.finished {
                return;
            }
            if self.pos < 0.0 || self.pos as usize >= self.frames {
                self.finished = true;
                return;
            }
            let i = self.pos as usize;
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

            self.pos += step * self.dir;
            if looping && !self.releasing {
                let (ls, le) = (self.loop_start as f64, self.loop_end as f64);
                match self.loop_mode {
                    LoopMode::Forward if self.pos >= le => self.pos -= len,
                    LoopMode::Reverse if self.pos < ls => self.pos += len,
                    LoopMode::PingPong => {
                        if self.dir > 0.0 && self.pos >= le {
                            self.pos = (2.0 * le - self.pos).min(le - 1e-3);
                            self.dir = -1.0;
                        } else if self.dir < 0.0 && self.pos < ls {
                            self.pos = 2.0 * ls - self.pos;
                            self.dir = 1.0;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
