//! Device chains and pattern automation.

use super::*;
use trakr_core::{AutoTarget, Automation, DeviceSpec};

impl Editor {
    /// Append a native device to a track. Returns false for an unknown id.
    pub fn device_add(&mut self, track: usize, id: &str) -> bool {
        let Some(spec) = DeviceSpec::new(id) else {
            self.status = format!("unknown device {id}");
            return false;
        };
        if track >= self.song.tracks.len() {
            return false;
        }
        self.checkpoint();
        self.song_mut().tracks[track].devices.push(spec);
        self.sync();
        true
    }

    pub fn device_remove(&mut self, track: usize, index: usize) {
        if self.song.tracks.get(track).is_some_and(|t| index < t.devices.len()) {
            self.checkpoint();
            self.song_mut().tracks[track].devices.remove(index);
            // Automation lanes pointing at later devices follow them; lanes on the removed one go away.
            for p in &mut self.song_mut().patterns {
                if !p
                    .automation
                    .iter()
                    .any(|a| a.track == track && matches!(a.target, AutoTarget::Device { .. }))
                {
                    continue;
                }
                let p = Arc::make_mut(p);
                p.automation.retain(|a| {
                    !(a.track == track
                        && matches!(a.target, AutoTarget::Device { device, .. } if device == index))
                });
                for a in &mut p.automation {
                    if let (true, AutoTarget::Device { device, .. }) = (a.track == track, &mut a.target) {
                        if *device > index {
                            *device -= 1;
                        }
                    }
                }
            }
            self.sync();
        }
    }

    /// Move a device up (`delta < 0`) or down the chain.
    pub fn device_move(&mut self, track: usize, index: usize, delta: i32) {
        let Some(n) = self.song.tracks.get(track).map(|t| t.devices.len()) else { return };
        let to = index as i32 + delta;
        if index >= n || to < 0 || to as usize >= n {
            return;
        }
        self.checkpoint();
        self.song_mut().tracks[track].devices.swap(index, to as usize);
        self.sync();
    }

    pub fn device_toggle_bypass(&mut self, track: usize, index: usize) {
        if self.song.tracks.get(track).is_some_and(|t| index < t.devices.len()) {
            self.checkpoint();
            let d = &mut self.song_mut().tracks[track].devices[index];
            d.bypass = !d.bypass;
            self.sync();
        }
    }

    /// Set a device parameter; dragging a slider is one undo step.
    pub fn device_set_param(&mut self, track: usize, index: usize, param: usize, value: f32) {
        let Some(spec) = self.song.tracks.get(track).and_then(|t| t.devices.get(index)) else { return };
        let Some((_, min, max, _, _, _, _)) = spec.param_meta(param) else { return };
        if !(value.is_finite() && min.is_finite() && max.is_finite()) {
            return;
        }
        let value = value.clamp(min.min(max), min.max(max));
        let defaults: Vec<f32> =
            (0..spec.param_count()).filter_map(|i| spec.param_meta(i).map(|m| m.3)).collect();
        self.checkpoint_key("device_param");
        let d = &mut self.song_mut().tracks[track].devices[index];
        if d.params.len() < defaults.len() {
            let have = d.params.len();
            d.params.extend_from_slice(&defaults[have..]);
        }
        d.params[param] = value;
        self.sync();
    }

    /// Set a device's text (the formula source of a formula modulator).
    pub fn device_set_text(&mut self, track: usize, index: usize, text: &str) {
        if !self.song.tracks.get(track).is_some_and(|t| t.devices.get(index).is_some_and(|d| d.text != text))
        {
            return;
        }
        let text: String = text.chars().take(512).collect();
        self.checkpoint_key("device_text");
        self.song_mut().tracks[track].devices[index].text = text;
        self.sync();
    }

    // ---- automation -------------------------------------------------------------------

    /// The automation lane for a target in the current pattern, if it has one.
    pub fn automation_lane(&self, track: usize, target: AutoTarget) -> Option<&Automation> {
        let pi = self.pattern_index()?;
        self.song.patterns.get(pi)?.automation.iter().find(|a| a.track == track && a.target == target)
    }

    /// Edit (creating if needed) the lane for `target` in the current pattern. Lanes left without
    /// points are removed. Repeated calls with the same key share one undo step.
    pub fn automation_edit<F: FnOnce(&mut Automation)>(
        &mut self,
        key: &'static str,
        track: usize,
        target: AutoTarget,
        f: F,
    ) {
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint_key(key);
        let p = Arc::make_mut(&mut self.song_mut().patterns[pi]);
        let i = match p.automation.iter().position(|a| a.track == track && a.target == target) {
            Some(i) => i,
            None => {
                p.automation.push(Automation::new(track, target));
                p.automation.len() - 1
            }
        };
        f(&mut p.automation[i]);
        if p.automation[i].points.is_empty() {
            p.automation.remove(i);
        }
        self.sync();
    }

    /// Copy the lane's points in `[from, to]` (positions kept relative to `from`).
    pub fn automation_copy(&mut self, track: usize, target: AutoTarget, from: f32, to: f32) -> usize {
        let pts: Vec<(f32, f32)> = self
            .automation_lane(track, target)
            .map(|a| a.points_in(from, to).iter().map(|p| (p.0 - from, p.1)).collect())
            .unwrap_or_default();
        let n = pts.len();
        self.auto_clip = pts;
        self.status = if n == 0 { "no points in that range".into() } else { format!("copied {n} point(s)") };
        n
    }

    /// Paste the copied points with the first one at `at`, replacing that span.
    pub fn automation_paste(&mut self, track: usize, target: AutoTarget, at: f32) {
        if self.auto_clip.is_empty() {
            self.status = "nothing copied".into();
            return;
        }
        let clip = self.auto_clip.clone();
        let max = self.lines_in_pattern().saturating_sub(1) as f32;
        self.automation_edit("auto_paste", track, target, |a| a.paste(&clip, at, max));
    }

    pub fn automation_clear_range(&mut self, track: usize, target: AutoTarget, from: f32, to: f32) {
        self.automation_edit("auto_clear_range", track, target, |a| a.clear_range(from, to));
    }

    pub fn automation_scale(&mut self, track: usize, target: AutoTarget, from: f32, to: f32, factor: f32) {
        if factor.is_finite() {
            self.automation_edit("auto_scale", track, target, |a| a.scale_range(from, to, 0.5, factor));
        }
    }

    pub fn automation_flip(&mut self, track: usize, target: AutoTarget, from: f32, to: f32) {
        self.automation_edit("auto_flip", track, target, |a| a.flip_range(from, to));
    }

    pub fn automation_reverse(&mut self, track: usize, target: AutoTarget, from: f32, to: f32) {
        self.automation_edit("auto_reverse", track, target, |a| a.reverse_range(from, to));
    }

    pub fn automation_shift(&mut self, track: usize, target: AutoTarget, from: f32, to: f32, delta: f32) {
        let max = self.lines_in_pattern().saturating_sub(1) as f32;
        self.automation_edit("auto_shift", track, target, |a| a.shift_range(from, to, delta, max));
    }

    pub fn automation_set_curve(&mut self, track: usize, target: AutoTarget, curve: trakr_core::AutoCurve) {
        self.automation_edit("auto_curve", track, target, |a| a.curve = curve);
    }
}
