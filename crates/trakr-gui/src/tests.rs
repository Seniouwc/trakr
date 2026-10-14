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
        SampleTab::Env(envelopes::EnvKind::Volume),
        SampleTab::Env(envelopes::EnvKind::Pan),
        SampleTab::Env(envelopes::EnvKind::Pitch),
        SampleTab::Env(envelopes::EnvKind::Cutoff),
    ];
    for dock in [Dock::Instrument, Dock::Mixer, Dock::Devices, Dock::Automation, Dock::Hidden] {
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
