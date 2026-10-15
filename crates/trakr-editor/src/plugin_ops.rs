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

    // ---- plugin windows, state ------------------------------------------------------------

    /// Whether the hosted plugin behind `key` can open a window of its own.
    pub fn plugin_has_window(&self, key: PluginKey) -> bool {
        self.plugin_guis.get(&key).is_some_and(|g| g.has_window())
    }

    pub fn plugin_window_visible(&self, key: PluginKey) -> bool {
        self.plugin_guis.get(&key).is_some_and(|g| g.is_visible())
    }

    /// Open the plugin's window, or close it when it is already open.
    pub fn plugin_window_toggle(&mut self, key: PluginKey) {
        let Some(g) = self.plugin_guis.get_mut(&key) else {
            self.status = "this plugin is not loaded".into();
            return;
        };
        if !g.has_window() {
            self.status = "this plugin has no window of its own: use the sliders".into();
        } else if g.is_visible() {
            g.hide();
        } else if let Err(e) = g.show() {
            self.status = format!("plugin window: {e}");
        }
    }

    /// True while any plugin window is open (the frontend should then call `tick` often).
    pub fn plugin_windows_active(&self) -> bool {
        self.plugin_guis.values().any(|g| g.is_visible())
    }

    fn plugin_spec_mut(&mut self, key: PluginKey) -> Option<&mut DeviceSpec> {
        let song = Arc::make_mut(&mut self.song);
        match key {
            PluginKey::Device(t, d) => song.tracks.get_mut(t)?.devices.get_mut(d),
            PluginKey::Instrument(i) => song.instruments.get_mut(i)?.plugin.as_mut(),
        }
    }

    /// Main-thread plugin work, once per frame: timers and callbacks, then whatever the plugins changed
    /// themselves (parameters turned in their windows, saved state).
    pub(crate) fn plugin_poll(&mut self) {
        if let Some(h) = &mut self.plugin_host {
            h.poll();
        }
        if self.plugin_guis.is_empty() {
            return;
        }
        let mut states: Vec<(PluginKey, Vec<u8>)> = Vec::new();
        let mut params: Vec<(PluginKey, usize, f32)> = Vec::new();
        let mut changes = Vec::new();
        for (key, g) in self.plugin_guis.iter_mut() {
            if g.take_state_dirty() {
                if let Some(bytes) = g.save_state() {
                    states.push((*key, bytes));
                }
            }
            changes.clear();
            g.take_param_changes(&mut changes);
            params.extend(changes.iter().map(|(i, v)| (*key, *i, *v)));
        }
        if states.is_empty() && params.is_empty() {
            return;
        }
        for (key, bytes) in states {
            if bytes.len() <= trakr_core::plugin::MAX_PLUGIN_STATE {
                if let Some(spec) = self.plugin_spec_mut(key) {
                    spec.text = trakr_core::encode_state(&bytes);
                }
            }
        }
        for (key, i, v) in params {
            if let Some(spec) = self.plugin_spec_mut(key) {
                if let Some(p) = spec.plugin.as_ref().and_then(|p| p.params.get(i)) {
                    let v = p.clamp(v);
                    if spec.params.len() <= i {
                        let defaults: Vec<f32> =
                            spec.plugin.iter().flat_map(|p| p.params.iter().map(|q| q.default)).collect();
                        let have = spec.params.len();
                        spec.params.extend_from_slice(&defaults[have.min(defaults.len())..]);
                    }
                    if let Some(slot) = spec.params.get_mut(i) {
                        *slot = v;
                    }
                }
            }
        }
        self.sync();
    }
}
