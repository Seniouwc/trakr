//! Preferences window: key bindings, appearance, autosave.

use crate::theme::{Palette, THEMES};
use crate::GuiApp;
use eframe::egui::{self, Key};
use trakr_editor::{Action, KeyCode, KeyEvent};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PrefsTab {
    #[default]
    Keys,
    Appearance,
    Audio,
    General,
}

/// Map an egui key to the editor's key code (letters become lowercase characters).
pub fn egui_to_keycode(k: Key) -> Option<KeyCode> {
    use Key::*;
    Some(match k {
        ArrowUp => KeyCode::Up,
        ArrowDown => KeyCode::Down,
        ArrowLeft => KeyCode::Left,
        ArrowRight => KeyCode::Right,
        PageUp => KeyCode::PageUp,
        PageDown => KeyCode::PageDown,
        Home => KeyCode::Home,
        End => KeyCode::End,
        Tab => KeyCode::Tab,
        Backspace => KeyCode::Backspace,
        Delete => KeyCode::Delete,
        Insert => KeyCode::Insert,
        Escape => KeyCode::Esc,
        Space => KeyCode::Char(' '),
        F1 => KeyCode::F(1),
        F2 => KeyCode::F(2),
        F3 => KeyCode::F(3),
        F4 => KeyCode::F(4),
        F5 => KeyCode::F(5),
        F6 => KeyCode::F(6),
        F7 => KeyCode::F(7),
        F8 => KeyCode::F(8),
        F9 => KeyCode::F(9),
        F10 => KeyCode::F(10),
        F11 => KeyCode::F(11),
        F12 => KeyCode::F(12),
        A => KeyCode::Char('a'),
        B => KeyCode::Char('b'),
        C => KeyCode::Char('c'),
        D => KeyCode::Char('d'),
        E => KeyCode::Char('e'),
        F => KeyCode::Char('f'),
        G => KeyCode::Char('g'),
        H => KeyCode::Char('h'),
        I => KeyCode::Char('i'),
        J => KeyCode::Char('j'),
        K => KeyCode::Char('k'),
        L => KeyCode::Char('l'),
        M => KeyCode::Char('m'),
        N => KeyCode::Char('n'),
        O => KeyCode::Char('o'),
        P => KeyCode::Char('p'),
        Q => KeyCode::Char('q'),
        R => KeyCode::Char('r'),
        S => KeyCode::Char('s'),
        T => KeyCode::Char('t'),
        U => KeyCode::Char('u'),
        V => KeyCode::Char('v'),
        W => KeyCode::Char('w'),
        X => KeyCode::Char('x'),
        Y => KeyCode::Char('y'),
        Z => KeyCode::Char('z'),
        Num0 => KeyCode::Char('0'),
        Num1 => KeyCode::Char('1'),
        Num2 => KeyCode::Char('2'),
        Num3 => KeyCode::Char('3'),
        Num4 => KeyCode::Char('4'),
        Num5 => KeyCode::Char('5'),
        Num6 => KeyCode::Char('6'),
        Num7 => KeyCode::Char('7'),
        Num8 => KeyCode::Char('8'),
        Num9 => KeyCode::Char('9'),
        _ => return None,
    })
}

/// One theme in the picker: a row of colour swatches and the name.
fn theme_row(ui: &mut egui::Ui, current: &mut String, id: &str, label: &str, s: &crate::theme::Scheme) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(66.0, 14.0), egui::Sense::hover());
        let colors = [s.base, s.surface2, s.red, s.green, s.blue];
        for (i, c) in colors.iter().enumerate() {
            let r = egui::Rect::from_min_size(
                rect.min + egui::vec2(i as f32 * 13.0, 0.0),
                egui::vec2(13.0, 14.0),
            );
            ui.painter().rect_filled(r, 0.0, *c);
        }
        ui.selectable_value(current, id.to_string(), label);
    });
}

impl GuiApp {
    /// The colour scheme behind a theme id (built-in scheme or user theme file), if it is one.
    pub fn scheme_for(&self, id: &str) -> Option<crate::theme::Scheme> {
        crate::theme::scheme_named(id).or_else(|| self.user_themes.iter().find(|t| t.0 == id).map(|t| t.2))
    }

    /// Read theme files from the config folder again.
    pub fn reload_user_themes(&mut self) {
        self.user_themes = self
            .editor
            .config_dir()
            .map(|d| crate::theme::load_user_themes(&d.join("themes")))
            .unwrap_or_default();
    }

