//! Instrument modulation: per-note LFOs (including stepper and random), key / velocity / macro routings,
//! macro knobs, key tracking, velocity-to-cutoff and note fade-out.

use crate::GuiApp;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use trakr_core::{
    InstLfo, InstMod, LfoShape, LfoTarget, ModSource, MAX_INST_LFOS, MAX_INST_MODS, MAX_MACROS, MAX_STEPS,
};

const TARGETS: [(LfoTarget, &str); 6] = [
    (LfoTarget::Volume, "Volume"),
    (LfoTarget::Pan, "Pan"),
    (LfoTarget::Pitch, "Pitch (semitones)"),
    (LfoTarget::Cutoff, "Filter cutoff (octaves)"),
    (LfoTarget::Resonance, "Filter resonance (octaves)"),
    (LfoTarget::Drive, "Drive"),
];
const SHAPES: [(LfoShape, &str); 7] = [
    (LfoShape::Sine, "Sine"),
    (LfoShape::Triangle, "Triangle"),
    (LfoShape::Saw, "Saw"),
    (LfoShape::Square, "Square"),
    (LfoShape::Random, "Random (held)"),
    (LfoShape::SmoothRandom, "Random (smooth)"),
    (LfoShape::Stepper, "Stepper"),
];

fn max_depth(t: LfoTarget) -> f32 {
    match t {
        LfoTarget::Volume | LfoTarget::Pan | LfoTarget::Drive => 1.0,
        LfoTarget::Pitch => 12.0,
        LfoTarget::Cutoff => 5.0,
        LfoTarget::Resonance => 3.0,
    }
}

fn target_combo(ui: &mut egui::Ui, id: impl std::hash::Hash, t: &mut LfoTarget) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(TARGETS.iter().find(|x| x.0 == *t).map_or("", |x| x.1))
        .show_ui(ui, |ui| {
            for (v, name) in TARGETS {
                ui.selectable_value(t, v, name);
            }
        });
}

fn source_name(s: ModSource) -> String {
    match s {
        ModSource::Key => "Key".into(),
        ModSource::Velocity => "Velocity".into(),
        ModSource::Macro(k) => format!("Macro {}", k + 1),
    }
}

/// A row of draggable bars for a stepper's levels (-1..1).
fn step_editor(ui: &mut egui::Ui, l: &mut InstLfo) -> bool {
    let n = (l.steps as usize).clamp(1, MAX_STEPS);
    let size = Vec2::new((n as f32 * 18.0).max(120.0), 54.0);
    let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    let r = resp.rect;
    painter.rect_filled(r, 2.0, Color32::from_gray(16));
    let w = r.width() / n as f32;
    painter.line_segment(
        [Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y)],
        Stroke::new(1.0_f32, Color32::from_gray(50)),
    );
    for i in 0..n {
        let v = l.step_values[i].clamp(-1.0, 1.0);
        let bar = Rect::from_min_max(
            Pos2::new(r.left() + i as f32 * w + 1.0, r.center().y.min(r.center().y - v * r.height() * 0.5)),
            Pos2::new(
                r.left() + (i + 1) as f32 * w - 1.0,
                r.center().y.max(r.center().y - v * r.height() * 0.5),
            ),
        );
        painter.rect_filled(bar, 1.0, Color32::from_rgb(120, 180, 255));
    }
    let mut changed = false;
    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let i = (((p.x - r.left()) / w) as usize).min(n - 1);
            let v = (1.0 - (p.y - r.top()) / r.height() * 2.0).clamp(-1.0, 1.0);
            l.step_values[i] = (v * 20.0).round() / 20.0;
            changed = true;
        }
    }
    changed
}

