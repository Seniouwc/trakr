//! Native UI for trakr (egui/eframe). A renderer over `trakr-editor`: it translates input into
//! `KeyEvent`/`Action` and draws; behavior lives in the editor.

mod devices_view;
mod dialogs;
mod envelopes;
mod fonts;
mod grid;
mod input;
mod menus;
mod midi_view;
mod mixer;
mod modulation_view;
mod panels;
mod phrases_view;
mod prefs_view;
mod sample_view;
mod scripts_view;
#[cfg(test)]
mod tests;
mod theme;
mod tools_view;

use dialogs::{Confirm, FileDialog};
use eframe::egui;
use trakr_editor::Editor;

/// Run the GUI until the window closes.
pub fn run(editor: Editor) -> anyhow::Result<()> {
    let size = editor.prefs.window_size.filter(|s| s[0] >= 640.0 && s[1] >= 400.0).unwrap_or([1280.0, 780.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(size).with_title("trakr"),
        ..Default::default()
    };
    eframe::run_native(
        "trakr",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(GuiApp::new(editor)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// What the bottom dock shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Instrument,
    Mixer,
    Devices,
    Automation,
    Phrases,
    Hidden,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SampleTab {
    Sample,
    Filter,
    Modulation,
    Env(envelopes::EnvKind),
}

pub(crate) struct GuiApp {
    pub editor: Editor,
    pub dialog: Option<FileDialog>,
    pub confirm: Option<Confirm>,
    pub allow_close: bool,
    pub allow_close_now: bool,
    pub show_help: bool,
    pub show_about: bool,
    /// Selected frame range in the sample view.
    pub sample_sel: Option<(usize, usize)>,
    pub sample_drag: Option<usize>,
    /// Visible frame window (start, length) in the waveform; `None` = whole sample.
    pub wave_view: Option<(usize, usize)>,
    pub wav_format: trakr_editor::WavFormat,
    pub gain_db: f32,
    pub pitch_st: f32,
    pub resample_hz: u32,
    pub slice_parts: u32,
    pub xfade_ms: f32,
    pub stretch: f32,
    pub slice_sens: f32,
    pub dither_bits: u32,
    pub show_spectrum: bool,
    pub dock: Dock,
    pub meter_hold: Vec<f32>,
    pub sample_tab: SampleTab,
    pub env_sel: Option<usize>,
    pub env_drag: Option<usize>,
    pub env_span: f32,
    pub auto_target: Option<(usize, trakr_core::AutoTarget)>,
    pub auto_drag: Option<usize>,
    pub title: String,
    /// A text field had keyboard focus at the end of the last frame.
    pub text_editing: bool,
    pub grid_info: GridInfo,
    checked_recovery: bool,
    pub palette: theme::Palette,
    /// Themes from `<config>/themes/*.json`: (id, name, scheme).
    pub user_themes: Vec<(String, String, theme::Scheme)>,
    applied_theme: String,
    applied_fonts: Option<(String, String)>,
    pub font_list: Vec<fonts::FontFile>,
    pub font_filter: String,
    pub font_message: String,
    applied_scale: f32,
    pub show_prefs: bool,
    pub prefs_tab: prefs_view::PrefsTab,
    pub capturing: Option<trakr_editor::Action>,
    pub binding_message: String,
    /// Keys go to the phrase editor instead of the pattern grid (set by clicking in it).
    pub phrase_focus: bool,
    pub show_console: bool,
    pub show_midi_map: bool,
    pub show_palette: bool,
    pub palette_query: String,
    pub show_find: bool,
    pub find: tools_view::FindState,
    pub script_text: String,
    pub script_name: String,
}

impl GuiApp {
    fn new(editor: Editor) -> Self {
        let editor_dock = editor.prefs.dock.clone();
        Self {
            editor,
            dialog: None,
            confirm: None,
            allow_close: false,
            allow_close_now: false,
            show_help: false,
            show_about: false,
            sample_sel: None,
            sample_drag: None,
            wave_view: None,
            wav_format: trakr_editor::WavFormat::Pcm16,
            gain_db: 0.0,
            pitch_st: 0.0,
            resample_hz: 44_100,
            slice_parts: 8,
            xfade_ms: 50.0,
            stretch: 1.0,
            slice_sens: 0.5,
            dither_bits: 16,
            show_spectrum: false,
            dock: match std::env::var("TRAKR_GUI_DOCK").as_deref().unwrap_or(editor_dock.as_str()) {
                "mixer" => Dock::Mixer,
                "devices" => Dock::Devices,
                "automation" => Dock::Automation,
                "phrases" => Dock::Phrases,
                "hidden" => Dock::Hidden,
                _ => Dock::Instrument,
            },
            meter_hold: Vec::new(),
            sample_tab: SampleTab::Sample,
            env_sel: None,
            env_drag: None,
            env_span: 2.0,
            auto_target: None,
            auto_drag: None,
            title: String::new(),
            text_editing: false,
            checked_recovery: false,
            phrase_focus: false,
            show_console: false,
            show_midi_map: false,
            show_palette: false,
            palette_query: String::new(),
            show_find: false,
            find: Default::default(),
            script_text: String::new(),
            script_name: String::new(),
            palette: theme::Palette::named("dark"),
            user_themes: Vec::new(),
            applied_theme: "dark".into(),
            applied_fonts: None,
            font_list: Vec::new(),
            font_filter: String::new(),
            font_message: String::new(),
            applied_scale: 1.0,
            show_prefs: false,
            prefs_tab: Default::default(),
            capturing: None,
            binding_message: String::new(),
            grid_info: GridInfo {
                area: egui::Rect::NOTHING,
                char_w: 8.0,
                row_h: 16.0,
                cursor_row_y: 0.0,
                cols: Vec::new(),
            },
        }
    }
}

impl Dock {
    fn name(self) -> &'static str {
        match self {
            Dock::Instrument => "instrument",
            Dock::Mixer => "mixer",
            Dock::Devices => "devices",
            Dock::Automation => "automation",
            Dock::Phrases => "phrases",
            Dock::Hidden => "hidden",
        }
    }
}

impl Drop for GuiApp {
    fn drop(&mut self) {
        self.editor.save_prefs();
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }
}

/// Where the pattern grid was drawn last frame (for tests and hit-testing helpers).
#[derive(Clone, Debug)]
pub struct GridInfo {
    pub area: egui::Rect,
    pub char_w: f32,
    pub row_h: f32,
    /// Top of the row the cursor sits on.
    pub cursor_row_y: f32,
    /// (track, column within track, left x, width in characters) of every drawn column.
    pub cols: Vec<(usize, usize, f32, usize)>,
}

impl GuiApp {
    /// One UI frame. Kept separate from `eframe::App::update` so tests can drive it with synthetic input.
    pub(crate) fn frame(&mut self, ctx: &egui::Context) {
        // Keys go to the editor unless a text field is being edited (egui reports focus on the pattern
        // grid itself as "wants keyboard input", so that flag cannot be used).
        self.apply_appearance(ctx);
        self.capture_binding(ctx);
        if !self.text_editing && self.dialog.is_none() && self.confirm.is_none() && self.capturing.is_none() {
            let keys = input::collect_keys(ctx);
            if !keys.is_empty() {
                // Keep arrows and Tab from moving egui's widget focus: the grid takes focus back.
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
            }
            for k in keys {
                if self.dock == Dock::Phrases && self.phrase_focus {
                    self.editor.phrase_key(k);
                } else {
                    self.editor.on_key(k);
                }
            }
        }
        if !self.checked_recovery {
            self.checked_recovery = true;
            if self.editor.recovery_available() {
                self.confirm = Some(Confirm::Recover);
            }
        }
        self.editor.prefs.dock = self.dock.name().to_string();
        let size = ctx.screen_rect().size();
        self.editor.prefs.window_size = Some([size.x, size.y]);
        self.editor.tick();
        ctx.request_repaint_after(std::time::Duration::from_millis(33));

        if self.editor.quit {
            self.editor.quit = false;
            self.request_close(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.editor.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit);
        }

        let title = format!("trakr - {}{}", self.editor.song.name, if self.editor.dirty { " *" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        self.menu_bar(ctx);
        self.transport_bar(ctx);
        self.status_bar(ctx);
        self.dock_panel(ctx);
        self.side_panel(ctx);
        egui::CentralPanel::default().show(ctx, |ui| self.pattern_view(ui));
        self.show_dialogs(ctx);
        self.prefs_window(ctx);
        self.console_window(ctx);
        self.tool_windows(ctx);
        self.midi_window(ctx);
        self.text_editing = ctx.output(|o| o.ime.is_some());
    }
}

impl GuiApp {
    pub fn request_close(&mut self, ctx: &egui::Context) {
        if self.editor.dirty {
            self.confirm = Some(Confirm::Quit);
        } else {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl GuiApp {
    /// Bottom dock with tabs (tabs sit above the dock), so the pattern editor keeps most of the window.
    fn dock_panel(&mut self, ctx: &egui::Context) {
        if self.dock != Dock::Hidden {
            egui::TopBottomPanel::bottom("dock")
                .resizable(true)
                .default_height(240.0)
                .height_range(120.0..=600.0)
                .show(ctx, |ui| match self.dock {
                    Dock::Instrument => self.sample_ui(ui),
                    Dock::Mixer => self.mixer_ui(ui),
                    Dock::Devices => self.devices_ui(ui),
                    Dock::Automation => self.automation_ui(ui),
                    Dock::Phrases => self.phrases_ui(ui),
                    Dock::Hidden => {}
                });
        }
        egui::TopBottomPanel::bottom("dock_tabs").resizable(false).exact_height(26.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.dock, Dock::Instrument, "Instrument / sample");
                ui.selectable_value(&mut self.dock, Dock::Mixer, "Mixer");
                ui.selectable_value(&mut self.dock, Dock::Devices, "Devices");
                ui.selectable_value(&mut self.dock, Dock::Automation, "Automation");
                ui.selectable_value(&mut self.dock, Dock::Phrases, "Phrases");
                ui.selectable_value(&mut self.dock, Dock::Hidden, "Hide");
            });
        });
    }
}
