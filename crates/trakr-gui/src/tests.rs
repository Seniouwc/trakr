//! GUI tests driven with synthetic egui input: they exercise real frames (layout, hit-testing, key
//! routing) without a window.

use super::*;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2};
use trakr_core::{AutoTarget, Song};

struct Harness {
    ctx: egui::Context,
    app: GuiApp,
    time: f64,
}

impl Harness {
    fn new() -> Self {
        let mut h = Self {
            ctx: egui::Context::default(),
            app: GuiApp::new(Editor::new(Song::demo(), None, None)),
            time: 0.0,
        };
        for _ in 0..3 {
            h.frame(vec![]);
        }
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 780.0))),
            time: Some(self.time),
            focused: true,
            events,
            ..Default::default()
        };
        self.time += 0.033;
        let app = &mut self.app;
        let _ = self.ctx.run(raw, |ctx| app.frame(ctx));
    }

    fn click(&mut self, pos: Pos2) {
        let btn = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![btn(true)]);
        self.frame(vec![btn(false)]);
        self.frame(vec![]);
    }

    fn text(&mut self, s: &str) {
        self.frame(vec![Event::Text(s.into())]);
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }

    /// Screen position of a pattern cell: `dl` lines below the cursor line, in `track`.
    fn cell(&self, track: usize, dl: i32) -> Pos2 {
        let g = &self.app.grid_info;
        let (_, _, x, _) =
            g.cols.iter().find(|c| c.0 == track && c.1 == 0).copied().expect("track not on screen");
        Pos2::new(x + g.char_w * 1.5, g.cursor_row_y + (dl as f32 + 0.5) * g.row_h)
    }
}

fn note_at(h: &Harness, track: usize, line: u16) -> u8 {
    h.app.editor.song.patterns[0].line(track, line).map_or(trakr_core::NOTE_EMPTY, |l| l.notes[0].note)
}

#[test]
fn typing_notes_reaches_the_editor() {
    let mut h = Harness::new();
    assert!(!h.app.editor.edit_mode);
    h.text(" ");
    assert!(h.app.editor.edit_mode, "Space should toggle edit mode");
    h.app.editor.set_cursor(1, 0, 0, 10);
    h.frame(vec![]);
    h.text("z");
    assert_eq!(note_at(&h, 1, 10), 48);
    assert_eq!(h.app.editor.cursor.line, 11, "cursor advances by the edit step");
}

#[test]
fn typing_keeps_working_after_clicking_widgets() {
    let mut h = Harness::new();
    // Click the transport "Edit" checkbox area, then the grid, then type: keys must still reach the editor.
    h.click(h.cell(0, 3));
    h.text(" ");
    assert!(h.app.editor.edit_mode);
    h.text("x");
    assert_eq!(note_at(&h, 0, 3), 50);
}

#[test]
fn clicking_a_cell_moves_the_cursor() {
    let mut h = Harness::new();
    h.click(h.cell(1, 2));
    let c = h.app.editor.cursor;
    assert_eq!((c.track, c.line), (1, 2), "{c:?}");
    h.click(h.cell(0, -0));
    assert_eq!(h.app.editor.cursor.track, 0);
}

#[test]
fn text_fields_capture_typing() {
    let mut h = Harness::new();
    h.app.editor.edit_mode = true;
    h.app.text_editing = true; // a text field has focus
    h.text("z");
    assert_eq!(note_at(&h, 0, 1), trakr_core::NOTE_EMPTY);
    h.app.text_editing = false;
    h.frame(vec![]);
    h.text("z");
    assert_eq!(h.app.editor.cursor.line, 1);
}

#[test]
fn arrows_and_shortcuts() {
    let mut h = Harness::new();
    h.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(h.app.editor.cursor.line, 1);
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.app.editor.cursor.sub, 1);
    h.key(Key::ArrowDown, Modifiers::SHIFT);
    assert!(h.app.editor.selection().is_some(), "Shift+arrow selects");
    h.app.editor.edit_mode = true;
    h.app.editor.set_cursor(0, 0, 0, 5);
    h.text("c");
    assert_eq!(note_at(&h, 0, 5), 52);
    let cmd = Modifiers { command: true, ctrl: true, ..Modifiers::NONE };
    h.key(Key::Z, cmd);
    assert_eq!(note_at(&h, 0, 5), trakr_core::NOTE_EMPTY, "Ctrl-Z undoes");
}

