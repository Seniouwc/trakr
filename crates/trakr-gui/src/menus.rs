use crate::dialogs::{Confirm, FileMode};
use crate::GuiApp;
use eframe::egui::{self, Button, Ui};
use trakr_editor::Action;

use Action::*;

pub const ALL_ACTIONS: &[Action] = &[
    New,
    Save,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Clear,
    SelectAll,
    SelectNone,
    ToggleEdit,
    TransposeUp,
    TransposeDown,
    TransposeOctaveUp,
    TransposeOctaveDown,
    Reverse,
    Interpolate,
    InsertLine,
    DeleteLine,
    PlaySong,
    PlayPattern,
    PlayFromCursor,
    Stop,
    ToggleFollow,
    SeqPrev,
    SeqNext,
    SeqInsert,
    SeqClone,
    SeqDelete,
    SeqMoveUp,
    SeqMoveDown,
    PatternPrev,
    PatternNext,
    TrackAdd,
    TrackRemove,
    TrackMute,
    NoteColumnAdd,
    NoteColumnRemove,
    EffectColumnAdd,
    EffectColumnRemove,
    OctaveDown,
    OctaveUp,
    StepDown,
    StepUp,
    InstrumentPrev,
    InstrumentNext,
    InstrumentRemove,
    RecordToggle,
    RenderToSample,
    FadeInBlock,
    FadeOutBlock,
    TrackSolo,
    LoopStart,
    LoopEnd,
    LoopClear,
];

impl GuiApp {
    /// Menu entry for an action: label, shortcut hint, runs on click.
    fn item(&mut self, ui: &mut Ui, a: Action) {
        let enabled = match a {
            Undo => self.editor.can_undo(),
            Redo => self.editor.can_redo(),
            _ => true,
        };
        if ui.add_enabled(enabled, Button::new(a.label()).shortcut_text(a.shortcut())).clicked() {
            self.run_action(a);
            ui.close_menu();
        }
    }

    pub fn run_action(&mut self, a: Action) {
        match a {
            New => self.guard_unsaved(Confirm::New),
            other => self.editor.do_action(other),
        }
    }

