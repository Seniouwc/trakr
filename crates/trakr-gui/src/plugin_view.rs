//! The "Plugin" tab of the instrument panel: play a hosted instrument plugin instead of samples.

use crate::GuiApp;
use eframe::egui;

impl GuiApp {
    pub fn plugin_instrument_ui(&mut self, ui: &mut egui::Ui) {
        let current = self.editor.current_instrument().and_then(|i| i.plugin.clone());
        match current {
            Some(spec) => {
                let name = spec.plugin.as_ref().map_or("plugin", |p| p.name.as_str()).to_string();
                ui.horizontal(|ui| {
                    ui.strong(name);
                    let key = trakr_editor::PluginKey::Instrument(self.editor.instrument as usize);
                    if self.editor.plugin_has_window(key) {
                        let open = self.editor.plugin_window_visible(key);
                        if ui
                            .button(if open { "Close window" } else { "Open window" })
                            .on_hover_text("The plugin's own window; its settings are saved with the song")
                            .clicked()
                        {
                            self.editor.plugin_window_toggle(key);
                        }
                    }
                    if ui
                        .button("Use samples again")
                        .on_hover_text("Remove the plugin; the samples play")
                        .clicked()
                    {
                        self.editor.instrument_clear_plugin();
                    }
                });
                for pi in 0..spec.param_count() {
                    let Some((pname, pmin, pmax, _, plog, pstepped, punit)) = spec.param_meta(pi) else {
                        continue;
                    };
                    let mut v = spec.param(pi);
                    ui.horizontal(|ui| {
                        ui.add_sized([110.0, 18.0], egui::Label::new(pname));
                        ui.spacing_mut().slider_width = 200.0;
                        let mut slider = egui::Slider::new(&mut v, pmin..=pmax).suffix(punit);
                        if plog {
                            slider = slider.logarithmic(true);
                        }
                        if pstepped {
                            slider = slider.step_by(1.0);
                        }
                        if ui.add(slider).changed() {
                            self.editor.instrument_plugin_set_param(pi, v);
                        }
                    });
                }
                ui.weak("Notes of this instrument play the plugin on the track they are entered on.");
            }
            None => {
                if !self.editor.has_plugin_host() {
                    ui.weak("This build cannot host plugins.");
                    return;
                }
                ui.label("Play a CLAP instrument plugin with this instrument.");
                ui.horizontal(|ui| {
                    if ui.button("Scan for plugins").clicked() {
                        self.editor.scan_plugins();
                    }
                    egui::ComboBox::from_id_salt("inst_plugin")
                        .selected_text("Choose instrument plugin")
                        .show_ui(ui, |ui| {
                            let list: Vec<_> =
                                self.editor.plugins().iter().filter(|p| p.instrument).cloned().collect();
                            if list.is_empty() {
                                ui.weak("None found (scan first)");
                            }
                            for p in list {
                                if ui.selectable_label(false, format!("{} ({})", p.name, p.vendor)).clicked()
                                {
                                    self.editor.instrument_set_plugin(&p);
                                }
                            }
                        });
                });
            }
        }
    }

    /// The "FX" tab: effects every note of this instrument passes through before joining its track.
    pub fn instrument_fx_ui(&mut self, ui: &mut egui::Ui) {
        use trakr_core::{DEVICES, MODULATORS};
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("inst_fx_add").selected_text("+ Add effect").show_ui(ui, |ui| {
                for d in DEVICES.iter().filter(|d| !MODULATORS.contains(&d.id) && d.id != "native.send") {
                    if ui.selectable_label(false, d.name).clicked() {
                        self.editor.instrument_fx_add(d.id);
                    }
                }
            });
            ui.weak("Runs on the instrument's notes before the track's own devices. Notes of one instrument on several tracks share the chain.");
        });
        let specs = self.editor.current_instrument().map(|i| i.fx.clone()).unwrap_or_default();
        if specs.is_empty() {
            ui.weak(
                "No effects. Add one to shape this instrument's sound on its own, whichever track plays it.",
            );
            return;
        }
        egui::ScrollArea::both().id_salt("inst_fx_scroll").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (index, spec) in specs.iter().enumerate() {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(280.0);
                            ui.horizontal(|ui| {
                                let mut on = !spec.bypass;
                                if ui.checkbox(&mut on, "").on_hover_text("Active").changed() {
                                    self.editor.instrument_fx_toggle_bypass(index);
                                }
                                ui.strong(spec.display_name());
                                if ui.small_button("<").clicked() {
                                    self.editor.instrument_fx_move(index, -1);
                                }
                                if ui.small_button(">").clicked() {
                                    self.editor.instrument_fx_move(index, 1);
                                }
                                if ui.small_button("x").on_hover_text("Remove").clicked() {
                                    self.editor.instrument_fx_remove(index);
                                }
                            });
                            for pi in 0..spec.param_count() {
                                let Some((pname, pmin, pmax, _, plog, pstepped, punit)) = spec.param_meta(pi)
                                else {
                                    continue;
                                };
                                let mut v = spec.param(pi);
                                ui.horizontal(|ui| {
                                    let label = pname.split(" (").next().unwrap_or(&pname).to_string();
                                    ui.add_sized([78.0, 18.0], egui::Label::new(label));
                                    ui.spacing_mut().slider_width = 90.0;
                                    let mut slider = egui::Slider::new(&mut v, pmin..=pmax).suffix(punit);
                                    if plog {
                                        slider = slider.logarithmic(true);
                                    }
                                    if pstepped {
                                        slider = slider.step_by(1.0);
                                    }
                                    if ui.add(slider).changed() {
                                        self.editor.instrument_fx_set_param(index, pi, v);
                                    }
                                });
                            }
                        });
                    });
                }
            });
        });
    }
}
