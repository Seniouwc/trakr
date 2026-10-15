//! Song structure: the pattern matrix (tracks x sequence slots) and the song properties window.

use crate::GuiApp;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use trakr_core::TrackKind;

const CELL: Vec2 = Vec2::new(30.0, 18.0);

impl GuiApp {
    /// Rows are tracks, columns are sequence slots. A filled cell has notes; click one to mute that track
    /// in that slot only. Click a slot number to jump there.
    pub fn matrix_ui(&mut self, ui: &mut egui::Ui) {
        let (playing, play_seq, _) = self.editor.playhead();
        let slots = self.editor.song.sequence.len();
        let tracks: Vec<usize> = self
            .editor
            .song
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t.kind, TrackKind::Sequencer | TrackKind::Group))
            .map(|(i, _)| i)
            .take(64)
            .collect();
        ui.horizontal_wrapped(|ui| {
            ui.strong("Pattern matrix");
            ui.weak("click a cell to mute that track in that slot; click a slot number to go there");
            if ui
                .small_button("Section starts here")
                .on_hover_text("Name a group of slots; Loop section plays it")
                .clicked()
            {
                self.editor.do_action(trakr_editor::Action::SectionToggle);
            }
            if ui.small_button("Loop section").clicked() {
                self.editor.do_action(trakr_editor::Action::PlaySection);
            }
        });
        let mut rename: Option<(usize, String)> = None;
        egui::ScrollArea::both().id_salt("matrix").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("matrix_grid").spacing([2.0, 2.0]).show(ui, |ui| {
                // section names above the slot numbers
                ui.label("");
                for slot in 0..slots {
                    let name = self.editor.song.slot_meta.get(slot).and_then(|m| m.section.clone());
                    match name {
                        Some(mut n) => {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut n)
                                    .desired_width(CELL.x)
                                    .font(egui::TextStyle::Small),
                            );
                            if r.changed() {
                                rename = Some((slot, n));
                            }
                        }
                        None => {
                            ui.label("");
                        }
                    }
                }
                ui.end_row();
                ui.label("");
                for slot in 0..slots {
                    let pi = self.editor.song.sequence[slot];
                    let label = egui::RichText::new(format!("{slot:02X}")).monospace().color(
                        if playing && play_seq == slot { Color32::LIGHT_GREEN } else { Color32::GRAY },
                    );
                    let r = ui
                        .selectable_label(self.editor.seq_pos == slot, label)
                        .on_hover_text(format!("pattern {pi:02X}"));
                    if r.clicked() {
                        self.editor.seq_goto(slot);
                    }
                }
                ui.end_row();
                for &t in &tracks {
                    let tr = &self.editor.song.tracks[t];
                    ui.label(egui::RichText::new(tr.name.clone()).color(if t == self.editor.cursor.track {
                        Color32::YELLOW
                    } else {
                        Color32::GRAY
                    }));
                    for slot in 0..slots {
                        let pi = self.editor.song.sequence[slot];
                        let notes = self
                            .editor
                            .song
                            .patterns
                            .get(pi)
                            .and_then(|p| p.tracks.get(t))
                            .map_or(0, |pt| pt.lines.len());
                        let muted = self.editor.song.slot_muted(slot, t);
                        let (rect, resp) = ui.allocate_exact_size(CELL, Sense::click());
                        let base =
                            if notes > 0 { Color32::from_rgb(50, 110, 150) } else { Color32::from_gray(32) };
                        let fill = if muted { Color32::from_rgb(90, 40, 40) } else { base };
                        let painter = ui.painter();
                        painter.rect_filled(rect, 3.0, fill);
                        if muted {
                            painter.line_segment(
                                [rect.left_top() + Vec2::splat(4.0), rect.right_bottom() - Vec2::splat(4.0)],
                                Stroke::new(1.5_f32, Color32::LIGHT_RED),
                            );
                            painter.line_segment(
                                [
                                    rect.left_bottom() + Vec2::new(4.0, -4.0),
                                    rect.right_top() + Vec2::new(-4.0, 4.0),
                                ],
                                Stroke::new(1.5_f32, Color32::LIGHT_RED),
                            );
                        }
                        if self.editor.seq_pos == slot && self.editor.cursor.track == t {
                            painter.rect_stroke(
                                rect,
                                3.0,
                                Stroke::new(1.5_f32, Color32::YELLOW),
                                egui::StrokeKind::Inside,
                            );
                        }
                        if playing && play_seq == slot {
                            painter.rect_filled(
                                Rect::from_min_size(
                                    Pos2::new(rect.left(), rect.bottom() - 3.0),
                                    Vec2::new(rect.width(), 3.0),
                                ),
                                0.0,
                                Color32::LIGHT_GREEN,
                            );
                        }
                        let resp = resp.on_hover_text(format!(
                            "slot {slot:02X}, pattern {pi:02X}: {notes} line(s) with content{}",
                            if muted { " (muted here)" } else { "" }
                        ));
                        if resp.clicked() {
                            self.editor.seq_goto(slot);
                            self.editor.set_cursor(t, 0, 0, self.editor.cursor.line);
                            self.editor.slot_toggle_mute(slot, t);
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some((slot, name)) = rename {
            self.editor.section_set(slot, Some(name));
        }
    }

    pub fn song_window(&mut self, ctx: &egui::Context) {
        if !self.show_song {
            return;
        }
        let mut open = true;
        egui::Window::new("Song properties").open(&mut open).default_width(420.0).show(ctx, |ui| {
            let mut name = self.editor.song.name.clone();
            let mut author = self.editor.song.author.clone();
            egui::Grid::new("song_info").num_columns(2).show(ui, |ui| {
                ui.label("Title");
                if ui.text_edit_singleline(&mut name).changed() {
                    self.editor.set_song_name(name);
                }
                ui.end_row();
                ui.label("Artist");
                if ui.text_edit_singleline(&mut author).changed() {
                    self.editor.set_song_author(author);
                }
                ui.end_row();
            });
            let (mut bpm, mut lpb, mut tpl) =
                (self.editor.song.bpm, self.editor.song.lpb as u32, self.editor.song.tpl as u32);
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("BPM");
                changed |= ui.add(egui::DragValue::new(&mut bpm).range(32.0..=999.0)).changed();
                ui.label("LPB");
                changed |= ui.add(egui::DragValue::new(&mut lpb).range(1..=32)).changed();
                ui.label("TPL");
                changed |= ui.add(egui::DragValue::new(&mut tpl).range(1..=16)).changed();
            });
            if changed {
                self.editor.set_tempo(bpm, lpb as u8, tpl as u8);
            }
            ui.separator();
            ui.strong("Groove");
            let mut groove = self.editor.song.groove.clone();
            let mut on = !groove.is_empty();
            if ui
                .checkbox(&mut on, "Shuffle the timing of lines")
                .on_hover_text(
                    "Each line of the cycle starts late (right) or early (left) by a fraction of a line",
                )
                .changed()
            {
                groove = if on { vec![0.0, 0.2, 0.0, 0.2] } else { Vec::new() };
                self.editor.set_groove(groove.clone());
            }
            if on {
                let mut edited = false;
                for (i, g) in groove.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Line {} of {}", i + 1, self.editor.song.groove.len()));
                        edited |= ui.add(egui::Slider::new(g, -0.45..=0.45).fixed_decimals(2)).changed();
                    });
                }
                ui.horizontal(|ui| {
                    if ui.small_button("Longer cycle").clicked()
                        && groove.len() < trakr_core::song::MAX_GROOVE
                    {
                        groove.push(0.0);
                        edited = true;
                    }
                    if ui.small_button("Shorter cycle").clicked() && groove.len() > 1 {
                        groove.pop();
                        edited = true;
                    }
                });
                if edited {
                    self.editor.set_groove(groove);
                }
            }
            ui.separator();
            ui.strong("Comments");
            let mut comments = self.editor.song.comments.clone();
            if ui
                .add(egui::TextEdit::multiline(&mut comments).desired_rows(6).desired_width(f32::INFINITY))
                .changed()
            {
                self.editor.set_song_comments(comments);
            }
        });
        self.show_song = open;
    }
}