    pub fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    self.item(ui, New);
                    if ui.button("Open...").clicked() {
                        self.guard_unsaved(Confirm::Open);
                        ui.close_menu();
                    }
                    self.item(ui, Save);
                    if ui.button("Save as...").clicked() {
                        self.open_dialog(FileMode::SaveAs);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Import sample...").clicked() {
                        self.open_dialog(FileMode::ImportSample);
                        ui.close_menu();
                    }
                    if ui.button("Export song as WAV...").clicked() {
                        self.open_dialog(FileMode::ExportSong);
                        ui.close_menu();
                    }
                    ui.separator();
                    self.item(ui, Quit);
                });
                ui.menu_button("Edit", |ui| {
                    for a in [Undo, Redo] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [Cut, Copy, Paste, Clear] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [SelectAll, SelectNone, ToggleEdit] {
                        self.item(ui, a);
                    }
                });
                ui.menu_button("Block", |ui| {
                    for a in [TransposeUp, TransposeDown, TransposeOctaveUp, TransposeOctaveDown] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [Reverse, Interpolate, FadeInBlock, FadeOutBlock] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [InsertLine, DeleteLine] {
                        self.item(ui, a);
                    }
                });
                ui.menu_button("Pattern", |ui| {
                    for a in [SeqInsert, SeqClone, SeqDelete, SeqMoveUp, SeqMoveDown] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [PatternPrev, PatternNext, SeqPrev, SeqNext] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [LoopStart, LoopEnd, LoopClear] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    ui.menu_button("Set length", |ui| {
                        for n in [16u16, 32, 48, 64, 96, 128, 192, 256, 512] {
                            if ui.button(format!("{n} lines")).clicked() {
                                self.editor.set_pattern_lines(n);
                                ui.close_menu();
                            }
                        }
                    });
                });
                ui.menu_button("Track", |ui| {
                    for a in [TrackAdd, TrackRemove, TrackMute, TrackSolo] {
                        self.item(ui, a);
                    }
                    ui.separator();
                    for a in [NoteColumnAdd, NoteColumnRemove, EffectColumnAdd, EffectColumnRemove] {
                        self.item(ui, a);
                    }
                });
                ui.menu_button("Instrument", |ui| {
                    if ui.button("Import sample as new instrument...").clicked() {
                        self.open_dialog(FileMode::ImportSample);
                        ui.close_menu();
                    }
                    if ui.button("Add sample file to this instrument...").clicked() {
                        self.open_dialog(FileMode::AddSample);
                        ui.close_menu();
                    }
                    self.item(ui, RecordToggle);
                    self.item(ui, RenderToSample);
                    ui.separator();
                    if ui.button("New empty instrument").clicked() {
                        self.editor.instrument_add_empty();
                        ui.close_menu();
                    }
                    for a in [InstrumentRemove, InstrumentPrev, InstrumentNext] {
                        self.item(ui, a);
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.radio_value(&mut self.dock, crate::Dock::Instrument, "Instrument / sample editor");
                    ui.radio_value(&mut self.dock, crate::Dock::Mixer, "Mixer");
                    ui.radio_value(&mut self.dock, crate::Dock::Hidden, "Hide bottom panel");
                });
                ui.menu_button("Play", |ui| {
                    for a in [PlaySong, PlayPattern, PlayFromCursor, Stop, ToggleFollow] {
                        self.item(ui, a);
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Keyboard reference").clicked() {
                        self.show_help = true;
                        ui.close_menu();
                    }
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    pub fn transport_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("transport").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, a, tip) in [
                    ("Play", PlaySong, "Play song from the start (F5)"),
                    ("Loop", PlayPattern, "Loop the current pattern (F6)"),
                    ("From cursor", PlayFromCursor, "Play from the cursor line (F7)"),
                    ("Stop", Stop, "Stop (F8)"),
                ] {
                    if ui.button(label).on_hover_text(tip).clicked() {
                        self.run_action(a);
                    }
                }
                ui.separator();
                let ed = &mut self.editor;
                ui.checkbox(&mut ed.edit_mode, "Edit").on_hover_text("Enter notes and values (Space)");
                ui.checkbox(&mut ed.follow, "Follow").on_hover_text("Cursor follows the playhead (F11)");
                ui.separator();
                let (mut bpm, mut lpb, mut tpl) = (ed.song.bpm, ed.song.lpb as u32, ed.song.tpl as u32);
                let mut changed = false;
                ui.label("BPM");
                changed |= ui.add(egui::DragValue::new(&mut bpm).range(32.0..=999.0).speed(0.5)).changed();
                ui.label("LPB");
                changed |= ui.add(egui::DragValue::new(&mut lpb).range(1..=32)).changed();
                ui.label("TPL");
                changed |= ui.add(egui::DragValue::new(&mut tpl).range(1..=16)).changed();
                if changed {
                    ed.set_tempo(bpm, lpb as u8, tpl as u8);
                }
                ui.separator();
                let mut oct = ed.octave as u32;
                ui.label("Octave");
                if ui.add(egui::DragValue::new(&mut oct).range(0..=8)).changed() {
                    ed.octave = oct as u8;
                }
                let mut step = ed.edit_step as u32;
                ui.label("Step");
                if ui.add(egui::DragValue::new(&mut step).range(0..=64)).changed() {
                    ed.edit_step = step as u16;
                }
                ui.separator();
                let recording = ed.is_recording();
                let rec = egui::Button::new(
                    egui::RichText::new(if recording { "Stop rec" } else { "Rec" }).color(if recording {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::LIGHT_RED
                    }),
                )
                .fill(if recording {
                    egui::Color32::from_rgb(170, 30, 30)
                } else {
                    egui::Color32::TRANSPARENT
                });
                let tip = if ed.can_record() {
                    "Record a sample from the audio input (Ctrl-R)"
                } else {
                    "No audio input available"
                };
                if ui.add_enabled(ed.can_record(), rec).on_hover_text(tip).clicked() {
                    ed.record_toggle();
                }
                if recording {
                    ui.add(egui::ProgressBar::new(ed.rec_peak.min(1.0)).desired_width(70.0));
                    ui.monospace(format!("{:.1}s", ed.record_seconds()));
                }
                let (playing, seq, line) = ed.playhead();
                ui.separator();
                ui.monospace(if playing {
                    format!("PLAY {seq:02X}:{line:02X}")
                } else {
                    "STOP      ".into()
                });
            });
        });
    }

    pub fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let ed = &self.editor;
                ui.monospace(format!(
                    "{}  |  track {} col {} line {:02X}  |  {}",
                    if ed.edit_mode { "EDIT" } else { "----" },
                    ed.cursor.track + 1,
                    ed.cursor.col + 1,
                    ed.cursor.line,
                    ed.status
                ));
            });
        });
    }
}
