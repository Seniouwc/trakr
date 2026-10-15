//! Instrument envelope editor: drag points, double-click to add, right-click to remove.

use crate::GuiApp;
use eframe::egui::{self, Color32, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::Arc;
use trakr_core::{Envelope, Instrument};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnvKind {
    Volume,
    Pan,
    Pitch,
    Cutoff,
}

impl EnvKind {
    pub fn label(self) -> &'static str {
        match self {
            EnvKind::Volume => "Volume",
            EnvKind::Pan => "Pan",
            EnvKind::Pitch => "Pitch",
            EnvKind::Cutoff => "Cutoff",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            EnvKind::Volume => "volume multiplier over the life of the note (top = full)",
            EnvKind::Pan => "pan over the note: bottom = left, middle = center, top = right",
            EnvKind::Pitch => "pitch bend over the note: middle = no change, top/bottom = +/- 12 semitones",
            EnvKind::Cutoff => "filter cutoff over the note: middle = no change, top/bottom = +/- 5 octaves (needs the instrument filter on)",
        }
    }

    fn get(self, i: &Instrument) -> &Envelope {
        match self {
            EnvKind::Volume => &i.volume_env,
            EnvKind::Pan => &i.pan_env,
            EnvKind::Pitch => &i.pitch_env,
            EnvKind::Cutoff => &i.cutoff_env,
        }
    }

    fn get_mut(self, i: &mut Instrument) -> &mut Envelope {
        Arc::make_mut(match self {
            EnvKind::Volume => &mut i.volume_env,
            EnvKind::Pan => &mut i.pan_env,
            EnvKind::Pitch => &mut i.pitch_env,
            EnvKind::Cutoff => &mut i.cutoff_env,
        })
    }

    fn preset(self) -> Envelope {
        match self {
            EnvKind::Volume => Envelope::adsr(0.02, 0.2, 0.6, 0.3),
            EnvKind::Pan => {
                Envelope { enabled: true, points: vec![(0.0, 0.5), (1.0, 0.5)], ..Default::default() }
            }
            EnvKind::Pitch | EnvKind::Cutoff => {
                Envelope { enabled: true, points: vec![(0.0, 0.5), (0.5, 0.5)], ..Default::default() }
            }
        }
    }
}