    /// Apply the saved theme and scale when they change.
    pub fn apply_appearance(&mut self, ctx: &egui::Context) {
        if self.applied_theme != self.editor.prefs.theme {
            let id = self.editor.prefs.theme.clone();
            if self.applied_theme.is_empty() {
                self.reload_user_themes();
            }
            match self.scheme_for(&id) {
                Some(s) => {
                    ctx.set_visuals(s.visuals());
                    self.palette = s.palette();
                }
                None => {
                    ctx.set_visuals(crate::theme::visuals(&id));
                    self.palette = Palette::named(&id);
                }
            }
            self.applied_theme = id;
        }
        let fonts = (self.editor.prefs.ui_font.clone(), self.editor.prefs.grid_font.clone());
        if self.applied_fonts.as_ref() != Some(&fonts) {
            let (defs, problems) = crate::fonts::definitions(&fonts.0, &fonts.1);
            ctx.set_fonts(defs);
            self.font_message = problems.join("; ");
            if !problems.is_empty() {
                self.editor.status = problems[0].clone();
            }
            self.applied_fonts = Some(fonts);
        }
        let scale = self.editor.prefs.ui_scale.clamp(0.6, 3.0);
        if (self.applied_scale - scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(scale);
            self.applied_scale = scale;
        }
    }