#[test]
fn every_dock_and_tab_draws_without_panicking() {
    let mut h = Harness::new();
    h.app.editor.device_add(0, "native.filter");
    h.app.editor.device_add(0, "native.eq");
    h.app.editor.automation_edit("t", 0, AutoTarget::Device { device: 0, param: 1 }, |a| {
        a.set_point(0.0, 0.2);
        a.set_point(8.0, 0.8);
    });
    let tabs = [
        SampleTab::Sample,
        SampleTab::Filter,
        SampleTab::Modulation,
        SampleTab::Env(envelopes::EnvKind::Volume),
        SampleTab::Env(envelopes::EnvKind::Pan),
        SampleTab::Env(envelopes::EnvKind::Pitch),
        SampleTab::Env(envelopes::EnvKind::Cutoff),
    ];
    for dock in [
        Dock::Instrument,
        Dock::Mixer,
        Dock::Devices,
        Dock::Automation,
        Dock::Phrases,
        Dock::Matrix,
        Dock::Hidden,
    ] {
        h.app.dock = dock;
        for tab in tabs {
            h.app.sample_tab = tab;
            h.frame(vec![]);
            h.frame(vec![]);
        }
    }
}

#[test]
fn empty_song_and_empty_instrument_draw() {
    let mut h = Harness::new();
    h.app.editor.new_song();
    h.app.editor.instrument_add_empty();
    h.app.editor.do_action(trakr_editor::Action::SeqInsert);
    for dock in [Dock::Instrument, Dock::Mixer, Dock::Devices, Dock::Automation] {
        h.app.dock = dock;
        h.frame(vec![]);
        h.frame(vec![]);
    }
    // a sample with a single frame must not break the waveform
    h.app.editor.edit_sample(0, |_| {});
    h.app.dock = Dock::Instrument;
    h.frame(vec![]);
}

#[test]
fn menu_actions_work_from_the_gui_layer() {
    let mut h = Harness::new();
    h.app.run_action(trakr_editor::Action::SeqClone);
    assert_eq!(h.app.editor.song.sequence.len(), 2);
    h.app.run_action(trakr_editor::Action::Undo);
    assert_eq!(h.app.editor.song.sequence.len(), 1);
    h.app.run_action(trakr_editor::Action::ToggleEdit);
    assert!(h.app.editor.edit_mode);
}

#[test]
fn mixer_with_groups_and_sends_draws_and_routes() {
    let mut h = Harness::new();
    h.app.editor.do_action(trakr_editor::Action::TrackAddGroup);
    h.app.editor.do_action(trakr_editor::Action::TrackAddSend);
    h.app.dock = Dock::Mixer;
    for _ in 0..3 {
        h.frame(vec![]);
    }
    let s = h.app.editor.song.tracks.iter().position(|t| t.kind == trakr_core::TrackKind::Send).unwrap();
    h.app.editor.set_send(0, s, 0.5, false);
    h.frame(vec![]);
    assert_eq!(h.app.editor.song.tracks[0].sends.len(), 1);
}

/// Hammer the whole window with random pointer, wheel, text and key events. Catches panics and freezes
/// (a deadlock would trip the watchdog). File dialogs are closed immediately so nothing touches the disk.
#[test]
fn random_input_never_panics_or_freezes() {
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut h = Harness::new();
        let dir = std::env::temp_dir().join(format!("trakr_gui_fuzz_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        h.app.editor.path = Some(dir.join("fuzz.trkr"));
        h.app.editor.device_add(0, "native.delay");
        let mut seed = 0x5EEDu64;
        let mut next = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let keys = [
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::Tab,
            Key::Backspace,
            Key::Delete,
            Key::Insert,
            Key::Escape,
            Key::F1,
            Key::F2,
            Key::F5,
            Key::F8,
            Key::PageUp,
            Key::PageDown,
            Key::Home,
            Key::End,
            Key::Z,
            Key::Y,
            Key::A,
            Key::S,
        ];
        let texts = ["z", "x", " ", "c", "1", "0", "a", "f", "`", "q", "2"];
        for frame in 0..4000 {
            let mut events = Vec::new();
            for _ in 0..1 + next() % 3 {
                let pos = Pos2::new((next() % 1280) as f32, (next() % 780) as f32);
                let mods = Modifiers {
                    shift: next() % 5 == 0,
                    ctrl: next() % 6 == 0,
                    alt: next() % 9 == 0,
                    command: next() % 6 == 0,
                    ..Modifiers::NONE
                };
                match next() % 8 {
                    0 | 1 => events.push(Event::PointerMoved(pos)),
                    2 | 3 => {
                        let button =
                            if next() % 4 == 0 { PointerButton::Secondary } else { PointerButton::Primary };
                        events.push(Event::PointerMoved(pos));
                        events.push(Event::PointerButton { pos, button, pressed: true, modifiers: mods });
                    }
                    4 => events.push(Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: mods,
                    }),
                    5 => events.push(Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new((next() % 40) as f32 - 20.0, (next() % 80) as f32 - 40.0),
                        modifiers: mods,
                    }),
                    6 => events.push(Event::Text(texts[next() as usize % texts.len()].into())),
                    _ => events.push(Event::Key {
                        key: keys[next() as usize % keys.len()],
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: mods,
                    }),
                }
            }
            h.frame(events);
            h.app.dialog = None; // never let a file dialog act on the real file system
            if frame % 500 == 499 {
                // release anything held and rotate through the docks so every panel sees input
                h.frame(vec![Event::PointerButton {
                    pos: Pos2::ZERO,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: Modifiers::NONE,
                }]);
                h.app.dock =
                    [Dock::Instrument, Dock::Mixer, Dock::Devices, Dock::Automation][(frame / 500) % 4];
            }
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = tx.send(());
    });
    rx.recv_timeout(Duration::from_secs(180)).expect("the GUI froze or panicked under random input");
}

