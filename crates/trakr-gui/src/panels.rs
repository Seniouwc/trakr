use crate::dialogs::FileMode;
use crate::GuiApp;
use eframe::egui;
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
                    self.editor.select_sample(0);
                    self.sample_sel = None;
                    self.wave_view = None;
                }
            }
        });
    }
}
