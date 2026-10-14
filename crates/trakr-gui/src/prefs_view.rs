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

impl GuiApp {
    /// Apply the saved theme and scale when they change.
    pub fn apply_appearance(&mut self, ctx: &egui::Context) {
        let p = &self.editor.prefs;
        if self.applied_theme != p.theme {
            ctx.set_visuals(crate::theme::visuals(&p.theme));
            self.palette = Palette::named(&p.theme);
            self.applied_theme = p.theme.clone();
        }
        let scale = p.ui_scale.clamp(0.6, 3.0);
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
                ui.selectable_value(&mut self.prefs_tab, PrefsTab::General, "General");
            });
            ui.separator();
            match self.prefs_tab {
                PrefsTab::Keys => self.keys_tab(ui),
                PrefsTab::Appearance => self.appearance_tab(ui),
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
        ui.horizontal(|ui| {
            ui.label("Theme");
            for (id, label) in THEMES {
                ui.selectable_value(&mut theme, id.to_string(), label);
            }
        });
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
        self.editor.prefs.theme = theme;
        self.editor.prefs.ui_scale = scale;
        self.editor.prefs.font_size = font;
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
    }
}
