use crate::dialogs::FileMode;
use crate::GuiApp;
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use trakr_core::LoopMode;
use trakr_editor::Action;

impl GuiApp {
    pub fn side_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("side").resizable(true).default_width(230.0).show(ctx, |ui| {
            self.sequence_list(ui);
            ui.add_space(6.0);
            ui.separator();
            self.instrument_list(ui);
        });
    }

    fn sequence_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Sequence");
        ui.horizontal_wrapped(|ui| {
            for (label, a, tip) in [
                ("New", Action::SeqInsert, "Insert a new empty pattern after this slot"),
                ("Clone", Action::SeqClone, "Duplicate this pattern into a new slot"),
                ("Del", Action::SeqDelete, "Remove this slot"),
                ("Up", Action::SeqMoveUp, "Move slot up"),
                ("Down", Action::SeqMoveDown, "Move slot down"),
            ] {
                if ui.small_button(label).on_hover_text(tip).clicked() {
                    self.run_action(a);
                }
            }
        });
        let (playing, play_seq, _) = self.editor.playhead();
        let height = (ui.available_height() * 0.5).max(120.0);
        egui::ScrollArea::vertical().id_salt("seq").max_height(height).auto_shrink([false, false]).show(
            ui,
            |ui| {
                let n = self.editor.song.sequence.len();
                for slot in 0..n {
                    let pi = self.editor.song.sequence[slot];
                    let p = &self.editor.song.patterns[pi];
                    let name = if p.name.is_empty() { String::new() } else { format!(" {}", p.name) };
                    let marker = if playing && play_seq == slot { ">" } else { " " };
                    let text = format!("{marker}{slot:02X}  P{pi:02X}  {:3}{name}", p.number_of_lines);
                    let sel = slot == self.editor.seq_pos;
                    let r = ui.selectable_label(sel, egui::RichText::new(text).monospace());
                    if r.clicked() {
                        self.editor.seq_goto(slot);
                    }
                    if sel && self.editor.follow && playing {
                        r.scroll_to_me(Some(egui::Align::Center));
                    }
                }
            },
        );
        ui.horizontal(|ui| {
            if let Some(pi) = self.editor.pattern_index() {
                let mut lines = self.editor.song.patterns[pi].number_of_lines as u32;
                ui.label("Lines");
                if ui.add(egui::DragValue::new(&mut lines).range(1..=512)).changed() {
                    self.editor.set_pattern_lines(lines as u16);
                }
                if ui.small_button("<").on_hover_text("Use previous pattern in this slot").clicked() {
                    self.editor.do_action(Action::PatternPrev);
                }
                if ui.small_button(">").on_hover_text("Use next pattern in this slot").clicked() {
                    self.editor.do_action(Action::PatternNext);
                }
            }
        });
    }

    fn instrument_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Instruments");
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Import...").on_hover_text("Load a sample file as a new instrument").clicked()
            {
                self.open_dialog(FileMode::ImportSample);
            }
            if ui.small_button("Empty").clicked() {
                self.editor.instrument_add_empty();
            }
            if ui.small_button("Remove").clicked() {
                self.editor.do_action(Action::InstrumentRemove);
            }
            if ui.small_button("Preview").on_hover_text("Play the instrument at C-4").clicked() {
                self.editor.preview_note(48);
            }
        });
        egui::ScrollArea::vertical().id_salt("insts").auto_shrink([false, false]).show(ui, |ui| {
            if self.editor.song.instruments.is_empty() {
                ui.weak("No instruments. Use Import... to load a sample.");
            }
            for i in 0..self.editor.song.instruments.len() {
                let name = self.editor.song.instruments[i].name.clone();
                let sel = self.editor.instrument as usize == i;
                if ui
                    .selectable_label(sel, egui::RichText::new(format!("{i:02X}  {name}")).monospace())
                    .clicked()
                {
                    self.editor.instrument = i.min(0xFE) as u8;
                    self.sample_sel = None;
                }
            }
        });
    }

    pub fn sample_panel(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("sample").resizable(true).default_height(210.0).show(ctx, |ui| {
            let Some(inst) = self.editor.current_instrument() else {
                ui.weak("Select or import an instrument to edit its sample.");
                return;
            };
            let mut name = inst.name.clone();
            let mut volume = inst.volume;
            let has_sample = !inst.samples.is_empty();
            ui.horizontal(|ui| {
                ui.label("Instrument");
                if ui.text_edit_singleline(&mut name).changed() {
                    self.editor.instrument_rename(name);
                }
                ui.label("Volume");
                if ui.add(egui::DragValue::new(&mut volume).range(0.0..=4.0).speed(0.01)).changed() {
                    self.editor.instrument_set_volume(volume);
                }
            });
            if !has_sample {
                ui.weak("This instrument has no sample. Use File > Import sample.");
                return;
            }
            self.sample_controls(ui);
            self.waveform(ui);
        });
    }

    fn sample_controls(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.editor.current_instrument().and_then(|i| i.samples.first()) else { return };
        let frames = s.frames();
        let (mut base, mut fine, mut vol, mut pan) =
            (s.base_note as u32, s.fine_tune as i32, s.volume, s.panning);
        let (mut mode, mut ls, mut le) = (s.loop_mode, s.loop_start as u64, s.loop_end as u64);
        let info = format!("{} Hz, {} ch, {} frames", s.sample_rate, s.channels, frames);
        ui.horizontal_wrapped(|ui| {
            ui.monospace(info);
            ui.separator();
            ui.label("Base note");
            let c = ui.add(
                egui::DragValue::new(&mut base)
                    .range(0..=119)
                    .custom_formatter(|n, _| trakr_core::note::note_name(n as u8)),
            );
            if c.changed() {
                self.editor.edit_sample_coalesced("base", 0, |s| s.base_note = base as u8);
            }
            ui.label("Fine");
            if ui.add(egui::DragValue::new(&mut fine).range(-128..=127)).changed() {
                self.editor.edit_sample_coalesced("fine", 0, |s| s.fine_tune = fine as i8);
            }
            ui.label("Vol");
            if ui.add(egui::DragValue::new(&mut vol).range(0.0..=4.0).speed(0.01)).changed() {
                self.editor.edit_sample_coalesced("svol", 0, |s| s.volume = vol);
            }
            ui.label("Pan");
            if ui.add(egui::DragValue::new(&mut pan).range(-1.0..=1.0).speed(0.01)).changed() {
                self.editor.edit_sample_coalesced("span", 0, |s| s.panning = pan);
            }
        });
        ui.horizontal_wrapped(|ui| {
            let before = mode;
            egui::ComboBox::from_id_salt("loopmode").selected_text(format!("Loop: {mode:?}")).show_ui(
                ui,
                |ui| {
                    for m in [LoopMode::Off, LoopMode::Forward, LoopMode::Reverse, LoopMode::PingPong] {
                        ui.selectable_value(&mut mode, m, format!("{m:?}"));
                    }
                },
            );
            if mode != before {
                self.editor.edit_sample(0, |s| s.loop_mode = mode);
            }
            ui.label("Start");
            if ui.add(egui::DragValue::new(&mut ls).range(0..=frames as u64).speed(16.0)).changed() {
                self.editor.edit_sample_coalesced("ls", 0, |s| s.loop_start = (ls as usize).min(s.loop_end));
            }
            ui.label("End");
            if ui.add(egui::DragValue::new(&mut le).range(0..=frames as u64).speed(16.0)).changed() {
                self.editor.edit_sample_coalesced("le", 0, |s| s.loop_end = (le as usize).max(s.loop_start));
            }
            ui.separator();
            if let Some((a, b)) = self.sample_sel {
                if ui.button("Loop = selection").clicked() {
                    self.editor.edit_sample(0, |s| {
                        s.loop_start = a;
                        s.loop_end = b;
                        if s.loop_mode == LoopMode::Off {
                            s.loop_mode = LoopMode::Forward;
                        }
                    });
                }
                if ui.button("Trim to selection").clicked() {
                    self.editor.edit_sample(0, |s| s.trim(a, b));
                    self.sample_sel = None;
                }
            }
            if ui.button("Normalize").clicked() {
                self.editor.edit_sample(0, |s| s.normalize());
            }
            if ui.button("Reverse").clicked() {
                self.editor.edit_sample(0, |s| s.reverse());
            }
            if ui.button("Fade in").clicked() {
                let n = self.sample_sel.map_or(frames, |(a, b)| b - a).min(frames);
                self.editor.edit_sample(0, |s| s.fade_in(n));
            }
            if ui.button("Fade out").clicked() {
                let n = self.sample_sel.map_or(frames, |(a, b)| b - a).min(frames);
                self.editor.edit_sample(0, |s| s.fade_out(n));
            }
            if ui.button("To mono").clicked() {
                self.editor.edit_sample(0, |s| s.to_mono());
            }
        });
    }

    /// Waveform with loop markers; drag to select a range.
    fn waveform(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.editor.current_instrument().and_then(|i| i.samples.first()) else { return };
        let (frames, ch) = (s.frames().max(1), s.channels as usize);
        let (loop_mode, ls, le) = (s.loop_mode, s.loop_start, s.loop_end);
        let size = Vec2::new(ui.available_width(), ui.available_height().max(60.0));
        let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let r = resp.rect;
        painter.rect_filled(r, 2.0, Color32::from_gray(14));
        let mid = r.center().y;
        painter.line_segment(
            [Pos2::new(r.left(), mid), Pos2::new(r.right(), mid)],
            Stroke::new(1.0_f32, Color32::from_gray(40)),
        );
        let w = r.width().max(1.0) as usize;
        let half = r.height() * 0.5 - 2.0;
        for x in 0..w {
            let a = x * frames / w;
            let b = (((x + 1) * frames / w).max(a + 1)).min(frames);
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            for f in a..b {
                let v = s.data.get(f * ch).copied().unwrap_or(0.0);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            let px = r.left() + x as f32;
            painter.line_segment(
                [Pos2::new(px, mid - hi * half), Pos2::new(px, mid - lo * half - 0.5)],
                Stroke::new(1.0_f32, Color32::from_rgb(90, 200, 120)),
            );
        }
        let to_x = |f: usize| r.left() + f as f32 / frames as f32 * r.width();
        let to_frame = |x: f32| (((x - r.left()) / r.width()).clamp(0.0, 1.0) * frames as f32) as usize;
        if loop_mode != LoopMode::Off {
            for f in [ls, le] {
                painter.line_segment(
                    [Pos2::new(to_x(f), r.top()), Pos2::new(to_x(f), r.bottom())],
                    Stroke::new(1.0_f32, Color32::from_rgb(230, 170, 40)),
                );
            }
        }
        if let Some((a, b)) = self.sample_sel {
            let sel = Rect::from_min_max(Pos2::new(to_x(a), r.top()), Pos2::new(to_x(b), r.bottom()));
            painter.rect_filled(sel, 0.0, Color32::from_rgba_unmultiplied(80, 140, 255, 50));
        }
        if let Some(p) = resp.interact_pointer_pos() {
            if resp.drag_started() {
                self.sample_drag = Some(to_frame(p.x));
            }
            if resp.dragged() {
                if let Some(a) = self.sample_drag {
                    let f = to_frame(p.x);
                    self.sample_sel = Some((a.min(f), a.max(f)));
                }
            }
        }
        if resp.clicked() {
            self.sample_sel = None;
        }
    }
}
