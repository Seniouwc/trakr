//! In-app file browser and confirmation prompts (no system dialog dependencies).

use crate::GuiApp;
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Open,
    SaveAs,
    ImportSample,
}

pub struct FileDialog {
    pub mode: FileMode,
    dir: PathBuf,
    name: String,
    entries: Vec<(String, bool)>,
    error: String,
}

impl FileDialog {
    pub fn new(mode: FileMode, start: Option<&Path>) -> Self {
        let dir = start
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .filter(|p| p.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let name = match mode {
            FileMode::SaveAs => {
                start.and_then(|p| p.file_name()).map_or("song.trkr".into(), |n| n.to_string_lossy().into())
            }
            _ => String::new(),
        };
        let mut d = Self { mode, dir, name, entries: vec![], error: String::new() };
        d.refresh();
        d
    }

    fn accepts(&self, file: &str) -> bool {
        let ext = Path::new(file).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let ext = ext.as_deref().unwrap_or("");
        match self.mode {
            FileMode::Open | FileMode::SaveAs => matches!(ext, "trkr" | "xrns"),
            FileMode::ImportSample => matches!(ext, "wav" | "flac" | "ogg" | "mp3" | "aif" | "aiff"),
        }
    }

    fn refresh(&mut self) {
        let mut v: Vec<(String, bool)> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let is_dir = e.path().is_dir();
                        (!name.starts_with('.') && (is_dir || self.accepts(&name))).then_some((name, is_dir))
                    })
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
        self.entries = v;
    }

    fn enter(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.error.clear();
        self.refresh();
    }
}

pub enum Confirm {
    Quit,
    New,
    Open,
}

impl GuiApp {
    pub fn open_dialog(&mut self, mode: FileMode) {
        let start = self.editor.path.clone();
        self.dialog = Some(FileDialog::new(mode, start.as_deref()));
    }

    /// Run `then` now, or after asking about unsaved changes.
    pub fn guard_unsaved(&mut self, c: Confirm) {
        if self.editor.dirty {
            self.confirm = Some(c);
        } else {
            self.proceed(c);
        }
    }

    fn proceed(&mut self, c: Confirm) {
        match c {
            Confirm::Quit => self.allow_close_now = true,
            Confirm::New => self.editor.new_song(),
            Confirm::Open => self.open_dialog(FileMode::Open),
        }
    }

    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        self.file_dialog_ui(ctx);
        self.confirm_ui(ctx);
        self.help_ui(ctx);
        if self.allow_close_now {
            self.allow_close_now = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn file_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.dialog.take() else { return };
        let title = match d.mode {
            FileMode::Open => "Open song (.trkr, .xrns)",
            FileMode::SaveAs => "Save song as",
            FileMode::ImportSample => "Import sample (wav, flac, ogg, mp3, aiff)",
        };
        let mut keep = true;
        let mut chosen: Option<PathBuf> = None;
        egui::Window::new(title).collapsible(false).default_width(480.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Up").clicked() {
                    if let Some(p) = d.dir.parent() {
                        let p = p.to_path_buf();
                        d.enter(p);
                    }
                }
                ui.monospace(d.dir.display().to_string());
            });
            ui.separator();
            let mut goto: Option<PathBuf> = None;
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                for (name, is_dir) in d.entries.clone() {
                    let label = if is_dir { format!("[{name}]") } else { name.clone() };
                    let r = ui.selectable_label(d.name == name, label);
                    if r.clicked() {
                        if is_dir {
                            goto = Some(d.dir.join(&name));
                        } else {
                            d.name = name.clone();
                        }
                    }
                    if r.double_clicked() && !is_dir {
                        chosen = Some(d.dir.join(&name));
                    }
                }
            });
            if let Some(p) = goto {
                d.enter(p);
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("File:");
                ui.text_edit_singleline(&mut d.name);
            });
            if !d.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
            }
            ui.horizontal(|ui| {
                let verb = if d.mode == FileMode::SaveAs { "Save" } else { "Open" };
                if ui.button(verb).clicked() && !d.name.is_empty() {
                    chosen = Some(d.dir.join(&d.name));
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        if let Some(mut path) = chosen {
            match d.mode {
                FileMode::Open => {
                    if path.is_file() {
                        self.editor.open(&path);
                        keep = false;
                    } else {
                        d.error = "file does not exist".into();
                    }
                }
                FileMode::SaveAs => {
                    if path.extension().is_none() {
                        path.set_extension("trkr");
                    }
                    self.editor.save_as(path);
                    keep = false;
                }
                FileMode::ImportSample => {
                    if path.is_file() {
                        self.editor.import_sample(&path);
                        keep = false;
                    } else {
                        d.error = "file does not exist".into();
                    }
                }
            }
        }
        if keep {
            self.dialog = Some(d);
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm.take() else { return };
        let mut action: Option<&str> = None;
        egui::Window::new("Unsaved changes").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("This song has unsaved changes.");
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    action = Some("save");
                }
                if ui.button("Discard").clicked() {
                    action = Some("discard");
                }
                if ui.button("Cancel").clicked() {
                    action = Some("cancel");
                }
            });
        });
        match action {
            Some("save") => {
                if self.editor.path.is_some() {
                    self.editor.save();
                    self.proceed(c);
                } else {
                    self.open_dialog(FileMode::SaveAs);
                }
            }
            Some("discard") => {
                self.editor.dirty = false;
                self.proceed(c);
            }
            Some(_) => {}
            None => self.confirm = Some(c),
        }
    }

    fn help_ui(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        egui::Window::new("Keyboard reference").open(&mut open).default_width(420.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                ui.heading("Entering notes");
                ui.label("Turn on edit mode with Space. The keys Z S X D C V G B H N J M play the lower octave, Q 2 W 3 E R 5 T 6 Y 7 U the upper octave. ` writes a note-off. In the volume, pan, delay and effect fields, type hex digits.");
                ui.label("Click any cell to place the cursor; drag to select a block, or hold Shift with the arrow keys.");
                ui.separator();
                ui.heading("Shortcuts");
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for a in menus::ALL_ACTIONS {
                        if !a.shortcut().is_empty() {
                            ui.label(a.label());
                            ui.monospace(a.shortcut());
                            ui.end_row();
                        }
                    }
                });
            });
        });
        self.show_help = open;
        let mut about = self.show_about;
        egui::Window::new("About trakr").open(&mut about).collapsible(false).show(ctx, |ui| {
            ui.label("trakr: an open source tracker for Linux, in Rust.");
            ui.label("MIT licensed. Independent of Renoise.");
        });
        self.show_about = about;
    }
}

use crate::menus;
