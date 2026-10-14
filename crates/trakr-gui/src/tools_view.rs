//! Command palette and find/replace windows.

use crate::GuiApp;
use eframe::egui;
use trakr_core::note::parse_note;
use trakr_editor::{Action, CellSpec, Scope};

/// Parse the three find/replace text fields into a spec. Empty fields mean "any" / "leave alone".
fn spec(note: &str, inst: &str, fx: &str) -> Result<CellSpec, String> {
    let mut s = CellSpec::default();
    if !note.trim().is_empty() {
        s.note = Some(parse_note(note).ok_or_else(|| format!("bad note {note:?} (try C-4, F#3, OFF)"))?);
    }
    if !inst.trim().is_empty() {
        s.instrument =
            Some(u8::from_str_radix(inst.trim(), 16).map_err(|_| format!("bad instrument {inst:?} (hex)"))?);
    }
    let fx = fx.trim().to_ascii_uppercase();
    if !fx.is_empty() {
        let b = fx.as_bytes();
        if b.len() != 2 || !b.iter().all(u8::is_ascii_alphanumeric) {
            return Err(format!("bad effect {fx:?} (two characters like 0A)"));
        }
        s.effect = Some([b[0], b[1]]);
    }
    Ok(s)
}

#[derive(Default)]
pub struct FindState {
    pub find: [String; 3],
    pub with: [String; 3],
    pub message: String,
}

impl GuiApp {
    pub fn tool_windows(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.editor.palette_requested) {
            self.show_palette = true;
            self.palette_query.clear();
        }
        if std::mem::take(&mut self.editor.find_requested) {
            self.show_find = true;
        }
        self.palette_window(ctx);
        self.find_window(ctx);
    }

    fn palette_window(&mut self, ctx: &egui::Context) {
        if !self.show_palette {
            return;
        }
        let q = self.palette_query.to_lowercase();
        let mut matches: Vec<Action> = Action::ALL
            .iter()
            .copied()
            .filter(|a| !matches!(a, Action::OpenPalette))
            .filter(|a| q.split_whitespace().all(|w| a.label().to_lowercase().contains(w)))
            .collect();
        matches.truncate(14);
        let mut run: Option<Action> = None;
        let mut close = false;
        egui::Window::new("Command palette")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text("Type a command...")
                        .desired_width(360.0),
                );
                r.request_focus();
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = matches.first().copied();
                }
                for a in &matches {
                    let text = if a.shortcut().is_empty() {
                        a.label().to_string()
                    } else {
                        format!("{}    ({})", a.label(), self.editor.shortcut_for(*a))
                    };
                    if ui.selectable_label(false, text).clicked() {
                        run = Some(*a);
                    }
                }
                if matches.is_empty() {
                    ui.weak("No command matches");
                }
            });
        if let Some(a) = run {
            self.show_palette = false;
            self.editor.do_action(a);
        } else if close {
            self.show_palette = false;
        }
    }

    fn find_window(&mut self, ctx: &egui::Context) {
        if !self.show_find {
            return;
        }
        let mut open = true;
        egui::Window::new("Find and replace").open(&mut open).resizable(false).show(ctx, |ui| {
            egui::Grid::new("find_grid").num_columns(4).show(ui, |ui| {
                ui.label("");
                ui.label("Note");
                ui.label("Instrument (hex)");
                ui.label("Effect");
                ui.end_row();
                ui.label("Find");
                for f in &mut self.find.find {
                    ui.add(egui::TextEdit::singleline(f).desired_width(90.0));
                }
                ui.end_row();
                ui.label("Replace with");
                for f in &mut self.find.with {
                    ui.add(egui::TextEdit::singleline(f).desired_width(90.0));
                }
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button("Find next").clicked() {
                    match spec(&self.find.find[0], &self.find.find[1], &self.find.find[2]) {
                        Ok(s) => {
                            self.find.message.clear();
                            self.editor.find_next(s);
                        }
                        Err(e) => self.find.message = e,
                    }
                }
                for (label, scope) in [
                    ("Replace in selection", Scope::Selection),
                    ("Replace in pattern", Scope::Pattern),
                    ("Replace in song", Scope::Song),
                ] {
                    if ui.button(label).clicked() {
                        let f = spec(&self.find.find[0], &self.find.find[1], &self.find.find[2]);
                        let w = spec(&self.find.with[0], &self.find.with[1], &self.find.with[2]);
                        match (f, w) {
                            (Ok(f), Ok(w)) => {
                                self.find.message.clear();
                                self.editor.replace_all(f, w, scope);
                            }
                            (Err(e), _) | (_, Err(e)) => self.find.message = e,
                        }
                    }
                }
            });
            if !self.find.message.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.find.message);
            }
            ui.weak("Leave a Find field empty to match anything. Empty Replace fields stay unchanged.");
        });
        self.show_find = open;
    }
}
