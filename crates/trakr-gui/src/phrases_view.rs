//! Phrase editor dock: choose and configure the selected instrument's phrases, edit their lines.

use crate::GuiApp;
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Vec2};
use trakr_core::note::note_name;
use trakr_core::PhraseMode;
use trakr_editor::{cell_text, FX_HL, FX_SUBS, NOTE_HL, NOTE_SUBS};

const NOTE_COL_W: usize = 16;
const FX_COL_W: usize = 5;

impl GuiApp {
    pub fn phrases_ui(&mut self, ui: &mut egui::Ui) {
        let Some(inst) = self.editor.current_instrument() else {
            ui.label("Import a sample first: phrases belong to an instrument.");
            return;
        };
        let names: Vec<String> = inst.phrases.iter().map(|p| p.name.clone()).collect();
        let mode = inst.phrase_mode;
        let mut sel = self.editor.phrase.min(names.len().saturating_sub(1));

        ui.horizontal_wrapped(|ui| {
            if ui.button("Add phrase").clicked() {
                self.editor.phrase_add();
            }
            let has = !names.is_empty();
            if ui.add_enabled(has, egui::Button::new("Duplicate")).clicked() {
                self.editor.phrase_duplicate();
            }
            if ui.add_enabled(has, egui::Button::new("Remove")).clicked() {
                self.editor.phrase_remove();
            }
            if ui.add_enabled(has, egui::Button::new("Play")).clicked() {
                self.editor.phrase_preview();
            }
            if ui.add_enabled(has, egui::Button::new("Stop")).clicked() {
                self.editor.phrase_stop_preview();
            }
            ui.separator();
            let mut m = mode;
            egui::ComboBox::from_id_salt("phrase_mode")
                .selected_text(match mode {
                    PhraseMode::Off => "Phrases: off",
                    PhraseMode::Keymap => "Phrases: key mapped",
                    PhraseMode::Program => "Phrases: program",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut m, PhraseMode::Off, "Off (notes play the sample)");
                    ui.selectable_value(&mut m, PhraseMode::Keymap, "Key mapped (a note plays its phrase)");
                    ui.selectable_value(
                        &mut m,
                        PhraseMode::Program,
                        "Program (C-0 to B-0 pick a phrase, other notes transpose it)",
                    );
                });
            if m != mode {
                self.editor.phrase_set_mode(m);
            }
        });
        if names.is_empty() {
            ui.label("No phrases yet. A phrase is a small pattern that one note triggers, transposed to that note.");
            return;
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("phrase_sel")
                .selected_text(format!("{sel:02X} {}", names[sel]))
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut sel, i, format!("{i:02X} {n}"));
                    }
                });
            if sel != self.editor.phrase {
                self.editor.phrase_select(sel);
            }
        });
        let Some(p) = self.editor.current_phrase().cloned() else { return };
        let fmt = |v: f64, _: std::ops::RangeInclusive<usize>| note_name(v as u8);
        ui.horizontal_wrapped(|ui| {
            let mut name = p.name.clone();
            if ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0)).changed() {
                self.editor.edit_phrase("phrase_name", |p| p.name = name);
            }
            let mut lines = p.number_of_lines;
            ui.label("Lines");
            if ui.add(egui::DragValue::new(&mut lines).range(1..=512)).changed() {
                self.editor.edit_phrase("phrase_lines", |p| p.number_of_lines = lines);
            }
            let mut lpb = p.lpb;
            ui.label("LPB");
            if ui.add(egui::DragValue::new(&mut lpb).range(1..=32)).changed() {
                self.editor.edit_phrase("phrase_lpb", |p| p.lpb = lpb);
            }
            let mut shuffle = p.shuffle;
            ui.label("Shuffle");
            if ui
                .add(egui::DragValue::new(&mut shuffle).range(0.0..=0.75).speed(0.01).fixed_decimals(2))
                .on_hover_text(
                    "Swing: odd lines are shortened and even lines lengthened by this fraction of a line",
                )
                .changed()
            {
                self.editor.edit_phrase("phrase_shuffle", |p| p.shuffle = shuffle);
            }
            let (mut nc, mut ec) = (p.note_columns, p.effect_columns);
            ui.label("Note cols");
            let a = ui.add(egui::DragValue::new(&mut nc).range(1..=12)).changed();
            ui.label("Fx cols");
            let b = ui.add(egui::DragValue::new(&mut ec).range(0..=8)).changed();
            if a || b {
                self.editor.edit_phrase("phrase_cols", |p| {
                    p.note_columns = nc;
                    p.effect_columns = ec;
                });
            }
            let mut looping = p.looping;
            if ui.checkbox(&mut looping, "Loop").changed() {
                self.editor.edit_phrase("phrase_loop", |p| p.looping = looping);
            }
            if p.looping {
                let (mut ls, mut le) = (p.loop_start, p.loop_end.max(p.loop_start + 1));
                let a = ui.add(egui::DragValue::new(&mut ls).range(0..=p.number_of_lines)).changed();
                ui.label("to");
                let b = ui.add(egui::DragValue::new(&mut le).range(0..=p.number_of_lines)).changed();
                if a || b {
                    self.editor.edit_phrase("phrase_loop_range", |p| {
                        p.loop_start = ls;
                        p.loop_end = le;
                    });
                }
            }
            let mut base = p.base_note as usize;
            ui.label("Base");
            if ui.add(egui::DragValue::new(&mut base).range(0..=119).custom_formatter(fmt)).changed() {
                self.editor.edit_phrase("phrase_base", |p| p.base_note = base as u8);
            }
            let mut keys = (p.key_range.0 as usize, p.key_range.1 as usize);
            ui.label("Keys");
            let a = ui.add(egui::DragValue::new(&mut keys.0).range(0..=119).custom_formatter(fmt)).changed();
            let b = ui.add(egui::DragValue::new(&mut keys.1).range(0..=119).custom_formatter(fmt)).changed();
            if a || b {
                self.editor.edit_phrase("phrase_keys", |p| p.key_range = (keys.0 as u8, keys.1 as u8));
            }
        });
        self.phrase_grid(ui);
    }

    fn phrase_grid(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;
        let font = FontId::monospace(self.editor.prefs.font_size.clamp(9.0, 32.0));
        let (cw, rh) = ui.fonts(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = resp.rect;
        painter.rect_filled(area, 0.0, pal.bg);
        let Some(p) = self.editor.current_phrase().cloned() else { return };
        let (nn, nf) = (p.note_columns as usize, p.effect_columns as usize);
        let cur = self.editor.phrase_cursor;
        let height = ((area.height() / rh) as usize).max(1);
        let top = cur.line as isize - (height / 2) as isize;
        let gutter = 3.0 * cw;
        let col_x = |c: usize| {
            area.left()
                + gutter
                + if c < nn { c * NOTE_COL_W } else { nn * NOTE_COL_W + (c - nn) * FX_COL_W } as f32 * cw
        };

        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(area.left(), area.top() + (height / 2) as f32 * rh),
                Vec2::new(area.width(), rh),
            ),
            0.0,
            pal.band,
        );
        let lpb = p.lpb.max(1) as isize;
        for r in 0..height {
            let li = top + r as isize;
            if !(0..p.number_of_lines as isize).contains(&li) {
                continue;
            }
            let y = area.top() + r as f32 * rh;
            let fg = if li % (lpb * 4) == 0 {
                pal.text_bar
            } else if li % lpb == 0 {
                pal.text_beat
            } else {
                pal.text
            };
            painter.text(
                Pos2::new(area.left() + 2.0, y),
                Align2::LEFT_TOP,
                format!("{li:02X}"),
                font.clone(),
                fg,
            );
            let line = p.line(li as u16);
            for c in 0..nn + nf {
                let text = cell_text(line, c, nn);
                let x = col_x(c);
                if li == cur.line as isize && c == cur.col {
                    let (a, b) = if c < nn {
                        NOTE_HL[cur.sub.min(NOTE_SUBS - 1)]
                    } else {
                        FX_HL[cur.sub.min(FX_SUBS - 1)]
                    };
                    let hl = Rect::from_min_size(
                        Pos2::new(x + a as f32 * cw, y),
                        Vec2::new((b - a) as f32 * cw, rh),
                    );
                    let color = if self.editor.edit_mode { pal.cursor_edit } else { pal.cursor_view };
                    painter.rect_filled(hl, 0.0, color);
                }
                painter.text(Pos2::new(x, y), Align2::LEFT_TOP, text, font.clone(), fg);
            }
        }

        if resp.clicked() || resp.drag_started() {
            self.phrase_focus = true;
            if let Some(pos) = resp.interact_pointer_pos() {
                let row = ((pos.y - area.top()) / rh).floor() as isize;
                let li = (top + row).clamp(0, p.number_of_lines as isize - 1) as u16;
                let mut best = (0, 0);
                for c in 0..nn + nf {
                    if pos.x >= col_x(c) {
                        let sub_count = if c < nn { NOTE_HL.len() } else { FX_HL.len() };
                        let rel = ((pos.x - col_x(c)) / cw) as usize;
                        let hl: &[(usize, usize)] = if c < nn { &NOTE_HL } else { &FX_HL };
                        let sub = (0..sub_count).find(|&s| rel < hl[s].1).unwrap_or(sub_count - 1);
                        best = (c, sub);
                    }
                }
                self.editor.phrase_set_cursor(best.0, best.1, li);
            }
        }
        if self.phrase_focus {
            let id = resp.id;
            ui.memory_mut(|m| {
                m.request_focus(id);
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }
    }
}
