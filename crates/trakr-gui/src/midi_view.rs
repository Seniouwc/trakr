//! MIDI mapping window: learn controllers for the mixer, device parameters and commands, and tempo sync.

use crate::GuiApp;
use eframe::egui;
use trakr_editor::{Action, MapTarget};

impl GuiApp {
    pub fn midi_window(&mut self, ctx: &egui::Context) {
        if !self.show_midi_map {
            self.editor.midi_learn_cancel();
            return;
        }
        let mut open = true;
        egui::Window::new("MIDI mapping").open(&mut open).default_width(460.0).show(ctx, |ui| {
            let track = self.editor.cursor.track;
            let tname = self.editor.song.tracks.get(track).map_or("?".to_string(), |t| t.name.clone());
            let mut sync = self.editor.prefs.midi_sync_in;
            if ui.checkbox(&mut sync, "Follow the tempo of an incoming MIDI clock").changed() {
                self.editor.prefs.midi_sync_in = sync;
            }
            ui.separator();
            ui.strong(format!("Learn a controller for the selected track ({tname})"));
            ui.horizontal(|ui| {
                if ui.button("Volume").clicked() {
                    self.editor.midi_learn_start(MapTarget::TrackVolume(track));
                }
                if ui.button("Pan").clicked() {
                    self.editor.midi_learn_start(MapTarget::TrackPan(track));
                }
            });
            let devs: Vec<(usize, String, usize)> = self
                .editor
                .song
                .tracks
                .get(track)
                .map(|t| {
                    t.devices
                        .iter()
                        .enumerate()
                        .map(|(i, d)| (i, d.display_name(), d.param_count()))
                        .collect()
                })
                .unwrap_or_default();
            if devs.is_empty() {
                ui.weak("Add a device to the track to map its parameters.");
            }
            for (di, name, np) in devs {
                ui.menu_button(format!("Device: {name}"), |ui| {
                    for pi in 0..np.min(64) {
                        let pname = self.editor.song.tracks[track].devices[di]
                            .param_meta(pi)
                            .map_or(String::new(), |m| m.0);
                        if ui.button(pname).clicked() {
                            self.editor.midi_learn_start(MapTarget::DeviceParam {
                                track,
                                device: di,
                                param: pi,
                            });
                            ui.close_menu();
                        }
                    }
                });
            }
            ui.menu_button("Command", |ui| {
                for a in Action::ALL {
                    if ui.button(a.label()).clicked() {
                        self.editor.midi_learn_start(MapTarget::Action(format!("{a:?}")));
                        ui.close_menu();
                    }
                }
            });
            if self.editor.midi_learn.is_some() {
                ui.colored_label(egui::Color32::YELLOW, "Move a controller on your device...");
                if ui.button("Cancel").clicked() {
                    self.editor.midi_learn_cancel();
                }
            }
            ui.separator();
            ui.strong("Mappings");
            let list: Vec<(usize, String)> = self
                .editor
                .prefs
                .midi_map
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let ch = m.channel.map_or("any".to_string(), |c| (c + 1).to_string());
                    (
                        i,
                        format!(
                            "CC {} (channel {ch}) -> {}",
                            m.controller,
                            m.target.describe(&self.editor.song)
                        ),
                    )
                })
                .collect();
            if list.is_empty() {
                ui.weak(
                    "None yet. CC 7 and CC 10 control the selected track's volume and pan unless remapped.",
                );
            }
            for (i, text) in list {
                ui.horizontal(|ui| {
                    ui.label(text);
                    if ui.small_button("Remove").clicked() {
                        self.editor.midi_unmap(i);
                    }
                });
            }
        });
        self.show_midi_map = open;
        if !open {
            self.editor.midi_learn_cancel();
        }
    }
}