impl GuiApp {
    pub fn envelope_editor(&mut self, ui: &mut egui::Ui, kind: EnvKind) {
        let Some(env) = self.editor.current_instrument().map(|i| kind.get(i).clone()) else { return };
        ui.horizontal_wrapped(|ui| {
            let mut on = env.enabled;
            if ui.checkbox(&mut on, "Enabled").changed() {
                self.editor.edit_instrument("env_enable", |i| {
                    let e = kind.get_mut(i);
                    e.enabled = on;
                    if on && e.points.is_empty() {
                        *e = kind.preset();
                    }
                });
            }
            if ui.button("Preset").on_hover_text("Replace with a starting shape").clicked() {
                self.editor.edit_instrument("env_preset", |i| *kind.get_mut(i) = kind.preset());
                self.env_sel = None;
            }
            if ui.button("Clear").clicked() {
                self.editor.edit_instrument("env_clear", |i| *kind.get_mut(i) = Envelope::default());
                self.env_sel = None;
            }
            ui.separator();
            ui.label("Sustain");
            let sel = self.env_sel.filter(|&i| i < env.points.len());
            if ui.add_enabled(sel.is_some(), egui::Button::new("Hold at selected point")).clicked() {
                if let Some(i) = sel {
                    self.editor.edit_instrument("env_sustain", |inst| {
                        let e = kind.get_mut(inst);
                        e.sustain = if e.sustain == Some(i) { None } else { Some(i) };
                    });
                }
            }
            ui.label(match env.loop_points {
                Some((a, b)) => format!("Loop {a}-{b}"),
                None => "No loop".to_string(),
            });
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Loop start"))
                .on_hover_text("Repeat from the selected point while the key is held")
                .clicked()
            {
                if let Some(i) = sel {
                    self.editor.edit_instrument("env_loop", |inst| {
                        let e = kind.get_mut(inst);
                        let end = e
                            .loop_points
                            .map_or(e.points.len().saturating_sub(1), |l| l.1)
                            .max(i + 1)
                            .min(e.points.len().saturating_sub(1));
                        e.loop_points = (end > i).then_some((i, end));
                    });
                }
            }
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Loop end"))
                .on_hover_text("...up to the selected point")
                .clicked()
            {
                if let Some(i) = sel {
                    self.editor.edit_instrument("env_loop", |inst| {
                        let e = kind.get_mut(inst);
                        let start = e.loop_points.map_or(0, |l| l.0).min(i.saturating_sub(1));
                        e.loop_points = (i > start).then_some((start, i));
                    });
                }
            }
            if ui.add_enabled(env.loop_points.is_some(), egui::Button::new("No loop")).clicked() {
                self.editor.edit_instrument("env_loop", |inst| kind.get_mut(inst).loop_points = None);
            }
            ui.separator();
            let mut curve = env.curve;
            egui::ComboBox::from_id_salt("env_curve")
                .selected_text(format!("Curve: {}", curve.label()))
                .show_ui(ui, |ui| {
                    for c in trakr_core::AutoCurve::ALL {
                        ui.selectable_value(&mut curve, c, c.label());
                    }
                });
            if curve != env.curve {
                self.editor.edit_instrument("env_curve", |i| kind.get_mut(i).curve = curve);
            }
            if ui.small_button("Flip").on_hover_text("Mirror the values").clicked() {
                self.editor.edit_instrument("env_flip", |i| kind.get_mut(i).flip());
            }
            if ui.small_button("Reverse").on_hover_text("Play the shape backwards").clicked() {
                self.editor.edit_instrument("env_reverse", |i| kind.get_mut(i).reverse());
            }
            if ui.small_button("x0.5").on_hover_text("Halve the depth around the middle").clicked() {
                self.editor.edit_instrument("env_scale", |i| kind.get_mut(i).scale_values(0.5));
            }
            if ui.small_button("x2").on_hover_text("Double the depth around the middle").clicked() {
                self.editor.edit_instrument("env_scale", |i| kind.get_mut(i).scale_values(2.0));
            }
            if ui.small_button("Slower").on_hover_text("Stretch the time axis by two").clicked() {
                self.editor.edit_instrument("env_time", |i| kind.get_mut(i).scale_time(2.0));
            }
            if ui.small_button("Faster").on_hover_text("Squeeze the time axis by two").clicked() {
                self.editor.edit_instrument("env_time", |i| kind.get_mut(i).scale_time(0.5));
            }
            if ui.small_button("Copy").on_hover_text("Copy this envelope").clicked() {
                self.env_clip = Some(env.clone());
            }
            if ui
                .add_enabled(self.env_clip.is_some(), egui::Button::new("Paste"))
                .on_hover_text(
                    "Replace this envelope with the copied one (also between instruments and envelope types)",
                )
                .clicked()
            {
                if let Some(c) = self.env_clip.clone() {
                    self.editor.edit_instrument("env_paste", |i| *kind.get_mut(i) = c);
                    self.env_sel = None;
                }
            }
            ui.label("View");
            ui.add(egui::DragValue::new(&mut self.env_span).range(0.1..=30.0).speed(0.05).suffix(" s"));
            ui.weak(kind.hint());
        });
        ui.weak("Drag points. Double-click to add a point. Right-click a point to delete it. The held point waits for note-off.");

        let size = Vec2::new(ui.available_width(), ui.available_height().max(100.0));
        let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let r = resp.rect.shrink2(Vec2::new(8.0, 8.0));
        painter.rect_filled(resp.rect, 2.0, Color32::from_gray(14));
        let span = self.env_span.max(env.end_time() * 1.1).max(0.1);
        let to_pos = |t: f32, v: f32| Pos2::new(r.left() + t / span * r.width(), r.bottom() - v * r.height());
        let from_pos = |p: Pos2| {
            ((p.x - r.left()) / r.width() * span, ((r.bottom() - p.y) / r.height()).clamp(0.0, 1.0))
        };

        // Grid.
        for i in 0..=4 {
            let y = r.bottom() - r.height() * i as f32 / 4.0;
            painter.line_segment(
                [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                Stroke::new(1.0_f32, Color32::from_gray(32)),
            );
        }
        let mut s = 0.0;
        while s <= span {
            let x = to_pos(s, 0.0).x;
            painter.line_segment(
                [Pos2::new(x, r.top()), Pos2::new(x, r.bottom())],
                Stroke::new(1.0_f32, Color32::from_gray(28)),
            );
            painter.text(
                Pos2::new(x + 2.0, r.bottom() - 12.0),
                egui::Align2::LEFT_TOP,
                format!("{s:.1}s"),
                egui::FontId::monospace(10.0),
                Color32::from_gray(90),
            );
            s += if span > 6.0 { 1.0 } else { 0.5 };
        }

        let accent = if env.enabled { Color32::from_rgb(90, 200, 120) } else { Color32::from_gray(110) };
        let pts: Vec<Pos2> = env.points.iter().map(|&(t, v)| to_pos(t, v)).collect();
        if pts.len() > 1 {
            if env.curve == trakr_core::AutoCurve::Linear {
                painter.add(egui::Shape::line(pts.clone(), Stroke::new(2.0_f32, accent)));
            } else {
                // draw the segments the way the engine evaluates them
                let mut line = Vec::new();
                for w in env.points.windows(2) {
                    for k in 0..24 {
                        let f = k as f32 / 24.0;
                        line.push(to_pos(
                            w[0].0 + (w[1].0 - w[0].0) * f,
                            w[0].1 + (w[1].1 - w[0].1) * env.curve.shape(f),
                        ));
                    }
                }
                line.push(pts[pts.len() - 1]);
                painter.add(egui::Shape::line(line, Stroke::new(2.0_f32, accent)));
            }
        }
        for (i, p) in pts.iter().enumerate() {
            let selected = self.env_sel == Some(i);
            painter.circle_filled(
                *p,
                if selected { 6.0 } else { 4.5 },
                if selected { Color32::WHITE } else { accent },
            );
            if env.sustain == Some(i) {
                painter.line_segment(
                    [Pos2::new(p.x, r.top()), Pos2::new(p.x, r.bottom())],
                    Stroke::new(1.0_f32, Color32::from_rgb(230, 170, 40)),
                );
                painter.text(
                    Pos2::new(p.x + 3.0, r.top()),
                    egui::Align2::LEFT_TOP,
                    "hold",
                    egui::FontId::monospace(10.0),
                    Color32::from_rgb(230, 170, 40),
                );
            }
        }
        let _ = Rect::NOTHING;

        let nearest = |pos: Pos2| pts.iter().position(|p| p.distance(pos) <= 10.0);
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.drag_started_by(PointerButton::Primary) {
                self.env_drag = nearest(pos);
                if self.env_drag.is_some() {
                    self.env_sel = self.env_drag;
                }
            }
            if resp.dragged_by(PointerButton::Primary) {
                if let Some(i) = self.env_drag {
                    let (t, v) = from_pos(pos);
                    self.editor.edit_instrument("env_drag", |inst| kind.get_mut(inst).move_point(i, t, v));
                }
            }
            if resp.double_clicked() && nearest(pos).is_none() {
                let (t, v) = from_pos(pos);
                let mut idx = 0;
                self.editor.edit_instrument("env_add", |inst| {
                    let e = kind.get_mut(inst);
                    e.enabled = true;
                    idx = e.insert(t, v);
                });
                self.env_sel = Some(idx);
            } else if resp.clicked() {
                self.env_sel = nearest(pos);
            }
            if resp.secondary_clicked() {
                if let Some(i) = nearest(pos) {
                    self.editor.edit_instrument("env_remove", |inst| kind.get_mut(inst).remove(i));
                    self.env_sel = None;
                }
            }
        }
        if resp.drag_stopped() {
            self.env_drag = None;
        }
    }
}
