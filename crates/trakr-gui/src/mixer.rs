//! Mixer strips: per-track volume, pan, mute, solo and level meters.

use crate::GuiApp;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Vec2};
use trakr_core::{MuteState, TrackKind};

const DB_MIN: f32 = -60.0;
const DB_MAX: f32 = 6.0;

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 0.0 {
        DB_MIN
    } else {
        (20.0 * g.log10()).clamp(DB_MIN, DB_MAX)
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    if db <= DB_MIN {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Vertical level bar for a linear peak value.
fn meter(ui: &mut egui::Ui, level: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 1.0, Color32::from_gray(20));
    let frac = ((gain_to_db(level) - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0);
    let color = if level > 1.0 {
        Color32::from_rgb(220, 50, 50)
    } else if level > 0.7 {
        Color32::from_rgb(220, 190, 50)
    } else {
        Color32::from_rgb(70, 200, 100)
    };
    let h = rect.height() * frac;
    p.rect_filled(
        Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - h), rect.right_bottom()),
        1.0,
        color,
    );
}

impl GuiApp {
    pub fn mixer_ui(&mut self, ui: &mut egui::Ui) {
        // Peak hold with decay, updated once per frame.
        let n = self.editor.song.tracks.len();
        self.meter_hold.resize(n + 2, 0.0);
        if let Some(e) = &self.editor.engine {
            for t in 0..n {
                self.meter_hold[t] = e.pos.track_peak(t).max(self.meter_hold[t] * 0.9);
            }
            let (l, r) = e.pos.master_peak();
            self.meter_hold[n] = l.max(self.meter_hold[n] * 0.9);
            self.meter_hold[n + 1] = r.max(self.meter_hold[n + 1] * 0.9);
        }
        egui::ScrollArea::horizontal().id_salt("mixer_scroll").show(ui, |ui| {
            ui.horizontal(|ui| {
                for t in 0..n {
                    self.strip(ui, t);
                    ui.separator();
                }
            });
        });
    }

    fn strip(&mut self, ui: &mut egui::Ui, t: usize) {
        let tr = &self.editor.song.tracks[t];
        let (name, kind, vol, pan, muted, solo) =
            (tr.name.clone(), tr.kind, tr.volume, tr.panning, tr.mute != MuteState::Active, tr.solo);
        let selected = t == self.editor.cursor.track;
        let n = self.editor.song.tracks.len();
        ui.vertical(|ui| {
            ui.set_width(64.0);
            if ui.selectable_label(selected, egui::RichText::new(&name).small()).clicked() {
                let line = self.editor.cursor.line;
                self.editor.set_cursor(t, 0, 0, line);
            }
            ui.horizontal(|ui| {
                let mb = egui::Button::new("M").small().fill(if muted {
                    Color32::from_rgb(160, 50, 50)
                } else {
                    Color32::TRANSPARENT
                });
                if ui.add(mb).on_hover_text("Mute").clicked() {
                    self.editor.track_toggle_mute_at(t);
                }
                if kind == TrackKind::Sequencer {
                    let sb = egui::Button::new("S").small().fill(if solo {
                        Color32::from_rgb(170, 140, 30)
                    } else {
                        Color32::TRANSPARENT
                    });
                    if ui.add(sb).on_hover_text("Solo").clicked() {
                        self.editor.track_toggle_solo(t);
                    }
                }
            });
            let mut db = gain_to_db(vol);
            ui.horizontal(|ui| {
                let s = egui::Slider::new(&mut db, DB_MIN..=DB_MAX).vertical().show_value(false);
                if ui.add_sized([18.0, 90.0], s).changed() {
                    self.editor.set_track_volume(t, db_to_gain(db));
                }
                if kind == TrackKind::Master {
                    meter(ui, self.meter_hold[n], 90.0);
                    meter(ui, self.meter_hold[n + 1], 90.0);
                } else {
                    meter(ui, self.meter_hold[t], 90.0);
                }
            });
            ui.small(if db <= DB_MIN { "-inf dB".to_string() } else { format!("{db:.1} dB") });
            let mut p = pan;
            if ui.add(egui::DragValue::new(&mut p).range(-1.0..=1.0).speed(0.01).fixed_decimals(2)).changed()
            {
                self.editor.set_track_panning(t, p);
            }
        });
    }
}
