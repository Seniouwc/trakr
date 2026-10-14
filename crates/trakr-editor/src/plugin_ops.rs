//! Hosted plugins: scanning, adding to a track.

use super::*;
use trakr_core::{DeviceSpec, PluginInfo};

impl Editor {
    /// Give the editor a plugin host (the binary supplies one that loads CLAP files).
    pub fn set_plugin_host(&mut self, host: Box<dyn trakr_engine::PluginHost>) {
        self.plugin_host = Some(host);
        // Plugins in the song that loaded as pass-throughs before the host existed get built now.
        self.sent_chains.clear();
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
}
