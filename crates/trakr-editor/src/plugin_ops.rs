//! Hosted plugins: scanning, adding to a track.

use super::*;
use trakr_core::{DeviceSpec, PluginInfo};

impl Editor {
    /// Give the editor a plugin host (the binary supplies one that loads CLAP files).
    pub fn set_plugin_host(&mut self, host: Box<dyn trakr_engine::PluginHost>) {
        self.plugin_host = Some(host);
        // Plugins in the song that loaded as pass-throughs before the host existed get built now.
        self.sent_chains.clear();
        self.sent_inst_plugins.clear();
        self.needs_sync = true;
        self.flush_sync();
    }

    pub fn has_plugin_host(&self) -> bool {
        self.plugin_host.is_some()
    }

    /// Look for plugins on this machine (can take a moment: every plugin file is opened).
    pub fn scan_plugins(&mut self) {
        let Some(host) = &mut self.plugin_host else {
            self.status = "this build cannot host plugins".into();
            return;
        };
        self.plugin_list = host.scan();
        self.status = format!("found {} plugin(s)", self.plugin_list.len());
    }

    /// Plugins found by the last scan.
    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugin_list
    }

    /// Append a hosted plugin to a track's device chain. Returns false (and says why) when it cannot load.
    pub fn device_add_plugin(&mut self, track: usize, plugin: &PluginInfo) -> bool {
        if track >= self.song.tracks.len() {
            return false;
        }
        let rate = self.engine.as_ref().map_or(44_100, |e| e.sample_rate);
        let Some(host) = &mut self.plugin_host else {
            self.status = "this build cannot host plugins".into();
            return false;
        };
        match host.load(&plugin.path, &plugin.plugin_id, rate) {
            Ok((device, _)) if device.is_instrument() => {
                self.status = format!("{} is an instrument: add it from the instrument panel", plugin.name);
                false
            }
            Ok((_device, reference)) => {
                self.checkpoint();
                self.song_mut().tracks[track].devices.push(DeviceSpec::from_plugin(reference));
                self.status = format!("added {}", plugin.name);
                self.sync();
                true
            }
            Err(e) => {
                self.status = format!("cannot load {}: {e}", plugin.name);
                false
            }
        }
    }

    /// Make the selected instrument play a hosted instrument plugin (its samples are kept but unused).
    pub fn instrument_set_plugin(&mut self, plugin: &PluginInfo) -> bool {
        let inst = self.instrument as usize;
        if inst >= self.song.instruments.len() {
            self.status = "no instrument selected".into();
            return false;
        }
        let rate = self.engine.as_ref().map_or(44_100, |e| e.sample_rate);
        let Some(host) = &mut self.plugin_host else {
            self.status = "this build cannot host plugins".into();
            return false;
        };
        match host.load(&plugin.path, &plugin.plugin_id, rate) {
            Ok((device, reference)) => {
                if !device.is_instrument() {
                    self.status = format!("{} is an effect: add it from Devices > Add plugin", plugin.name);
                    return false;
                }
                self.checkpoint();
                self.song_mut().instruments[inst].plugin = Some(DeviceSpec::from_plugin(reference));
                self.status = format!("instrument {inst:02X} now plays {}", plugin.name);
                self.sync();
                true
            }
            Err(e) => {
                self.status = format!("cannot load {}: {e}", plugin.name);
                false
            }
        }
    }

    /// Go back to sample playback for the selected instrument.
    pub fn instrument_clear_plugin(&mut self) {
        let inst = self.instrument as usize;
        if self.song.instruments.get(inst).is_some_and(|i| i.plugin.is_some()) {
            self.checkpoint();
            self.song_mut().instruments[inst].plugin = None;
            self.status = format!("instrument {inst:02X} plays its samples again");
            self.sync();
        }
    }

    /// Set a parameter of the selected instrument's plugin; dragging a slider is one undo step.
    pub fn instrument_plugin_set_param(&mut self, param: usize, value: f32) {
        let inst = self.instrument as usize;
        let Some(spec) = self.song.instruments.get(inst).and_then(|i| i.plugin.as_ref()) else { return };
        let Some((_, min, max, _, _, _, _)) = spec.param_meta(param) else { return };
        if !(value.is_finite() && min.is_finite() && max.is_finite()) {
            return;
        }
        let value = value.clamp(min.min(max), min.max(max));
        let defaults: Vec<f32> =
            (0..spec.param_count()).filter_map(|i| spec.param_meta(i).map(|m| m.3)).collect();
        self.checkpoint_key("inst_plugin_param");
        let Some(spec) = self.song_mut().instruments[inst].plugin.as_mut() else { return };
        if spec.params.len() < defaults.len() {
            let have = spec.params.len();
            spec.params.extend_from_slice(&defaults[have..]);
        }
        spec.params[param] = value;
        self.sync();
    }
}