impl GuiApp {
    pub fn modulation_ui(&mut self, ui: &mut egui::Ui) {
        let Some(inst) = self.editor.current_instrument() else { return };
        let lfos = inst.lfos.clone();
        let mods = inst.mods.clone();
        let (key, vel, fade_out, macros, drive) =
            (inst.key_track, inst.vel_cutoff, inst.fade_out, inst.macros, inst.drive);

        ui.horizontal(|ui| {
            ui.strong("Per-note LFOs");
            if ui.add_enabled(lfos.len() < MAX_INST_LFOS, egui::Button::new("Add LFO")).clicked() {
                self.editor.edit_instrument("lfo_add", |i| i.lfos.push(InstLfo::default()));
            }
            ui.weak("Each note runs its own copy. Filter targets need the instrument filter turned on.");
        });
        for (n, l) in lfos.iter().enumerate() {
            let mut cur = *l;
            ui.horizontal(|ui| {
                target_combo(ui, ("lfo_t", n), &mut cur.target);
                egui::ComboBox::from_id_salt(("lfo_s", n))
                    .selected_text(SHAPES.iter().find(|t| t.0 == cur.shape).map_or("", |t| t.1))
                    .show_ui(ui, |ui| {
                        for (t, name) in SHAPES {
                            ui.selectable_value(&mut cur.shape, t, name);
                        }
                    });
                ui.add(egui::DragValue::new(&mut cur.rate).range(0.05..=30.0).speed(0.05).suffix(" Hz"));
                ui.label("Depth");
                ui.add(egui::DragValue::new(&mut cur.depth).range(0.0..=max_depth(cur.target)).speed(0.01));
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
            if cur.shape == LfoShape::Stepper {
                ui.horizontal(|ui| {
                    ui.label("   Steps");
                    let mut steps = cur.steps as u32;
                    if ui.add(egui::DragValue::new(&mut steps).range(1..=MAX_STEPS as u32)).changed() {
                        cur.steps = steps as u8;
                    }
                    step_editor(ui, &mut cur);
                });
            }
            if cur != *l {
                self.editor.edit_instrument("lfo_edit", |i| {
                    if let Some(x) = i.lfos.get_mut(n) {
                        *x = cur;
                    }
                });
            }
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Routings");
            if ui.add_enabled(mods.len() < MAX_INST_MODS, egui::Button::new("Add routing")).clicked() {
                self.editor.edit_instrument("mod_add", |i| {
                    i.mods.push(InstMod { source: ModSource::Key, target: LfoTarget::Cutoff, depth: 1.0 });
                });
            }
            ui.weak("Send the played key, the velocity or a macro knob to a target.");
        });
        for (n, m) in mods.iter().enumerate() {
            let mut cur = *m;
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt(("mod_s", n)).selected_text(source_name(cur.source)).show_ui(
                    ui,
                    |ui| {
                        ui.selectable_value(&mut cur.source, ModSource::Key, "Key");
                        ui.selectable_value(&mut cur.source, ModSource::Velocity, "Velocity");
                        for k in 0..MAX_MACROS as u8 {
                            ui.selectable_value(
                                &mut cur.source,
                                ModSource::Macro(k),
                                format!("Macro {}", k + 1),
                            );
                        }
                    },
                );
                ui.label("->");
                target_combo(ui, ("mod_t", n), &mut cur.target);
                let lim = max_depth(cur.target);
                ui.add(egui::DragValue::new(&mut cur.depth).range(-lim..=lim).speed(0.01).prefix("depth "));
                if ui.small_button("x").clicked() {
                    self.editor.edit_instrument("mod_remove", |i| {
                        if n < i.mods.len() {
                            i.mods.remove(n);
                        }
                    });
                }
            });
            if cur != *m {
                self.editor.edit_instrument("mod_edit", |i| {
                    if let Some(x) = i.mods.get_mut(n) {
                        *x = cur;
                    }
                });
            }
        }

        ui.separator();
        ui.strong("Macros");
        let mut macros2 = macros;
        ui.horizontal(|ui| {
            for (k, v) in macros2.iter_mut().enumerate() {
                ui.label(format!("M{}", k + 1));
                ui.add(egui::Slider::new(v, 0.0..=1.0).fixed_decimals(2));
            }
        });
        ui.weak("Pattern commands M1 to M4 (amount 00-FF) move the macros while a track plays.");
        if macros2 != macros {
            self.editor.edit_instrument("macros", |i| i.macros = macros2);
        }

        ui.separator();
        let (mut key2, mut vel2, mut fo2, mut drive2) = (key, vel, fade_out, drive);
        ui.horizontal_wrapped(|ui| {
            ui.label("Key tracking");
            ui.add(egui::Slider::new(&mut key2, 0.0..=2.0))
                .on_hover_text("How far the filter cutoff follows the key, around C-4");
            ui.label("Velocity to cutoff");
            ui.add(egui::Slider::new(&mut vel2, 0.0..=5.0).suffix(" oct"))
                .on_hover_text("Octaves of cutoff lost at the lowest velocity");
            ui.label("Drive");
            ui.add(egui::Slider::new(&mut drive2, 0.0..=1.0).fixed_decimals(2))
                .on_hover_text("Saturation applied to every note after the filter; LFOs and macros can move it");
            ui.label("Fade out");
            ui.add(egui::DragValue::new(&mut fo2).range(0.0..=60.0).speed(0.01).suffix(" s"))
                .on_hover_text("How long a note takes to fade away after note-off (0 = off)");
        });
        if key2 != key {
            self.editor.edit_instrument("key_track", |i| i.key_track = key2);
        }
        if vel2 != vel {
            self.editor.edit_instrument("vel_cutoff", |i| i.vel_cutoff = vel2);
        }
        if drive2 != drive {
            self.editor.edit_instrument("drive", |i| i.drive = drive2);
        }
        if fo2 != fade_out {
            self.editor.edit_instrument("fade_out", |i| i.fade_out = fo2);
        }
    }
}
