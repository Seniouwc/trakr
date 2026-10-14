//! Device chain editor and pattern automation lanes for the selected track.

use crate::GuiApp;
use eframe::egui::{self, Color32, PointerButton, Pos2, Sense, Stroke, Vec2};
use trakr_core::device::FILTER_MODES;
use trakr_core::{device_info, AutoTarget, DEVICES};

fn target_label(editor: &trakr_editor::Editor, track: usize, target: AutoTarget) -> String {
    match target {
        AutoTarget::TrackVolume => "Track volume".into(),
        AutoTarget::TrackPan => "Track pan".into(),
        AutoTarget::Device { device, param } => editor
            .song
            .tracks
            .get(track)
            .and_then(|t| t.devices.get(device))
            .and_then(|d| d.param_meta(param).map(|m| format!("{} / {}", d.display_name(), m.0)))
            .unwrap_or_else(|| "(missing device)".into()),
    }
}

impl GuiApp {
    pub fn devices_ui(&mut self, ui: &mut egui::Ui) {
        let track = self.editor.cursor.track;
        let Some(name) = self.editor.song.tracks.get(track).map(|t| t.name.clone()) else { return };
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Devices on {name}"));
            ui.label("(pick another track in the pattern editor or mixer)");
            ui.separator();
            egui::ComboBox::from_id_salt("add_device").selected_text("+ Add device").show_ui(ui, |ui| {
                for d in DEVICES {
                    if ui.selectable_label(false, d.name).clicked() {
                        self.editor.device_add(track, d.id);
                    }
                }
            });
            if self.editor.has_plugin_host() {
                egui::ComboBox::from_id_salt("add_plugin").selected_text("+ Add plugin").show_ui(ui, |ui| {
                    if ui
                        .button("Scan for plugins")
                        .on_hover_text("Looks in ~/.clap, /usr/lib/clap, /usr/local/lib/clap and $CLAP_PATH")
                        .clicked()
                    {
                        self.editor.scan_plugins();
                    }
                    ui.separator();
                    if self.editor.plugins().is_empty() {
                        ui.weak("None found yet (CLAP effects only)");
                    }
                    for p in self.editor.plugins().to_vec() {
                        if ui.selectable_label(false, format!("{} ({})", p.name, p.vendor)).clicked() {
                            self.editor.device_add_plugin(track, &p);
                        }
                    }
                });
            }
        });
        let count = self.editor.song.tracks[track].devices.len();
        if count == 0 {
            ui.weak("No devices. Add an effect with the menu above; devices run in order, top to bottom, before the track fader.");
        }
        egui::ScrollArea::both().id_salt("dev_scroll").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for i in 0..count {
                    self.device_card(ui, track, i);
                }
            });
        });
    }

    fn device_card(&mut self, ui: &mut egui::Ui, track: usize, index: usize) {
        let spec = self.editor.song.tracks[track].devices[index].clone();
        let info = device_info(&spec.id);
        let title = spec.display_name();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                ui.horizontal(|ui| {
                    let mut on = !spec.bypass;
                    if ui.checkbox(&mut on, "").on_hover_text("Active").changed() {
                        self.editor.device_toggle_bypass(track, index);
                    }
                    ui.strong(title);
                    if ui.small_button("<").on_hover_text("Move earlier in the chain").clicked() {
                        self.editor.device_move(track, index, -1);
                    }
                    if ui.small_button(">").on_hover_text("Move later in the chain").clicked() {
                        self.editor.device_move(track, index, 1);
                    }
                    if ui.small_button("x").on_hover_text("Remove").clicked() {
                        self.editor.device_remove(track, index);
                    }
                });
                if info.is_none() && spec.plugin.is_none() {
                    ui.weak("Unknown device: it passes audio through unchanged.");
                    return;
                }
                if spec.plugin.is_some() {
                    ui.weak("Plugin (runs inside trakr; no plugin window)");
                }
                for pi in 0..spec.param_count() {
                    let Some((pname, pmin, pmax, _, plog, pstepped, punit)) = spec.param_meta(pi) else {
                        continue;
                    };
                    let mut v = spec.param(pi);
                    ui.horizontal(|ui| {
                        let label = pname.split(" (").next().unwrap_or(&pname).to_string();
                        ui.add_sized([78.0, 18.0], egui::Label::new(label));
                        if spec.id == "native.filter" && pi == 0 {
                            let mut mode = v.round() as usize;
                            egui::ComboBox::from_id_salt(("fmode", track, index))
                                .selected_text(FILTER_MODES[mode.min(3)])
                                .show_ui(ui, |ui| {
                                    for (m, name) in FILTER_MODES.iter().enumerate() {
                                        ui.selectable_value(&mut mode, m, *name);
                                    }
                                });
                            if mode as f32 != v.round() {
                                self.editor.device_set_param(track, index, pi, mode as f32);
                            }
                        } else {
                            ui.spacing_mut().slider_width = 90.0;
                            let mut slider = egui::Slider::new(&mut v, pmin..=pmax).suffix(punit);
                            if plog {
                                slider = slider.logarithmic(true);
                            }
                            if pstepped {
                                slider = slider.step_by(1.0);
                            }
                            if ui.add(slider).changed() {
                                self.editor.device_set_param(track, index, pi, v);
                            }
                        }
                        let target = AutoTarget::Device { device: index, param: pi };
                        let has = self.editor.automation_lane(track, target).is_some();
                        let btn = egui::Button::new("A").small().fill(if has {
                            Color32::from_rgb(40, 100, 160)
                        } else {
                            Color32::TRANSPARENT
                        });
                        if ui.add(btn).on_hover_text("Automate this parameter over the pattern").clicked() {
                            self.auto_target = Some((track, target));
                            self.dock = crate::Dock::Automation;
                        }
                    });
                }
                if info.is_some_and(|i| trakr_core::MODULATORS.contains(&i.id)) {
                    let slot = spec.param(0).round() as usize;
                    let text = if slot == 0 {
                        "Not routed: set Target device to the slot of the device to modulate".to_string()
                    } else {
                        let tspec = self.editor.song.tracks[track].devices.get(slot - 1);
                        match tspec.and_then(|d| device_info(&d.id)) {
                            Some(ti) => {
                                let pi =
                                    (spec.param(1).round() as usize).min(ti.params.len().saturating_sub(1));
                                format!(
                                    "Modulates {} / {}",
                                    ti.name,
                                    ti.params.get(pi).map_or("", |p| p.name)
                                )
                            }
                            None => format!("Slot {slot} has no device"),
                        }
                    };
                    ui.weak(text);
                }
            });
        });
    }

    pub fn automation_ui(&mut self, ui: &mut egui::Ui) {
        let track = self.editor.cursor.track;
        let pattern = self.editor.pattern_index().unwrap_or(0);
        // Everything automatable on the selected track.
        let mut targets: Vec<(usize, AutoTarget)> =
            vec![(track, AutoTarget::TrackVolume), (track, AutoTarget::TrackPan)];
        for (di, d) in self.editor.song.tracks[track].devices.iter().enumerate() {
            targets
                .extend((0..d.param_count()).map(|pi| (track, AutoTarget::Device { device: di, param: pi })));
        }
        // Lanes that already exist in this pattern (on any track).
        let existing: Vec<(usize, AutoTarget)> = self
            .editor
            .song
            .patterns
            .get(pattern)
            .map(|p| p.automation.iter().map(|a| (a.track, a.target)).collect())
            .unwrap_or_default();

        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Automation, pattern {pattern:02X}"));
            let current = self.auto_target.map_or("Choose a parameter".to_string(), |(t, tg)| {
                format!(
                    "{}: {}",
                    self.editor.song.tracks.get(t).map_or("?", |tr| tr.name.as_str()),
                    target_label(&self.editor, t, tg)
                )
            });
            egui::ComboBox::from_id_salt("auto_target").width(300.0).selected_text(current).show_ui(
                ui,
                |ui| {
                    for (t, tg) in &targets {
                        let label = target_label(&self.editor, *t, *tg);
                        let mark = if self.editor.automation_lane(*t, *tg).is_some() { " *" } else { "" };
                        if ui
                            .selectable_label(self.auto_target == Some((*t, *tg)), format!("{label}{mark}"))
                            .clicked()
                        {
                            self.auto_target = Some((*t, *tg));
                        }
                    }
                },
            );
            if let Some((t, tg)) = self.auto_target {
                if ui.button("Clear lane").clicked() {
                    self.editor.automation_edit("auto_clear", t, tg, |a| a.points.clear());
                }
            }
            if !existing.is_empty() {
                ui.separator();
                ui.label("Lanes here:");
                for (t, tg) in existing {
                    let label = format!(
                        "{}: {}",
                        self.editor.song.tracks.get(t).map_or("?", |tr| tr.name.as_str()),
                        target_label(&self.editor, t, tg)
                    );
                    if ui.small_button(label).clicked() {
                        self.auto_target = Some((t, tg));
                    }
                }
            }
        });
        if self.auto_target.is_none() {
            self.auto_target = self
                .editor
                .song
                .patterns
                .get(pattern)
                .and_then(|p| p.automation.first())
                .map(|a| (a.track, a.target));
        }
        let Some((t, tg)) = self.auto_target else {
            ui.weak("Pick a parameter (or press A next to a device parameter), then double-click the graph to add points. The value follows the curve while the pattern plays.");
            return;
        };
        self.automation_graph(ui, t, tg);
    }

    fn automation_graph(&mut self, ui: &mut egui::Ui, track: usize, target: AutoTarget) {
        let lines = self.editor.lines_in_pattern() as f32;
        let points: Vec<(f32, f32)> =
            self.editor.automation_lane(track, target).map(|a| a.points.clone()).unwrap_or_default();
        let size = Vec2::new(ui.available_width(), ui.available_height().max(90.0));
        let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let r = resp.rect.shrink2(Vec2::new(10.0, 8.0));
        painter.rect_filled(resp.rect, 2.0, Color32::from_gray(14));
        let to_pos =
            |l: f32, v: f32| Pos2::new(r.left() + l / lines * r.width(), r.bottom() - v * r.height());
        let from_pos = |p: Pos2| {
            ((p.x - r.left()) / r.width() * lines, ((r.bottom() - p.y) / r.height()).clamp(0.0, 1.0))
        };
        let step = if lines > 128.0 {
            16.0
        } else if lines > 32.0 {
            8.0
        } else {
            4.0
        };
        let mut l = 0.0;
        while l <= lines {
            let x = to_pos(l, 0.0).x;
            painter.line_segment(
                [Pos2::new(x, r.top()), Pos2::new(x, r.bottom())],
                Stroke::new(1.0_f32, Color32::from_gray(if l % (step * 4.0) == 0.0 { 40 } else { 26 })),
            );
            painter.text(
                Pos2::new(x + 2.0, r.bottom() - 12.0),
                egui::Align2::LEFT_TOP,
                format!("{}", l as u32),
                egui::FontId::monospace(10.0),
                Color32::from_gray(90),
            );
            l += step;
        }
        for i in 0..=4 {
            let y = r.bottom() - r.height() * i as f32 / 4.0;
            painter.line_segment(
                [Pos2::new(r.left(), y), Pos2::new(r.right(), y)],
                Stroke::new(1.0_f32, Color32::from_gray(26)),
            );
        }
        let accent = Color32::from_rgb(80, 160, 240);
        let pts: Vec<Pos2> = points.iter().map(|&(l, v)| to_pos(l, v)).collect();
        if pts.len() > 1 {
            // Hold the first and last values to the edges, like the engine does.
            let mut line = vec![Pos2::new(r.left(), pts[0].y)];
            line.extend(pts.iter().copied());
            line.push(Pos2::new(r.right(), pts[pts.len() - 1].y));
            painter.add(egui::Shape::line(line, Stroke::new(2.0_f32, accent)));
        } else if let Some(p) = pts.first() {
            painter.line_segment(
                [Pos2::new(r.left(), p.y), Pos2::new(r.right(), p.y)],
                Stroke::new(2.0_f32, accent),
            );
        }
        for p in &pts {
            painter.circle_filled(*p, 4.5, accent);
        }
        // Value readout for the parameter under the pointer.
        if let (Some(pos), AutoTarget::Device { device, param }) = (resp.hover_pos(), target) {
            let (_, v) = from_pos(pos);
            let spec = self.editor.song.tracks.get(track).and_then(|tr| tr.devices.get(device));
            if let Some((value, unit)) =
                spec.and_then(|d| Some((d.denormalize_param(param, v)?, d.param_meta(param)?.6)))
            {
                painter.text(
                    pos + Vec2::new(12.0, -14.0),
                    egui::Align2::LEFT_TOP,
                    format!("{value:.2}{unit}"),
                    egui::FontId::monospace(11.0),
                    Color32::WHITE,
                );
            }
        }
        // Playhead.
        let (playing, seq, line) = self.editor.playhead();
        if playing && seq == self.editor.seq_pos {
            let x = to_pos(line as f32, 0.0).x;
            painter.line_segment(
                [Pos2::new(x, r.top()), Pos2::new(x, r.bottom())],
                Stroke::new(1.0_f32, Color32::from_rgb(90, 220, 120)),
            );
        }

        let nearest = |pos: Pos2| pts.iter().position(|p| p.distance(pos) <= 10.0);
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.drag_started_by(PointerButton::Primary) {
                self.auto_drag = nearest(pos);
            }
            if resp.dragged_by(PointerButton::Primary) {
                if let Some(i) = self.auto_drag {
                    let (l, v) = from_pos(pos);
                    let l = (l * 4.0).round() / 4.0; // quarter-line grid
                    self.editor.automation_edit("auto_drag", track, target, |a| a.move_point(i, l, v));
                }
            }
            if resp.double_clicked() && nearest(pos).is_none() {
                let (l, v) = from_pos(pos);
                let l = ((l * 4.0).round() / 4.0).clamp(0.0, lines);
                self.editor.automation_edit("auto_add", track, target, |a| {
                    a.set_point(l, v);
                });
            }
            if resp.secondary_clicked() {
                if let Some(i) = nearest(pos) {
                    self.editor.automation_edit("auto_remove", track, target, |a| a.remove_point(i));
                }
            }
        }
        if resp.drag_stopped() {
            self.auto_drag = None;
        }
    }
}
