//! Instrument modulation: per-note LFOs, key tracking and velocity-to-cutoff.

use crate::GuiApp;
use eframe::egui;
use trakr_core::{InstLfo, LfoShape, LfoTarget, MAX_INST_LFOS};

const TARGETS: [(LfoTarget, &str); 4] = [
    (LfoTarget::Volume, "Volume"),
    (LfoTarget::Pan, "Pan"),
    (LfoTarget::Pitch, "Pitch (semitones)"),
    (LfoTarget::Cutoff, "Filter cutoff (octaves)"),
];
const SHAPES: [(LfoShape, &str); 5] = [
    (LfoShape::Sine, "Sine"),
    (LfoShape::Triangle, "Triangle"),
    (LfoShape::Saw, "Saw"),
    (LfoShape::Square, "Square"),
    (LfoShape::Random, "Random"),
];

impl GuiApp {
    pub fn modulation_ui(&mut self, ui: &mut egui::Ui) {
        let Some(inst) = self.editor.current_instrument() else { return };
        let lfos = inst.lfos.clone();
        let (key, vel) = (inst.key_track, inst.vel_cutoff);
        ui.horizontal(|ui| {
            ui.strong("Per-note LFOs");
            if ui.add_enabled(lfos.len() < MAX_INST_LFOS, egui::Button::new("Add LFO")).clicked() {
                self.editor.edit_instrument("lfo_add", |i| i.lfos.push(InstLfo::default()));
            }
            ui.weak("Each note runs its own copy. Filter cutoff needs the instrument filter turned on.");
        });
        for (n, l) in lfos.iter().enumerate() {
            let mut cur = *l;
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("lfo_t", n))
                    .selected_text(TARGETS.iter().find(|t| t.0 == cur.target).map_or("", |t| t.1))
                    .show_ui(ui, |ui| {
                        for (t, name) in TARGETS {
                            ui.selectable_value(&mut cur.target, t, name);
                        }
                    });
                egui::ComboBox::from_id_salt(("lfo_s", n))
                    .selected_text(SHAPES.iter().find(|t| t.0 == cur.shape).map_or("", |t| t.1))
                    .show_ui(ui, |ui| {
                        for (t, name) in SHAPES {
                            ui.selectable_value(&mut cur.shape, t, name);
                        }
                    });
                ui.add(egui::DragValue::new(&mut cur.rate).range(0.05..=30.0).speed(0.05).suffix(" Hz"));
                let max = match cur.target {
                    LfoTarget::Volume | LfoTarget::Pan => 1.0,
                    LfoTarget::Pitch => 12.0,
                    LfoTarget::Cutoff => 5.0,
                };
                ui.label("Depth");
                ui.add(egui::DragValue::new(&mut cur.depth).range(0.0..=max).speed(0.01));
                ui.label("Fade in");
                ui.add(egui::DragValue::new(&mut cur.fade_in).range(0.0..=10.0).speed(0.05).suffix(" s"));
                if ui.small_button("x").on_hover_text("Remove this LFO").clicked() {
                    self.editor.edit_instrument("lfo_remove", |i| {
                        if n < i.lfos.len() {
                            i.lfos.remove(n);
                        }
                    });
                }
            });
            if cur != *l {
                self.editor.edit_instrument("lfo_edit", |i| {
                    if let Some(x) = i.lfos.get_mut(n) {
                        *x = cur;
                    }
                });
            }
        }
        ui.separator();
        let (mut key2, mut vel2) = (key, vel);
        ui.horizontal(|ui| {
            ui.label("Key tracking");
            ui.add(egui::Slider::new(&mut key2, 0.0..=2.0))
                .on_hover_text("How far the filter cutoff follows the key, around C-4");
            ui.label("Velocity to cutoff");
            ui.add(egui::Slider::new(&mut vel2, 0.0..=5.0).suffix(" oct"))
                .on_hover_text("Octaves of cutoff lost at the lowest velocity");
        });
        if key2 != key {
            self.editor.edit_instrument("key_track", |i| i.key_track = key2);
        }
        if vel2 != vel {
            self.editor.edit_instrument("vel_cutoff", |i| i.vel_cutoff = vel2);
        }
    }
}
