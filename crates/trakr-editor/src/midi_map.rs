//! MIDI learn: map controllers to mixer, device and command targets.

use super::*;
use serde::{Deserialize, Serialize};

/// What a mapped controller drives.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MapTarget {
    TrackVolume(usize),
    TrackPan(usize),
    DeviceParam {
        track: usize,
        device: usize,
        param: usize,
    },
    /// Runs the command (its `Action` name, e.g. `"PlaySong"`) when the controller goes above half.
    Action(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MidiMapping {
    /// `None` listens on every channel.
    pub channel: Option<u8>,
    pub controller: u8,
    pub target: MapTarget,
}

impl MapTarget {
    pub fn describe(&self, song: &Song) -> String {
        let name = |t: usize| song.tracks.get(t).map_or("?".to_string(), |t| t.name.clone());
        match self {
            MapTarget::TrackVolume(t) => format!("{} volume", name(*t)),
            MapTarget::TrackPan(t) => format!("{} pan", name(*t)),
            MapTarget::DeviceParam { track, device, param } => {
                let d = song.tracks.get(*track).and_then(|t| t.devices.get(*device));
                let p = d.and_then(|d| d.param_meta(*param)).map_or("?".to_string(), |m| m.0);
                format!("{} / {} / {}", name(*track), d.map_or("?".to_string(), |d| d.display_name()), p)
            }
            MapTarget::Action(a) => a.clone(),
        }
    }
}

/// Position 0..=1 of a track fader mapped like pattern automation (0 = silent, 60/66 = unity).
fn fader_gain(n: f32) -> f32 {
    if n <= 0.0 {
        0.0
    } else {
        10f32.powf((-60.0 + 66.0 * n) / 20.0)
    }
}

impl Editor {
    /// Start learning: the next controller that moves gets mapped to `target`.
    pub fn midi_learn_start(&mut self, target: MapTarget) {
        self.status = format!("move a MIDI controller to map it to {}", target.describe(&self.song));
        self.midi_learn = Some(target);
    }

    pub fn midi_learn_cancel(&mut self) {
        self.midi_learn = None;
    }

    pub fn midi_unmap(&mut self, index: usize) {
        if index < self.prefs.midi_map.len() {
            self.prefs.midi_map.remove(index);
        }
    }

    /// Handle a controller: complete a pending learn, or drive whatever it is mapped to.
    /// Returns true when the controller was consumed by a mapping.
    pub(crate) fn midi_cc(&mut self, channel: u8, controller: u8, value: u8) -> bool {
        if let Some(target) = self.midi_learn.take() {
            self.prefs
                .midi_map
                .retain(|m| !(m.controller == controller && m.channel.is_none_or(|c| c == channel)));
            self.status = format!("CC {controller} drives {}", target.describe(&self.song));
            self.prefs.midi_map.push(MidiMapping { channel: Some(channel), controller, target });
            return true;
        }
        let hits: Vec<MapTarget> = self
            .prefs
            .midi_map
            .iter()
            .filter(|m| m.controller == controller && m.channel.is_none_or(|c| c == channel))
            .map(|m| m.target.clone())
            .collect();
        if hits.is_empty() {
            return false;
        }
        let n = value as f32 / 127.0;
        for t in hits {
            match t {
                MapTarget::TrackVolume(t) => self.set_track_volume(t, fader_gain(n)),
                MapTarget::TrackPan(t) => self.set_track_panning(t, n * 2.0 - 1.0),
                MapTarget::DeviceParam { track, device, param } => {
                    let v = self
                        .song
                        .tracks
                        .get(track)
                        .and_then(|t| t.devices.get(device))
                        .and_then(|d| d.denormalize_param(param, n));
                    if let Some(v) = v {
                        self.device_set_param(track, device, param, v);
                    }
                }
                MapTarget::Action(name) => {
                    if value >= 64 {
                        if let Some(a) = Action::ALL.iter().copied().find(|a| format!("{a:?}") == name) {
                            self.do_action(a);
                        }
                    }
                }
            }
        }
        true
    }
}
