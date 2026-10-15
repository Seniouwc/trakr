//! In-app file browser and confirmation prompts (no system dialog dependencies).

use crate::GuiApp;
use eframe::egui;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    Open,
    SaveAs,
    ImportSample,
    AddSample,
    ExportSample,
    ExportSong,
    ExportXrns,
    ExportStems,
    ExportRange,
}

pub struct FileDialog {
    pub mode: FileMode,
    dir: PathBuf,
    name: String,
    entries: Vec<(String, bool)>,
    error: String,
    /// Extension of the export format currently chosen (filters the list).
    export_ext: String,
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
            FileMode::ExportSample => "sample.wav".into(),
            FileMode::ExportSong => "song.wav".into(),
            FileMode::ExportXrns => "song.xrns".into(),
            FileMode::ExportStems => "stems.wav".into(),
            FileMode::ExportRange => "range.wav".into(),
            _ => String::new(),
        };
        let mut d =
            Self { mode, dir, name, entries: vec![], error: String::new(), export_ext: String::new() };
        d.refresh();
        d
    }

    fn accepts(&self, file: &str) -> bool {
        let ext = Path::new(file).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        let ext = ext.as_deref().unwrap_or("");
        match self.mode {
            FileMode::Open | FileMode::SaveAs => matches!(ext, "trkr" | "xrns" | "mod" | "s3m" | "xm"),
            FileMode::ImportSample | FileMode::AddSample => {
                matches!(ext, "wav" | "flac" | "ogg" | "mp3" | "aif" | "aiff")
            }
            FileMode::ExportSample | FileMode::ExportSong | FileMode::ExportStems | FileMode::ExportRange => {
                ext == self.export_ext
            }
            FileMode::ExportXrns => ext == "xrns",
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
    /// An autosave from a crashed or unsaved session was found.
    Recover,
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
            Confirm::Recover => {}
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
            FileMode::Open => "Open song (.trkr, .xrns, .mod, .s3m, .xm)",
            FileMode::SaveAs => "Save song as",
            FileMode::ImportSample => "Import sample as new instrument (wav, flac, ogg, mp3, aiff)",
            FileMode::AddSample => "Add sample file to the selected instrument",
            FileMode::ExportSample => "Export sample",
            FileMode::ExportSong => "Export song",
            FileMode::ExportXrns => "Export song as .xrns",
            FileMode::ExportStems => "Export each track as its own file (name is the file prefix)",
            FileMode::ExportRange => "Export the loop range (or whole song)",
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
            if matches!(
                d.mode,
                FileMode::ExportSample | FileMode::ExportSong | FileMode::ExportStems | FileMode::ExportRange
            ) {
                let mut fmt = self.editor.prefs.export;
                if export_format_ui(ui, &mut fmt, d.mode != FileMode::ExportSample) {
                    self.editor.prefs.export = fmt;
                    self.editor.save_prefs();
                }
                let ext = fmt.extension();
                if d.export_ext != ext {
                    d.export_ext = ext.to_string();
                    let stem = Path::new(&d.name)
                        .file_stem()
                        .map_or("export".to_string(), |s| s.to_string_lossy().into());
                    d.name = format!("{stem}.{ext}");
                    d.refresh();
                }
                ui.separator();
            }
            ui.horizontal(|ui| {
                ui.label("File:");
                ui.text_edit_singleline(&mut d.name);
            });
            if !d.error.is_empty() {
                ui.colored_label(egui::Color32::LIGHT_RED, &d.error);
            }
            ui.horizontal(|ui| {
                let verb = match d.mode {
                    FileMode::SaveAs
                    | FileMode::ExportSample
                    | FileMode::ExportSong
                    | FileMode::ExportXrns
                    | FileMode::ExportStems
                    | FileMode::ExportRange => "Save",
                    _ => "Open",
                };
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
                FileMode::ImportSample | FileMode::AddSample => {
                    if path.is_file() {
                        if d.mode == FileMode::AddSample {
                            self.editor.import_sample_into_instrument(&path);
                        } else {
                            self.editor.import_sample(&path);
                        }
                        self.sample_sel = None;
                        self.wave_view = None;
                        keep = false;
                    } else {
                        d.error = "file does not exist".into();
                    }
                }
                FileMode::ExportStems | FileMode::ExportRange => {
                    let fmt = self.editor.prefs.export;
                    path.set_extension(fmt.extension());
                    if d.mode == FileMode::ExportStems {
                        self.editor.export_stems(&path, fmt);
                    } else {
                        let last = self.editor.song.sequence.len().saturating_sub(1);
                        let (a, b) = self.editor.loop_range().unwrap_or((0, last));
                        self.editor.export_slots(&path, a, b, fmt);
                    }
                    keep = false;
                }
                FileMode::ExportXrns => {
                    if path.extension().is_none() {
                        path.set_extension("xrns");
                    }
                    self.editor.export_xrns(&path);
                    keep = false;
                }
                FileMode::ExportSample | FileMode::ExportSong => {
                    let fmt = self.editor.prefs.export;
                    path.set_extension(fmt.extension());
                    if d.mode == FileMode::ExportSong {
                        self.editor.export_song(&path, fmt);
                    } else {
                        let (a, b) = self.sample_sel.unwrap_or((0, 0));
                        self.editor.export_sample(&path, a, b, fmt);
                    }
                    keep = false;
                }
            }
        }
        if keep {
            self.dialog = Some(d);
        }
    }

    fn confirm_ui(&mut self, ctx: &egui::Context) {
        let Some(c) = self.confirm.take() else { return };
        if matches!(c, Confirm::Recover) {
            let mut choice: Option<bool> = None;
            egui::Window::new("Recover unsaved work?").collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("trakr found an autosave newer than the saved song (the last session may have ended unexpectedly).");
                ui.horizontal(|ui| {
                    if ui.button("Recover").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => self.editor.recover_autosave(),
                Some(false) => self.editor.discard_autosave(),
                None => self.confirm = Some(c),
            }
            return;
        }
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
                ui.heading("Effect commands");
                ui.label("Type the two-character id and a hex amount in an effect column, or in the small slot beside a note. Slides, fades and LFOs start on the line they appear on.");
                egui::Grid::new("fx").striped(true).show(ui, |ui| {
                    for (id, name, desc) in trakr_core::effects::EFFECTS {
                        ui.monospace(*id);
                        ui.label(*name);
                        ui.label(*desc);
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.heading("Fading notes");
                ui.label("Select lines and use Block > Fade in/out to ramp the volume column, or put 0I (fade in) / 0O (fade out) on the note's line. Volume values on empty lines change the sounding note.");
                ui.separator();
                ui.heading("Shortcuts");
                egui::Grid::new("keys").striped(true).show(ui, |ui| {
                    for a in menus::ALL_ACTIONS {
                        let key = self.editor.shortcut_for(*a);
                        if !key.is_empty() {
                            ui.label(a.label());
                            ui.monospace(key);
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

/// Format, quality, bit depth, dither and sample rate controls for audio export. Returns true on change.
fn export_format_ui(
    ui: &mut egui::Ui,
    fmt: &mut trakr_editor::ExportFormat,
    can_resample_render: bool,
) -> bool {
    use trakr_editor::Codec;
    let before = *fmt;
    ui.horizontal(|ui| {
        ui.label("Format:");
        egui::ComboBox::from_id_salt("export_codec").selected_text(fmt.codec.name()).show_ui(ui, |ui| {
            for c in Codec::ALL {
                ui.selectable_value(&mut fmt.codec, c, c.name());
            }
        });
        if fmt.codec.uses_bit_depth() {
            let max = if fmt.codec == Codec::Flac { 24 } else { 32 };
            fmt.bits = fmt.bits.min(max);
            egui::ComboBox::from_id_salt("export_bits")
                .selected_text(if fmt.bits == 32 {
                    "32-bit float".to_string()
                } else {
                    format!("{}-bit", fmt.bits)
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut fmt.bits, 16, "16-bit");
                    ui.selectable_value(&mut fmt.bits, 24, "24-bit");
                    if fmt.codec == Codec::Wav {
                        ui.selectable_value(&mut fmt.bits, 32, "32-bit float");
                    }
                });
            if fmt.bits < 32 {
                ui.checkbox(&mut fmt.dither, "Dither")
                    .on_hover_text("Triangular dither when reducing the bit depth");
            }
        }
    });
    if fmt.codec.uses_quality() {
        ui.horizontal(|ui| {
            ui.label("Quality:");
            let mut q = fmt.quality as i32;
            ui.add(
                egui::Slider::new(&mut q, 0..=trakr_editor::MAX_QUALITY as i32)
                    .text("0 = smallest, 10 = best"),
            );
            fmt.quality = q as u8;
        });
    }
    ui.horizontal(|ui| {
        ui.label("Sample rate:");
        let label = fmt.sample_rate.map_or("Same as source".to_string(), |r| format!("{r} Hz"));
        egui::ComboBox::from_id_salt("export_rate").selected_text(label).show_ui(ui, |ui| {
            ui.selectable_value(&mut fmt.sample_rate, None, "Same as source");
            for r in [22_050u32, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000] {
                ui.selectable_value(&mut fmt.sample_rate, Some(r), format!("{r} Hz"));
            }
        });
        if can_resample_render {
            ui.weak("songs are rendered at this rate directly");
        }
    });
    ui.weak(fmt.describe());
    *fmt != before
}