    /// While a key is being captured for a binding, take the next real key press.
    pub fn capture_binding(&mut self, ctx: &egui::Context) {
        let Some(action) = self.capturing else { return };
        let pressed = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    egui_to_keycode(*key).map(|code| (code, *modifiers))
                }
                _ => None,
            })
        });
        let Some((code, m)) = pressed else { return };
        self.capturing = None;
        if code == KeyCode::Esc && !m.command && !m.alt && !m.shift {
            self.binding_message.clear(); // Escape cancels
            return;
        }
        let key = KeyEvent { code, ctrl: m.command, alt: m.alt, shift: m.shift };
        self.binding_message = match self.editor.set_binding(action, Some(key)) {
            Ok(()) => format!("{} is now {}", action.label(), key.to_text()),
            Err(e) => e,
        };
    }

    pub fn prefs_window(&mut self, ctx: &egui::Context) {
        if !self.show_prefs {
            self.capturing = None;
            return;
        }
        let mut open = true;
        egui::Window::new("Preferences").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.prefs_tab, PrefsTab::Keys, "Keys");
                ui.selectable_value(&mut self.prefs_tab, PrefsTab::Appearance, "Appearance");
                ui.selectable_value(&mut self.prefs_tab, PrefsTab::Audio, "Audio");
                ui.selectable_value(&mut self.prefs_tab, PrefsTab::General, "General");
            });
            ui.separator();
            match self.prefs_tab {
                PrefsTab::Keys => self.keys_tab(ui),
                PrefsTab::Appearance => self.appearance_tab(ui),
                PrefsTab::Audio => self.audio_tab(ui),
                PrefsTab::General => self.general_tab(ui),
            }
        });
        if !open {
            self.show_prefs = false;
            self.capturing = None;
            self.editor.save_prefs();
        }
    }

    fn keys_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Click Set, then press the new shortcut (Esc cancels). Plain letters and digits stay reserved for entering notes.");
        if !self.binding_message.is_empty() {
            ui.colored_label(egui::Color32::from_rgb(230, 170, 60), &self.binding_message);
        }
        if ui.button("Reset all to defaults").clicked() {
            self.editor.reset_all_bindings();
            self.binding_message = "all shortcuts reset".into();
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("bindings").striped(true).num_columns(4).show(ui, |ui| {
                for &a in Action::ALL {
                    ui.label(a.label());
                    let text = self.editor.shortcut_for(a);
                    let capturing = self.capturing == Some(a);
                    ui.monospace(if capturing {
                        "press a key...".to_string()
                    } else if text.is_empty() {
                        "(none)".into()
                    } else {
                        text
                    });
                    if ui.small_button(if capturing { "Cancel" } else { "Set" }).clicked() {
                        self.capturing = if capturing { None } else { Some(a) };
                        self.binding_message.clear();
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Clear").clicked() {
                            let _ = self.editor.set_binding(a, None);
                        }
                        if ui.small_button("Default").clicked() {
                            self.editor.reset_binding(a);
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }

    fn appearance_tab(&mut self, ui: &mut egui::Ui) {
        let mut theme = self.editor.prefs.theme.clone();
        let current_label = THEMES
            .iter()
            .map(|t| (t.0, t.1.to_string()))
            .chain(crate::theme::SCHEMES.iter().map(|t| (t.0, t.1.to_string())))
            .chain(self.user_themes.iter().map(|t| (t.0.as_str(), format!("{} (yours)", t.1))))
            .find(|t| t.0 == theme)
            .map_or(theme.clone(), |t| t.1);
        ui.horizontal(|ui| {
            ui.label("Theme");
            egui::ComboBox::from_id_salt("theme_combo").width(260.0).selected_text(current_label).show_ui(
                ui,
                |ui| {
                    for (id, label) in THEMES {
                        ui.selectable_value(&mut theme, id.to_string(), label);
                    }
                    ui.separator();
                    for (id, label, s) in crate::theme::SCHEMES {
                        theme_row(ui, &mut theme, id, label, s);
                    }
                    if !self.user_themes.is_empty() {
                        ui.separator();
                        for (id, label, s) in &self.user_themes {
                            theme_row(ui, &mut theme, id, &format!("{label} (yours)"), s);
                        }
                    }
                },
            );
        });
        ui.horizontal(|ui| {
            if ui.button("Reload theme files").clicked() {
                self.reload_user_themes();
                self.applied_theme.clear(); // re-apply the current theme from the fresh files
            }
            if ui.button("Save current theme as a file").clicked() {
                self.save_theme_template();
            }
        });
        match self.editor.config_dir() {
            Some(d) => ui.weak(format!(
                "Your own themes: JSON files in {}/themes (only \"base\" and \"text\" are required).",
                d.display()
            )),
            None => ui.weak("No config folder available, so theme files cannot be used."),
        };
        let mut scale = self.editor.prefs.ui_scale;
        let mut font = self.editor.prefs.font_size;
        ui.horizontal(|ui| {
            ui.label("Interface scale");
            ui.add(egui::Slider::new(&mut scale, 0.75..=2.0).step_by(0.05));
        });
        ui.horizontal(|ui| {
            ui.label("Pattern font size");
            ui.add(egui::Slider::new(&mut font, 10.0..=26.0).step_by(1.0));
        });
        let (mut beat, mut bar, mut dec) = (
            self.editor.prefs.highlight_beat,
            self.editor.prefs.highlight_bar,
            self.editor.prefs.decimal_lines,
        );
        ui.horizontal(|ui| {
            ui.label("Highlight every");
            ui.add(egui::DragValue::new(&mut beat).range(0..=64).custom_formatter(|v, _| {
                if v == 0.0 {
                    "beat".to_string()
                } else {
                    format!("{v:.0} lines")
                }
            }));
            ui.label("and every");
            ui.add(egui::DragValue::new(&mut bar).range(0..=32).custom_formatter(|v, _| {
                if v == 0.0 {
                    "4 beats".to_string()
                } else {
                    format!("{v:.0} beats")
                }
            }));
            ui.checkbox(&mut dec, "Decimal line numbers");
        });
        self.editor.prefs.highlight_beat = beat;
        self.editor.prefs.highlight_bar = bar;
        self.editor.prefs.decimal_lines = dec;
        ui.separator();
        ui.strong("Fonts");
        let (mut ui_font, mut grid_font) =
            (self.editor.prefs.ui_font.clone(), self.editor.prefs.grid_font.clone());
        self.font_picker(ui, "Interface font", "ui_font_pick", &mut ui_font);
        self.font_picker(ui, "Pattern font", "grid_font_pick", &mut grid_font);
        if !grid_font.is_empty()
            && std::path::Path::new(&grid_font).is_file()
            && !crate::fonts::looks_monospaced(&grid_font)
        {
            ui.weak("This pattern font is not monospaced: columns will not line up. Pick a mono font for the grid.");
        }
        if !self.font_message.is_empty() {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.font_message);
        }
        ui.weak("Any .ttf or .otf file. Monospaced fonts line up best in the pattern grid. Missing glyphs fall back to the built-in font.");
        self.editor.prefs.ui_font = ui_font;
        self.editor.prefs.grid_font = grid_font;
        self.editor.prefs.theme = theme;
        self.editor.prefs.ui_scale = scale;
        self.editor.prefs.font_size = font;
    }

    #[cfg(test)]
    pub fn save_theme_template_for_test(&mut self) {
        self.save_theme_template();
    }

    /// A font chooser: system fonts (filterable), a custom path, and "built in".
    fn font_picker(&mut self, ui: &mut egui::Ui, label: &str, id: &str, value: &mut String) {
        if self.font_list.is_empty() {
            self.font_list = crate::fonts::scan_fonts(&crate::fonts::default_font_dirs());
        }
        let shown = if value.is_empty() {
            "Built in".to_string()
        } else {
            std::path::Path::new(value.as_str())
                .file_stem()
                .map_or(value.clone(), |s| s.to_string_lossy().into_owned())
        };
        ui.horizontal(|ui| {
            ui.label(label);
            egui::ComboBox::from_id_salt(id).width(280.0).selected_text(shown).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.font_filter).hint_text("filter..."));
                if ui.selectable_label(value.is_empty(), "Built in").clicked() {
                    value.clear();
                }
                let f = self.font_filter.to_lowercase();
                egui::ScrollArea::vertical().max_height(260.0).id_salt((id, "list")).show(ui, |ui| {
                    for font in
                        self.font_list.iter().filter(|x| f.is_empty() || x.name.to_lowercase().contains(&f))
                    {
                        let p = font.path.to_string_lossy().into_owned();
                        if ui.selectable_label(*value == p, &font.name).clicked() {
                            *value = p;
                        }
                    }
                });
            });
            if ui.small_button("Rescan").on_hover_text("Look for fonts again").clicked() {
                self.font_list = crate::fonts::scan_fonts(&crate::fonts::default_font_dirs());
            }
        });
        ui.horizontal(|ui| {
            ui.label("   or a file");
            let mut path = value.clone();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut path).desired_width(360.0).hint_text("/path/to/font.ttf"),
                )
                .changed()
            {
                *value = path;
            }
        });
    }

    /// Write the active theme's colours to `<config>/themes/my-theme.json` as a starting point.
    fn save_theme_template(&mut self) {
        let Some(dir) = self.editor.config_dir().map(|d| d.join("themes")) else {
            self.editor.status = "no config folder available".into();
            return;
        };
        let id = self.editor.prefs.theme.clone();
        let scheme = self.scheme_for(&id).unwrap_or_else(|| {
            crate::theme::scheme_named(if id == "light" { "catppuccin-latte" } else { "catppuccin-mocha" })
                .expect("built in")
        });
        let path = dir.join("my-theme.json");
        let result =
            std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, scheme.to_json("My theme")));
        self.editor.status = match result {
            Ok(()) => format!("wrote {}: edit it, then Reload theme files", path.display()),
            Err(e) => format!("cannot write theme file: {e}"),
        };
    }

    fn audio_tab(&mut self, ui: &mut egui::Ui) {
        let Some(ctl) = self.editor.audio_control() else {
            ui.label("Audio selection is not available in this build.");
            return;
        };
        let hosts = ctl.hosts();
        let mut a = self.editor.prefs.audio.clone();
        let outputs = ctl.outputs(&a.host);
        let inputs = ctl.inputs(&a.host);
        let rates = ctl.rates(&a.host, &a.output);
        let pick = |ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, options: &[String]| {
            ui.horizontal(|ui| {
                ui.add_sized([110.0, 18.0], egui::Label::new(label));
                let shown = if value.is_empty() { "Default".to_string() } else { value.clone() };
                egui::ComboBox::from_id_salt(id).selected_text(shown).width(300.0).show_ui(ui, |ui| {
                    ui.selectable_value(value, String::new(), "Default");
                    for o in options {
                        ui.selectable_value(value, o.clone(), o);
                    }
                });
            });
        };
        let host_before = a.host.clone();
        pick(ui, "audio_host", "Audio system", &mut a.host, &hosts);
        if a.host != host_before {
            a.output.clear();
            a.input.clear();
        }
        pick(ui, "audio_out", "Output device", &mut a.output, &outputs);
        pick(ui, "audio_in", "Input device", &mut a.input, &inputs);
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 18.0], egui::Label::new("Sample rate"));
            let shown =
                if a.sample_rate == 0 { "Default".to_string() } else { format!("{} Hz", a.sample_rate) };
            egui::ComboBox::from_id_salt("audio_rate").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut a.sample_rate, 0, "Default");
                for r in &rates {
                    ui.selectable_value(&mut a.sample_rate, *r, format!("{r} Hz"));
                }
            });
        });
        ui.horizontal(|ui| {
            ui.add_sized([110.0, 18.0], egui::Label::new("Buffer size"));
            let shown = if a.buffer_frames == 0 {
                "Default".to_string()
            } else {
                format!("{} frames", a.buffer_frames)
            };
            egui::ComboBox::from_id_salt("audio_buf").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut a.buffer_frames, 0, "Default");
                for b in [64u32, 128, 256, 512, 1024, 2048, 4096] {
                    ui.selectable_value(&mut a.buffer_frames, b, format!("{b} frames"));
                }
            });
            ui.weak("smaller = lower latency, more risk of dropouts");
        });
        let changed = a != self.editor.prefs.audio;
        self.editor.prefs.audio = a;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(true, egui::Button::new("Apply"))
                .on_hover_text("Restart the audio output with these settings")
                .clicked()
            {
                self.editor.audio_apply();
            }
            if changed {
                ui.weak("not applied yet");
            }
        });
        if let Some(i) = self.editor.audio_info() {
            let buffer = if i.buffer_frames > 0 {
                format!(", buffer {} frames ({:.1} ms)", i.buffer_frames, i.buffer_ms())
            } else {
                ", driver-chosen buffer".to_string()
            };
            ui.separator();
            ui.label(format!(
                "Running: {} / {} at {} Hz, {} channels{}",
                i.host, i.device, i.sample_rate, i.channels, buffer
            ));
        } else {
            ui.separator();
            ui.label("No audio output is running.");
        }
        if !hosts.iter().any(|h| h.eq_ignore_ascii_case("jack")) {
            ui.weak("JACK is not part of this build (build with --features trakr-engine/jack and libjack installed).");
        }
    }

    fn general_tab(&mut self, ui: &mut egui::Ui) {
        let mut secs = self.editor.prefs.autosave_seconds as u32;
        ui.horizontal(|ui| {
            ui.label("Autosave every");
            ui.add(egui::DragValue::new(&mut secs).range(0..=3600).suffix(" s"));
            ui.weak("(0 turns autosave off)");
        });
        self.editor.prefs.autosave_seconds = secs as u64;
        ui.checkbox(&mut self.editor.prefs.backups, "Keep the previous file as <name>.bak when saving");
        ui.checkbox(&mut self.editor.prefs.reopen_last, "Open the most recent song at start-up");
        ui.weak("Window size and the bottom panel are remembered automatically.");
        ui.separator();
        let mut on = self.editor.prefs.osc_port.is_some();
        let mut port = self.editor.prefs.osc_port.unwrap_or(9000);
        let mut any = self.editor.prefs.osc_any_interface;
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut on, "OSC remote control on UDP port").changed();
            changed |= ui.add(egui::DragValue::new(&mut port).range(1024..=65535)).changed();
            changed |= ui.checkbox(&mut any, "accept other computers").changed();
        });
        ui.weak("Addresses: /trakr/play, /stop, /bpm, /action <name>, /track/<n>/volume|pan|mute|solo, /device/<t>/<d>/<p>, /note/on <note> [vel] [inst]. This machine only unless you tick the box.");
        if changed {
            self.editor.prefs.osc_port = on.then_some(port);
            self.editor.prefs.osc_any_interface = any;
            if on {
                self.editor.osc_start(port, any);
            } else {
                self.editor.osc_stop();
            }
        }
        if let Some(a) = self.editor.osc_addr() {
            ui.weak(format!("Listening on {a}"));
        }
        let mut target = self.editor.prefs.osc_feedback.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("OSC feedback to");
            let edit = ui
                .add(egui::TextEdit::singleline(&mut target).hint_text("host:port, e.g. 192.168.1.20:9001"));
            if edit.lost_focus() {
                if target.trim().is_empty() {
                    self.editor.prefs.osc_feedback = None;
                    self.editor.osc_feedback_stop();
                } else if self.editor.osc_feedback_start(target.trim()) {
                    self.editor.prefs.osc_feedback = Some(target.trim().to_string());
                }
            }
        });
        ui.weak("Sends play state, position, tempo and each track's volume, pan, mute, solo and level whenever they change (for faders and lights on a control surface).");
    }
}