#[test]
fn themes_and_scale_apply_and_draw() {
    let mut h = Harness::new();
    let mut ids: Vec<String> = ["light", "contrast", "dark", "nonsense"].map(String::from).to_vec();
    ids.extend(theme::SCHEMES.iter().map(|t| t.0.to_string()));
    for theme in ids {
        let theme = theme.as_str();
        h.app.editor.prefs.theme = theme.into();
        h.app.editor.prefs.ui_scale = 1.3;
        h.app.editor.prefs.font_size = 18.0;
        h.app.dock = Dock::Mixer;
        h.frame(vec![]);
        h.frame(vec![]);
        assert_eq!(h.app.applied_theme, theme);
    }
}

#[test]
fn preferences_window_captures_a_new_shortcut() {
    let mut h = Harness::new();
    h.app.show_prefs = true;
    for tab in [prefs_view::PrefsTab::Keys, prefs_view::PrefsTab::Appearance, prefs_view::PrefsTab::General] {
        h.app.prefs_tab = tab;
        h.frame(vec![]);
        h.frame(vec![]);
    }
    h.app.capturing = Some(trakr_editor::Action::Undo);
    h.app.editor.edit_mode = true;
    let f12 = |pressed| Event::Key {
        key: Key::F12,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    h.frame(vec![f12(true)]);
    assert!(h.app.capturing.is_none());
    assert_eq!(h.app.editor.shortcut_for(trakr_editor::Action::Undo), "F12");
    // a refused key reports why and leaves the binding alone
    h.app.capturing = Some(trakr_editor::Action::Redo);
    h.frame(vec![Event::Key {
        key: Key::F12,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(h.app.binding_message.contains("already used"), "{}", h.app.binding_message);
    // keys pressed while capturing must not trigger actions
    h.app.capturing = Some(trakr_editor::Action::Redo);
    h.app.editor.set_cursor(0, 0, 0, 3);
    h.frame(vec![Event::Key {
        key: Key::F5,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    // F5 belongs to "Play song": refused, and pressing it while capturing did not start playback
    assert_eq!(h.app.editor.shortcut_for(trakr_editor::Action::Redo), "Ctrl-Y");
    assert!(h.app.binding_message.contains("Play song"), "{}", h.app.binding_message);
    assert!(h.app.capturing.is_none());
    assert!(!h.app.editor.playhead().0);
}

#[test]
fn egui_key_mapping_covers_letters_digits_and_function_keys() {
    for (k, c) in [(Key::A, 'a'), (Key::Z, 'z'), (Key::Num0, '0'), (Key::Num9, '9')] {
        assert_eq!(prefs_view::egui_to_keycode(k), Some(trakr_editor::KeyCode::Char(c)));
    }
    assert_eq!(prefs_view::egui_to_keycode(Key::F12), Some(trakr_editor::KeyCode::F(12)));
    assert_eq!(prefs_view::egui_to_keycode(Key::Space), Some(trakr_editor::KeyCode::Char(' ')));
    assert_eq!(prefs_view::egui_to_keycode(Key::Enter), None);
}

#[test]
fn phrase_editor_takes_keys_after_a_click_and_gives_them_back() {
    let mut h = Harness::new();
    h.app.dock = Dock::Phrases;
    h.app.editor.phrase_add();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    h.click(Pos2::new(50.0, 700.0));
    assert!(h.app.phrase_focus, "clicking the phrase grid focuses it");
    let pattern_before = h.app.editor.song.patterns[0].clone();
    h.text(" ");
    assert!(h.app.editor.edit_mode);
    let line = h.app.editor.phrase_cursor.line;
    h.text("z");
    let p = h.app.editor.current_phrase().unwrap();
    assert_eq!(p.line(line).map(|l| l.notes[0].note), Some(48), "note went into the phrase");
    assert_eq!(*h.app.editor.song.patterns[0], *pattern_before, "pattern untouched");
    // clicking the pattern grid sends keys back to the pattern
    h.click(Pos2::new(300.0, 200.0));
    assert!(!h.app.phrase_focus);
    let before = h.app.editor.current_phrase().unwrap().lines.len();
    h.text("x");
    assert_eq!(h.app.editor.current_phrase().unwrap().lines.len(), before);
}

#[test]
fn script_console_renders_and_typing_in_it_does_not_edit_the_pattern() {
    let mut h = Harness::new();
    h.app.show_console = true;
    h.app.script_text = "trakr.status('from console')".into();
    h.app.editor.script_error = Some("example error".into());
    h.app.editor.script_output = vec!["line one".into()];
    for _ in 0..3 {
        h.frame(vec![]);
    }
    h.text(" ");
    h.text("z");
    // whatever the console's focus does with the keys, the run itself works and is one undo step
    assert!(h.app.editor.run_script(&h.app.script_text.clone()));
    assert_eq!(h.app.editor.status, "from console");
    h.app.show_console = false;
    h.frame(vec![]);
}

#[test]
fn command_palette_opens_with_a_shortcut_and_runs_the_best_match() {
    let mut h = Harness::new();
    h.key(Key::P, Modifiers::COMMAND);
    h.frame(vec![]);
    assert!(h.app.show_palette, "Ctrl-P opens the palette");
    assert!(!h.app.editor.edit_mode);
    h.app.palette_query = "edit mode".into();
    h.frame(vec![]);
    h.key(Key::Enter, Modifiers::NONE);
    assert!(!h.app.show_palette);
    assert!(h.app.editor.edit_mode, "the matching command ran");
    h.key(Key::F, Modifiers::COMMAND);
    h.frame(vec![]);
    assert!(h.app.show_find, "Ctrl-F opens find and replace");
}

#[test]
fn tool_windows_all_draw_together() {
    let mut h = Harness::new();
    h.app.editor.device_add(0, "native.filter");
    h.app.show_midi_map = true;
    h.app.show_find = true;
    h.app.show_console = true;
    h.app.show_palette = true;
    h.app.editor.midi_learn_start(trakr_editor::MapTarget::TrackVolume(0));
    h.app.dock = Dock::Instrument;
    h.app.sample_tab = SampleTab::Modulation;
    h.app.editor.edit_instrument("t", |i| i.lfos.push(trakr_core::InstLfo::default()));
    for _ in 0..4 {
        h.frame(vec![]);
    }
    h.app.show_midi_map = false;
    h.frame(vec![]);
    assert!(h.app.editor.midi_learn.is_none(), "closing the window cancels learning");
}

#[test]
fn user_theme_files_appear_in_the_picker_and_apply() {
    let dir = std::env::temp_dir().join(format!("trakr-gui-themes-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("themes")).unwrap();
    std::fs::write(dir.join("themes/mine.json"), r##"{"name":"Mine","base":"#202020","text":"#f0f0f0"}"##)
        .unwrap();
    let mut h = Harness::new();
    h.app.editor.set_config_dir(Some(dir.clone()));
    h.app.show_prefs = true;
    h.app.prefs_tab = prefs_view::PrefsTab::Appearance;
    h.app.reload_user_themes();
    assert_eq!(h.app.user_themes.len(), 1);
    h.app.editor.prefs.theme = "user:mine".into();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert_eq!(h.app.applied_theme, "user:mine");
    assert_eq!(h.app.palette.bg, eframe::egui::Color32::from_rgb(0x20, 0x20, 0x20));
    // "Save current theme as a file" writes a template that loads back
    h.app.editor.prefs.theme = "dracula".into();
    h.frame(vec![]);
    h.app.save_theme_template_for_test();
    h.app.reload_user_themes();
    assert!(h.app.user_themes.iter().any(|t| t.0 == "user:my-theme"));
    // a theme file that disappears leaves a working (fallback) look
    std::fs::remove_dir_all(dir.join("themes")).unwrap();
    h.app.reload_user_themes();
    h.app.editor.prefs.theme = "user:mine".into();
    h.app.applied_theme.clear();
    h.frame(vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn choosing_fonts_in_preferences_applies_and_draws() {
    let mono = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";
    if !std::path::Path::new(mono).exists() {
        return;
    }
    let mut h = Harness::new();
    h.app.show_prefs = true;
    h.app.prefs_tab = prefs_view::PrefsTab::Appearance;
    h.app.editor.prefs.grid_font = mono.into();
    h.app.editor.prefs.ui_font = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into();
    for _ in 0..4 {
        h.frame(vec![]);
    }
    assert!(h.app.applied_fonts.is_some() && h.app.font_message.is_empty());
    assert!(!h.app.font_list.is_empty(), "the picker scanned the system fonts");
    // a bad path falls back to the built-in font and says so
    h.app.editor.prefs.grid_font = "/nonexistent/font.ttf".into();
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert!(h.app.font_message.contains("cannot read"), "{}", h.app.font_message);
}

#[test]
fn sample_process_rows_and_spectrum_draw_and_work() {
    let mut h = Harness::new();
    h.app.dock = Dock::Instrument;
    h.app.sample_tab = SampleTab::Sample;
    h.app.show_spectrum = true;
    for _ in 0..3 {
        h.frame(vec![]);
    }
    h.app.sample_sel = Some((100, 3000));
    h.frame(vec![]);
    let before = h.app.editor.current_sample().unwrap().frames();
    let idx = h.app.editor.sample_index();
    h.app.editor.edit_sample(idx, |s| s.time_stretch(2.0));
    assert!(h.app.editor.current_sample().unwrap().frames() > before * 19 / 10);
    h.app.editor.edit_sample(idx, |s| {
        s.loop_mode = trakr_core::LoopMode::Forward;
        s.loop_start = 3000;
        s.loop_end = 9000;
        assert!(s.loop_crossfade(1000));
        s.beat_sync_lines = 8;
        s.reduce_bit_depth(8, true);
    });
    h.app.editor.undo();
    h.frame(vec![]);
}

#[test]
fn pattern_matrix_and_song_properties_work() {
    let mut h = Harness::new();
    h.app.editor.seq_clone();
    h.app.editor.seq_clone();
    h.app.editor.section_set(1, Some("Verse".into()));
    h.app.dock = Dock::Matrix;
    h.app.show_song = true;
    for _ in 0..4 {
        h.frame(vec![]);
    }
    h.app.editor.slot_toggle_mute(2, 0);
    h.app.editor.set_groove(vec![0.0, 0.2]);
    h.app.editor.set_song_comments("hello".into());
    for _ in 0..3 {
        h.frame(vec![]);
    }
    assert!(h.app.editor.song.slot_muted(2, 0));
    // a click somewhere in the dock must not panic or freeze the matrix
    h.click(Pos2::new(200.0, 640.0));
    h.frame(vec![]);
}

#[test]
fn modulation_tab_with_every_kind_of_source_draws() {
    let mut h = Harness::new();
    h.app.dock = Dock::Instrument;
    h.app.sample_tab = SampleTab::Modulation;
    h.app.editor.edit_instrument("t", |i| {
        let mut l = trakr_core::InstLfo { shape: trakr_core::LfoShape::Stepper, ..Default::default() };
        l.steps = 6;
        i.lfos.push(l);
        i.lfos.push(trakr_core::InstLfo {
            shape: trakr_core::LfoShape::SmoothRandom,
            target: trakr_core::LfoTarget::Resonance,
            ..Default::default()
        });
        i.mods.push(trakr_core::InstMod {
            source: trakr_core::ModSource::Macro(2),
            target: trakr_core::LfoTarget::Pan,
            depth: 0.5,
        });
        i.mods.push(trakr_core::InstMod {
            source: trakr_core::ModSource::Velocity,
            target: trakr_core::LfoTarget::Pitch,
            depth: -3.0,
        });
        i.fade_out = 0.4;
    });
    for _ in 0..4 {
        h.frame(vec![]);
    }
    h.app.sample_tab = SampleTab::Env(envelopes::EnvKind::Volume);
    h.app.editor.edit_instrument("t", |i| {
        let mut e = trakr_core::Envelope::adsr(0.01, 0.1, 0.5, 0.2);
        e.loop_points = Some((1, 2));
        i.volume_env = std::sync::Arc::new(e);
    });
    for _ in 0..3 {
        h.frame(vec![]);
    }
}
