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
        let Some(info) = self
            .song
            .tracks
            .get(track)
            .and_then(|t| t.devices.get(index))
            .and_then(|d| trakr_core::device_info(&d.id))
        else {
            return;
        };
        let Some(pi) = info.params.get(param) else { return };
        let value = value.clamp(pi.min, pi.max);
        self.checkpoint_key("device_param");
        let d = &mut self.song_mut().tracks[track].devices[index];
        if d.params.len() <= param {
            d.params = info.defaults();
        }
        d.params[param] = value;
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
}
