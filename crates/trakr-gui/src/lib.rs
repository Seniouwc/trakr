//! Native UI for trakr (egui/eframe). A renderer over `trakr-editor`: it translates input into
//! `KeyEvent`/`Action` and draws; behavior lives in the editor.

mod devices_view;
mod dialogs;
mod envelopes;
mod grid;
mod input;
mod menus;
mod mixer;
mod panels;
mod sample_view;

use dialogs::{Confirm, FileDialog};
use eframe::egui;
use trakr_editor::Editor;

/// Run the GUI until the window closes.
pub fn run(editor: Editor) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 780.0]).with_title("trakr"),
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
    Hidden,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SampleTab {
    Sample,
    Filter,
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
    pub dock: Dock,
    pub meter_hold: Vec<f32>,
    pub sample_tab: SampleTab,
    pub env_sel: Option<usize>,
    pub env_drag: Option<usize>,
    pub env_span: f32,
    pub auto_target: Option<(usize, trakr_core::AutoTarget)>,
    pub auto_drag: Option<usize>,
    pub title: String,
}

impl GuiApp {
    fn new(editor: Editor) -> Self {
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
            dock: match std::env::var("TRAKR_GUI_DOCK").as_deref() {
                Ok("mixer") => Dock::Mixer,
                Ok("devices") => Dock::Devices,
                Ok("automation") => Dock::Automation,
                Ok("hidden") => Dock::Hidden,
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
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !ctx.wants_keyboard_input() && self.dialog.is_none() && self.confirm.is_none() {
            for k in input::collect_keys(ctx) {
                self.editor.on_key(k);
            }
        }
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
                    Dock::Hidden => {}
                });
        }
        egui::TopBottomPanel::bottom("dock_tabs").resizable(false).exact_height(26.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.dock, Dock::Instrument, "Instrument / sample");
                ui.selectable_value(&mut self.dock, Dock::Mixer, "Mixer");
                ui.selectable_value(&mut self.dock, Dock::Devices, "Devices");
                ui.selectable_value(&mut self.dock, Dock::Automation, "Automation");
                ui.selectable_value(&mut self.dock, Dock::Hidden, "Hide");
            });
        });
    }
}
