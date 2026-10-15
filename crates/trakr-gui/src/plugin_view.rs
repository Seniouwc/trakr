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
}
