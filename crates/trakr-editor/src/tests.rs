use super::*;
use std::path::PathBuf;
use trakr_core::NOTE_EMPTY;

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c))
}

fn ed() -> Editor {
    Editor::new(Song::default(), None, None)
}

fn note_at(e: &Editor, t: usize, l: u16) -> u8 {
    e.song.patterns[0].line(t, l).map_or(NOTE_EMPTY, |x| x.notes[0].note)
}

fn put(e: &mut Editor, line: u16, note: u8) {
    e.set_cursor(0, 0, 0, line);
    e.edit(|l| l.notes[0].note = note);
}

#[test]
fn note_entry_writes_and_advances() {
    let mut app = ed();
    app.on_key(key(' '));
    app.on_key(key('z'));
    let line = app.song.patterns[0].line(0, 0).unwrap();
    assert_eq!(line.notes[0].note, 48);
    assert_eq!(line.notes[0].instrument, 0);
    assert_eq!(app.cursor.line, 1);
}

#[test]
fn nothing_written_when_edit_mode_off() {
    let mut app = ed();
    app.on_key(key('z'));
    assert!(app.song.patterns[0].line(0, 0).is_none());
}

#[test]
fn hex_entry_in_volume_column() {
    let mut app = ed();
    app.on_key(key(' '));
    for _ in 0..3 {
        app.on_key(KeyEvent::new(KeyCode::Right));
    }
    app.on_key(key('4'));
    app.on_key(key('0'));
    assert_eq!(app.song.patterns[0].line(0, 0).unwrap().notes[0].volume, 0x40);
}

#[test]
fn up_wraps_through_sequence() {
    let mut app = ed();
    app.on_key(KeyEvent::new(KeyCode::Up));
    assert_eq!(app.cursor.line, 63);
}

#[test]
fn undo_redo_roundtrip() {
    let mut app = ed();
    app.on_key(key(' '));
    app.on_key(key('z'));
    app.on_key(key('x'));
    assert_eq!(note_at(&app, 0, 1), 50);
    app.on_key(KeyEvent::ctrl('z'));
    assert_eq!(note_at(&app, 0, 1), NOTE_EMPTY);
    assert_eq!(note_at(&app, 0, 0), 48);
    app.on_key(KeyEvent::ctrl('z'));
    assert!(app.song.patterns[0].line(0, 0).is_none());
    app.on_key(KeyEvent::ctrl('y'));
    assert_eq!(note_at(&app, 0, 0), 48);
    // a new edit clears the redo stack
    app.on_key(key('c'));
    assert!(!app.can_redo());
}

#[test]
fn copy_paste_block() {
    let mut app = ed();
    app.edit_mode = true;
    for l in 0..4 {
        put(&mut app, l, 40 + l as u8);
    }
    app.set_cursor(0, 0, 0, 0);
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.do_action(Action::Copy);
    app.set_cursor(1, 0, 0, 8);
    app.do_action(Action::Paste);
    for l in 0..4u16 {
        assert_eq!(note_at(&app, 1, 8 + l), 40 + l as u8);
    }
    app.undo();
    assert_eq!(note_at(&app, 1, 8), NOTE_EMPTY);
}

#[test]
fn mix_paste_keeps_existing_cells_and_flood_paste_repeats() {
    let mut app = ed();
    app.edit_mode = true;
    // clipboard: a note on line 0 and an empty line 1
    put(&mut app, 0, 60);
    app.set_cursor(0, 0, 0, 0);
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.do_action(Action::Copy);
    // destination already has notes on both lines
    put(&mut app, 8, 30);
    put(&mut app, 9, 31);
    app.set_cursor(0, 0, 0, 8);
    app.do_action(Action::PasteMix);
    assert_eq!(
        (note_at(&app, 0, 8), note_at(&app, 0, 9)),
        (60, 31),
        "the empty clipboard line kept the old note"
    );
    // a plain paste overwrites both
    app.set_cursor(0, 0, 0, 8);
    put(&mut app, 9, 31);
    app.set_cursor(0, 0, 0, 8);
    app.do_action(Action::Paste);
    assert_eq!((note_at(&app, 0, 8), note_at(&app, 0, 9)), (60, NOTE_EMPTY));
    app.undo();
    // flood: the two-line clipboard repeats to the end of the pattern
    app.set_cursor(0, 0, 0, 20);
    app.do_action(Action::PasteFlood);
    let last = app.lines_in_pattern() - 1;
    assert_eq!(note_at(&app, 0, 20), 60);
    assert_eq!(note_at(&app, 0, 22), 60);
    assert_eq!(note_at(&app, 0, 21), NOTE_EMPTY);
    assert_eq!(note_at(&app, 0, last - 1), 60, "an even distance from the start gets a copy");
    app.undo();
    // flood inside a selection stops at its end
    app.set_cursor(0, 0, 0, 40);
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.do_action(Action::PasteFlood);
    assert_eq!((note_at(&app, 0, 40), note_at(&app, 0, 42)), (60, 60));
    assert_eq!(note_at(&app, 0, 44), NOTE_EMPTY, "stopped at the selection end");
    // edit mode off and empty clipboard are explained
    app.edit_mode = false;
    app.do_action(Action::PasteMix);
    assert!(app.status.contains("edit mode"));
    let mut fresh = ed();
    fresh.edit_mode = true;
    fresh.do_action(Action::PasteFlood);
    assert!(fresh.status.contains("clipboard is empty"));
    assert_eq!(
        Action::from_key(KeyEvent { code: KeyCode::Char('v'), ctrl: true, alt: false, shift: true }),
        Some(Action::PasteMix)
    );
    assert_eq!(
        Action::from_key(KeyEvent { code: KeyCode::Char('v'), ctrl: true, alt: true, shift: false }),
        Some(Action::PasteFlood)
    );
}

#[test]
fn cut_clears_and_transpose_shifts() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 48);
    put(&mut app, 1, 50);
    app.set_cursor(0, 0, 0, 0);
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.do_action(Action::TransposeUp);
    assert_eq!((note_at(&app, 0, 0), note_at(&app, 0, 1)), (49, 51));
    app.do_action(Action::TransposeOctaveDown);
    assert_eq!((note_at(&app, 0, 0), note_at(&app, 0, 1)), (37, 39));
    app.do_action(Action::Cut);
    assert_eq!(note_at(&app, 0, 0), NOTE_EMPTY);
    assert_eq!(app.clipboard.as_ref().unwrap().rows.len(), 2);
}

#[test]
fn reverse_and_interpolate() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 10);
    put(&mut app, 2, 20);
    app.set_cursor(0, 0, 0, 0);
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.on_key(KeyEvent::shift(KeyCode::Down));
    app.do_action(Action::Reverse);
    assert_eq!((note_at(&app, 0, 0), note_at(&app, 0, 2)), (20, 10));
    // interpolate volume 0x00..0x40 over three lines
    app.set_cursor(0, 0, 0, 0);
    app.edit(|l| l.notes[0].volume = 0);
    app.set_cursor(0, 0, 0, 2);
    app.edit(|l| l.notes[0].volume = 0x40);
    app.set_cursor(0, 0, 0, 0);
    for _ in 0..2 {
        app.on_key(KeyEvent::shift(KeyCode::Down));
    }
    app.do_action(Action::Interpolate);
    assert_eq!(app.song.patterns[0].line(0, 1).unwrap().notes[0].volume, 0x20);
}

#[test]
fn insert_and_delete_line() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 10);
    put(&mut app, 1, 20);
    app.set_cursor(0, 0, 0, 1);
    app.do_action(Action::InsertLine);
    assert_eq!((note_at(&app, 0, 0), note_at(&app, 0, 1), note_at(&app, 0, 2)), (10, NOTE_EMPTY, 20));
    app.do_action(Action::DeleteLine);
    assert_eq!((note_at(&app, 0, 0), note_at(&app, 0, 1)), (10, 20));
}

#[test]
fn sequence_editing() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 33);
    app.do_action(Action::SeqClone);
    assert_eq!(app.song.sequence, vec![0, 1]);
    assert_eq!(app.seq_pos, 1);
    app.do_action(Action::SeqInsert);
    assert_eq!(app.song.sequence, vec![0, 1, 2]);
    app.do_action(Action::SeqMoveUp);
    assert_eq!(app.song.sequence, vec![0, 2, 1]);
    assert_eq!(app.seq_pos, 1);
    app.do_action(Action::SeqDelete);
    assert_eq!(app.song.sequence, vec![0, 1]);
    app.undo();
    assert_eq!(app.song.sequence, vec![0, 2, 1]);
    app.set_pattern_lines(32);
    assert_eq!(app.lines_in_pattern(), 32);
}

#[test]
fn track_and_column_ops() {
    let mut app = ed();
    let n = app.song.tracks.len();
    app.do_action(Action::TrackAdd);
    assert_eq!(app.song.tracks.len(), n + 1);
    assert_eq!(app.song.tracks[n - 2].kind, trakr_core::TrackKind::Sequencer);
    assert_eq!(app.song.tracks[n - 1].kind, trakr_core::TrackKind::Sequencer);
    assert_eq!(app.song.tracks[n].kind, trakr_core::TrackKind::Master);
    app.do_action(Action::NoteColumnAdd);
    assert_eq!(app.cols(app.cursor.track).0, 2);
    app.do_action(Action::TrackRemove);
    assert_eq!(app.song.tracks.len(), n);
    app.cursor.track = n - 1; // master cannot be removed
    app.do_action(Action::TrackRemove);
    assert_eq!(app.song.tracks.len(), n);
}

#[test]
fn clear_selection_needs_edit_mode() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 10);
    app.edit_mode = false;
    app.set_cursor(0, 0, 0, 0);
    app.do_action(Action::Clear);
    assert_eq!(note_at(&app, 0, 0), 10);
}

#[test]
fn instrument_remove_remaps_notes() {
    let mut app = Editor::new(Song::demo(), None, None);
    app.do_action(Action::InstrumentRemove); // removes instrument 0
    assert!(app.song.instruments.is_empty());
    assert_eq!(app.song.patterns[0].line(0, 0).unwrap().notes[0].instrument, EMPTY);
    app.undo();
    assert_eq!(app.song.instruments.len(), 1);
    assert_eq!(app.song.patterns[0].line(0, 0).unwrap().notes[0].instrument, 0);
}

#[test]
fn edit_sample_is_undoable() {
    let mut app = Editor::new(Song::demo(), None, None);
    let before = app.song.instruments[0].samples[0].data[1000];
    app.edit_sample(0, |s| s.reverse());
    assert_ne!(app.song.instruments[0].samples[0].data[1000], before);
    app.undo();
    assert_eq!(app.song.instruments[0].samples[0].data[1000], before);
}

struct FakeInput {
    chunks: Vec<Vec<f32>>,
}

impl InputRecorder for FakeInput {
    fn sample_rate(&self) -> u32 {
        8000
    }
    fn channels(&self) -> u8 {
        1
    }
    fn drain(&mut self, out: &mut Vec<f32>) {
        if !self.chunks.is_empty() {
            out.extend(self.chunks.remove(0));
        }
    }
}

fn recording_editor() -> Editor {
    let mut app = Editor::new(Song::default(), None, None);
    app.set_recorder_factory(Box::new(|_| {
        Ok(Box::new(FakeInput { chunks: vec![vec![0.1; 100], vec![-0.5; 100]] }) as Box<dyn InputRecorder>)
    }));
    app
}

#[test]
fn record_creates_an_instrument() {
    let mut app = recording_editor();
    assert!(app.can_record() && !app.is_recording());
    app.on_key(KeyEvent::ctrl('r'));
    assert!(app.is_recording());
    app.tick();
    assert!(app.rec_peak < 0.2);
    app.tick();
    assert!((app.rec_peak - 0.5).abs() < 1e-6);
    app.on_key(KeyEvent::ctrl('r'));
    assert!(!app.is_recording());
    assert_eq!(app.song.instruments.len(), 1);
    let s = &app.song.instruments[0].samples[0];
    assert_eq!((s.sample_rate, s.channels, s.frames()), (8000, 1, 200));
    app.undo();
    assert!(app.song.instruments.is_empty());
}

#[test]
fn record_without_input_explains() {
    let mut app = ed();
    app.do_action(Action::RecordToggle);
    assert!(!app.is_recording());
    assert!(app.status.contains("no audio input"));
}

#[test]
fn sample_clipboard_and_slices() {
    let mut app = Editor::new(Song::demo(), None, None);
    let n = app.current_sample().unwrap().frames();
    app.sample_copy(0, 100);
    app.sample_paste(n);
    assert_eq!(app.current_sample().unwrap().frames(), n + 100);
    app.sample_cut(0, 50);
    assert_eq!(app.current_sample().unwrap().frames(), n + 50);
    app.sample_crop(0, 1000);
    assert_eq!(app.current_sample().unwrap().frames(), 1000);
    app.edit_sample(0, |s| s.slice_evenly(4));
    app.sample_split_slices();
    let inst = app.current_instrument().unwrap();
    assert_eq!(inst.samples.len(), 4);
    assert_eq!(inst.samples[3].key_range, (51, 51));
    // playing C-4+3 selects the 4th slice
    assert!(inst.sample_for(51).is_some_and(|s| s.name.ends_with("04")));
    app.undo();
    assert_eq!(app.current_instrument().unwrap().samples.len(), 1);
}

#[test]
fn render_to_sample_bounces_the_song() {
    let mut app = Editor::new(Song::demo(), None, None);
    app.render_to_sample();
    assert_eq!(app.song.instruments.len(), 2);
    let s = app.current_sample().unwrap();
    assert_eq!((s.channels, s.sample_rate), (2, 44_100));
    assert!(s.peak(0, s.frames()) > 0.1);
}

#[test]
fn export_sample_writes_a_readable_wav() {
    let mut app = Editor::new(Song::demo(), None, None);
    let path = std::env::temp_dir().join("trakr_export_test.wav");
    app.export_sample(&path, 0, 0, ExportFormat::wav(16));
    let back = trakr_sample::decode_file(&path).unwrap();
    assert_eq!(back.frames(), app.current_sample().unwrap().frames());
    let _ = std::fs::remove_file(path);
}

#[test]
fn fade_block_writes_volume_ramp() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 48);
    app.set_cursor(0, 0, 0, 0);
    for _ in 0..4 {
        app.on_key(KeyEvent::shift(KeyCode::Down));
    }
    app.do_action(Action::FadeInBlock);
    let vol = |l: u16| app.song.patterns[0].line(0, l).map_or(0xFF, |x| x.notes[0].volume);
    assert_eq!((vol(0), vol(2), vol(4)), (0, 0x40, 0x80));
    app.do_action(Action::FadeOutBlock);
    let vol = |l: u16| app.song.patterns[0].line(0, l).map_or(0xFF, |x| x.notes[0].volume);
    assert_eq!((vol(0), vol(2), vol(4)), (0x80, 0x40, 0));
    app.undo();
    let vol = |l: u16| app.song.patterns[0].line(0, l).map_or(0xFF, |x| x.notes[0].volume);
    assert_eq!(vol(4), 0x80);
}

#[test]
fn mixer_and_loop_range() {
    let mut app = ed();
    app.set_track_volume(1, 0.5);
    app.set_track_volume(1, 0.7); // drag: one undo step
    assert_eq!(app.song.tracks[1].volume, 0.7);
    app.undo();
    assert_eq!(app.song.tracks[1].volume, 1.0);
    app.track_toggle_solo(2);
    assert!(app.song.tracks[2].solo);
    app.track_toggle_mute_at(3);
    assert_eq!(app.song.tracks[3].mute, trakr_core::MuteState::Muted);

    app.do_action(Action::SeqInsert);
    app.do_action(Action::SeqInsert);
    app.seq_goto(1);
    app.do_action(Action::LoopStart);
    app.seq_goto(2);
    app.do_action(Action::LoopEnd);
    assert_eq!(app.loop_range(), Some((1, 2)));
    app.do_action(Action::SeqDelete);
    assert_eq!(app.loop_range(), Some((1, 1)));
    app.do_action(Action::LoopClear);
    assert_eq!(app.loop_range(), None);
}

#[test]
fn edit_instrument_envelope_is_undoable() {
    let mut app = Editor::new(Song::demo(), None, None);
    app.edit_instrument("env", |i| {
        *std::sync::Arc::make_mut(&mut i.volume_env) = trakr_core::Envelope::adsr(0.1, 0.1, 0.5, 0.2);
    });
    assert!(app.current_instrument().unwrap().volume_env.enabled);
    app.undo();
    assert!(!app.current_instrument().unwrap().volume_env.enabled);
}

fn engine_editor() -> (trakr_engine::Engine, Editor) {
    let (engine, handle) = trakr_engine::create(44_100);
    (engine, Editor::new(Song::demo(), None, Some(handle)))
}

fn render_peak(engine: &mut trakr_engine::Engine, ed: &mut Editor, frames: usize) -> f32 {
    ed.tick();
    let mut out = vec![0.0; frames * 2];
    for chunk in out.chunks_mut(512) {
        engine.render(chunk);
    }
    out.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn devices_reach_the_engine_and_undo() {
    let (mut engine, mut app) = engine_editor();
    assert!(app.device_add(0, "native.gainer"));
    app.device_set_param(0, 0, 0, -60.0);
    app.do_action(Action::PlaySong);
    assert!(render_peak(&mut engine, &mut app, 20_000) < 0.01, "gainer at -60 dB should silence the demo");
    app.undo(); // the param change
    app.undo(); // the device itself
    assert!(app.song.tracks[0].devices.is_empty());
    assert!(render_peak(&mut engine, &mut app, 20_000) > 0.1, "chain should be gone after undo");
}

#[test]
fn device_ops_and_automation_lanes() {
    let mut app = ed();
    assert!(!app.device_add(0, "nope"));
    app.device_add(0, "native.filter");
    app.device_add(0, "native.delay");
    app.device_move(0, 0, 1);
    assert_eq!(app.song.tracks[0].devices[0].id, "native.delay");
    app.device_toggle_bypass(0, 0);
    assert!(app.song.tracks[0].devices[0].bypass);
    app.device_set_param(0, 1, 1, 99_999.0); // clamped to the catalogue range
    assert_eq!(app.song.tracks[0].devices[1].params[1], 20_000.0);

    use trakr_core::AutoTarget;
    let target = AutoTarget::Device { device: 1, param: 1 };
    app.automation_edit("auto", 0, target, |a| {
        a.set_point(0.0, 0.1);
        a.set_point(8.0, 0.9);
    });
    assert_eq!(app.automation_lane(0, target).unwrap().points.len(), 2);
    app.automation_edit("auto2", 0, target, |a| a.points.clear());
    assert!(app.automation_lane(0, target).is_none());
    // lanes follow their device when an earlier device is removed
    app.automation_edit("auto3", 0, target, |a| {
        a.set_point(0.0, 0.5);
    });
    app.device_remove(0, 0);
    let shifted = AutoTarget::Device { device: 0, param: 1 };
    assert!(app.automation_lane(0, shifted).is_some());
    app.undo();
    assert!(app.automation_lane(0, target).is_some());
}

#[test]
fn track_limit_is_enforced() {
    let mut app = ed();
    for _ in 0..(trakr_engine::MAX_METER_TRACKS + 20) {
        app.do_action(Action::TrackAdd);
    }
    assert_eq!(app.song.tracks.len(), trakr_engine::MAX_METER_TRACKS);
    assert!(app.status.contains("limit"));
}

struct FakeMidi {
    events: Vec<trakr_midi::MidiEvent>,
}

impl trakr_midi::MidiInput for FakeMidi {
    fn poll(&mut self, out: &mut Vec<trakr_midi::MidiEvent>) {
        out.append(&mut self.events);
    }
    fn port_name(&self) -> &str {
        "fake keyboard"
    }
}

struct FakeBackend;

impl trakr_midi::MidiBackend for FakeBackend {
    fn ports(&mut self) -> Vec<String> {
        vec!["fake keyboard".into()]
    }
    fn open(&mut self, _port: usize) -> anyhow::Result<Box<dyn trakr_midi::MidiInput>> {
        use trakr_midi::MidiEvent::*;
        Ok(Box::new(FakeMidi {
            events: vec![
                NoteOn { channel: 0, note: 60, velocity: 127 },
                NoteOff { channel: 0, note: 60 },
                NoteOn { channel: 0, note: 64, velocity: 64 },
            ],
        }))
    }
}

#[test]
fn midi_keyboard_records_notes_in_edit_mode() {
    let mut app = ed();
    assert!(!app.has_midi() && app.midi_ports().is_empty());
    app.set_midi_backend(Box::new(FakeBackend));
    assert_eq!(app.midi_ports(), vec!["fake keyboard".to_string()]);
    app.edit_mode = true;
    app.midi_connect(0);
    assert_eq!(app.midi_port_name(), Some("fake keyboard"));
    app.tick();
    let l0 = app.song.patterns[0].line(0, 0).unwrap().notes[0];
    assert_eq!((l0.note, l0.instrument, l0.volume), (48, 0, 0x80), "MIDI 60 is C-4");
    let l1 = app.song.patterns[0].line(0, 1).unwrap().notes[0];
    assert_eq!((l1.note, l1.volume), (52, 0x40));
    assert_eq!(app.cursor.line, 2);
    app.midi_disconnect();
    assert!(app.midi_port_name().is_none());
}

#[test]
fn midi_does_not_write_when_edit_mode_is_off_and_maps_controllers() {
    use trakr_midi::MidiEvent::*;
    let mut app = ed();
    app.handle_midi(NoteOn { channel: 0, note: 60, velocity: 100 });
    assert!(app.song.patterns[0].line(0, 0).is_none());
    app.handle_midi(Cc { channel: 0, controller: 7, value: 50 });
    assert_eq!(app.song.tracks[0].volume, 0.5);
    app.handle_midi(Cc { channel: 0, controller: 10, value: 127 });
    assert_eq!(app.song.tracks[0].panning, 1.0);
    app.handle_midi(Cc { channel: 0, controller: 10, value: 64 });
    assert_eq!(app.song.tracks[0].panning, 0.0);
    // transport messages must not panic without an engine
    app.handle_midi(Start);
    app.handle_midi(Continue);
    app.handle_midi(Stop);
}

#[test]
fn midi_note_on_plays_through_the_engine() {
    let (mut engine, mut app) = engine_editor();
    app.handle_midi(trakr_midi::MidiEvent::NoteOn { channel: 0, note: 60, velocity: 127 });
    assert!(render_peak(&mut engine, &mut app, 4000) > 0.1, "keyboard note should be audible");
}

#[test]
fn groups_sends_and_index_bookkeeping() {
    use trakr_core::{AutoTarget, TrackKind};
    let mut app = ed();
    app.do_action(Action::TrackAddGroup);
    let g = app.cursor.track;
    assert_eq!(app.song.tracks[g].kind, TrackKind::Group);
    assert_eq!(app.song.tracks[g + 1].kind, TrackKind::Master, "groups sit just before the master");
    app.do_action(Action::TrackAddSend);
    let s = app.cursor.track;
    assert_eq!(s, app.song.tracks.len() - 1);
    assert_eq!(app.song.tracks[s].kind, TrackKind::Send);

    app.set_track_group(0, Some(g));
    app.set_send(0, s, 0.5, true);
    assert_eq!(app.song.tracks[0].group, Some(g));
    assert_eq!(app.song.tracks[0].sends[0].target, s);
    // routing rules
    app.set_track_group(0, Some(1));
    assert_eq!(app.song.tracks[0].group, Some(g), "only group tracks accept routing");
    app.do_action(Action::TrackAddGroup);
    let g2 = app.cursor.track;
    app.set_track_group(g, Some(g2));
    app.set_track_group(g2, Some(g));
    assert_eq!(app.song.tracks[g2].group, None, "loops are refused");

    // an automation lane on the master follows it when a sequencer track is added
    let master = app.song.tracks.iter().position(|t| t.kind == TrackKind::Master).unwrap();
    app.automation_edit("a", master, AutoTarget::TrackVolume, |a| {
        a.set_point(0.0, 0.5);
    });
    app.set_cursor(0, 0, 0, 0);
    app.do_action(Action::TrackAdd);
    let new_master = app.song.tracks.iter().position(|t| t.kind == TrackKind::Master).unwrap();
    assert_eq!(new_master, master + 1);
    assert!(app.automation_lane(new_master, AutoTarget::TrackVolume).is_some());
    let g_now = app.song.tracks.iter().position(|t| t.name == "Group 01").unwrap();
    assert_eq!(app.song.tracks[0].group, Some(g_now), "group routing follows its group");

    // removing the group clears the routing; removing the send drops the feed
    app.cursor.track = g_now;
    app.do_action(Action::TrackRemove);
    assert_eq!(app.song.tracks[0].group, None);
    let s_now = app.song.tracks.iter().position(|t| t.kind == TrackKind::Send).unwrap();
    app.cursor.track = s_now;
    app.do_action(Action::TrackRemove);
    assert!(app.song.tracks[0].sends.is_empty());
    app.undo();
    assert_eq!(app.song.tracks[0].sends.len(), 1);
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("trakr_test_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn prefs_and_recent_files_roundtrip() {
    let dir = temp_dir("prefs");
    let mut app = ed();
    app.set_config_dir(Some(dir.join("cfg")));
    app.octave = 6;
    app.edit_step = 4;
    app.follow = false;
    let song_path = dir.join("a.trkr");
    app.save_as(song_path.clone());
    assert_eq!(app.recent_files(), std::slice::from_ref(&song_path));
    app.save_prefs();
    let mut fresh = ed();
    fresh.set_config_dir(Some(dir.join("cfg")));
    fresh.load_prefs();
    assert_eq!((fresh.octave, fresh.edit_step, fresh.follow), (6, 4, false));
    assert_eq!(fresh.recent_files(), &[song_path]);
    // corrupt prefs are ignored, not fatal
    std::fs::write(dir.join("cfg/prefs.json"), "not json").unwrap();
    let mut again = ed();
    again.set_config_dir(Some(dir.join("cfg")));
    again.load_prefs();
    assert_eq!(again.octave, 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn autosave_is_written_recovered_and_cleared() {
    let dir = temp_dir("autosave");
    let path = dir.join("song.trkr");
    let mut app = ed();
    app.save_as(path.clone());
    app.set_autosave_interval(std::time::Duration::from_millis(1));
    assert!(!app.recovery_available());
    app.edit_mode = true;
    put(&mut app, 3, 40);
    std::thread::sleep(std::time::Duration::from_millis(5));
    app.tick();
    let auto = app.autosave_path().unwrap();
    assert!(auto.exists(), "autosave should exist after an edit");
    // a "crash": a fresh editor opening the same project sees the newer autosave
    let mut after_crash = ed();
    after_crash.open(&path);
    assert!(after_crash.recovery_available());
    assert_eq!(note_at(&after_crash, 0, 3), NOTE_EMPTY);
    after_crash.recover_autosave();
    assert_eq!(note_at(&after_crash, 0, 3), 40);
    assert!(after_crash.dirty);
    // saving clears the autosave
    after_crash.save();
    assert!(!auto.exists());
    assert!(!after_crash.recovery_available());
    // untouched songs never autosave
    let mut clean = ed();
    clean.open(&path);
    clean.set_autosave_interval(std::time::Duration::from_millis(1));
    clean.tick();
    assert!(!clean.autosave_path().unwrap().exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// Compile-time check that `Action::ALL` lists every variant (adding a variant breaks this match).
#[test]
fn action_all_is_complete() {
    fn index(a: Action) -> usize {
        use Action::*;
        match a {
            New => 0,
            Save => 1,
            Quit => 2,
            Undo => 3,
            Redo => 4,
            Cut => 5,
            Copy => 6,
            Paste => 7,
            Clear => 8,
            SelectAll => 9,
            SelectNone => 10,
            ToggleEdit => 11,
            TransposeUp => 12,
            TransposeDown => 13,
            TransposeOctaveUp => 14,
            TransposeOctaveDown => 15,
            Reverse => 16,
            Interpolate => 17,
            InsertLine => 18,
            DeleteLine => 19,
            PlaySong => 20,
            PlayPattern => 21,
            PlayFromCursor => 22,
            Stop => 23,
            ToggleFollow => 24,
            SeqPrev => 25,
            SeqNext => 26,
            SeqInsert => 27,
            SeqClone => 28,
            SeqDelete => 29,
            SeqMoveUp => 30,
            SeqMoveDown => 31,
            PatternPrev => 32,
            PatternNext => 33,
            TrackAdd => 34,
            TrackAddGroup => 35,
            TrackAddSend => 36,
            TrackRemove => 37,
            TrackMute => 38,
            NoteColumnAdd => 39,
            NoteColumnRemove => 40,
            EffectColumnAdd => 41,
            EffectColumnRemove => 42,
            OctaveDown => 43,
            OctaveUp => 44,
            StepDown => 45,
            StepUp => 46,
            InstrumentPrev => 47,
            InstrumentNext => 48,
            InstrumentRemove => 49,
            RecordToggle => 50,
            RenderToSample => 51,
            FadeInBlock => 52,
            FadeOutBlock => 53,
            TrackSolo => 54,
            LoopStart => 55,
            LoopEnd => 56,
            LoopClear => 57,
            Randomize => 58,
            ExpandBlock => 59,
            ShrinkBlock => 60,
            RepeatBlock => 61,
            OpenPalette => 62,
            OpenFind => 63,
            OpenPreferences => 64,
            PlayBlock => 65,
            SectionToggle => 66,
            PlaySection => 67,
            SlotMuteToggle => 68,
            PasteMix => 69,
            PasteFlood => 70,
        }
    }
    for (i, a) in Action::ALL.iter().enumerate() {
        assert_eq!(index(*a), i, "{a:?} is out of order in Action::ALL");
    }
    assert_eq!(Action::ALL.len(), 71);
    for a in Action::ALL {
        assert!(!a.label().is_empty());
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn below(&mut self, n: usize) -> usize {
        self.next() as usize % n.max(1)
    }
}

fn check_invariants(app: &Editor, step: usize, what: &str) {
    let s = &app.song;
    let nt = s.tracks.len();
    assert!(nt >= 2, "step {step} ({what}): lost tracks");
    assert!(app.cursor.track < nt, "step {step} ({what}): cursor track {} of {nt}", app.cursor.track);
    let (n, f) = app.cols(app.cursor.track);
    assert!(app.cursor.col < n + f, "step {step} ({what}): cursor col");
    assert!(!s.sequence.is_empty() && app.seq_pos < s.sequence.len(), "step {step} ({what}): seq_pos");
    assert!(s.sequence.iter().all(|&p| p < s.patterns.len()), "step {step} ({what}): dangling slot");
    assert!(app.cursor.line < app.lines_in_pattern(), "step {step} ({what}): cursor line");
    for (i, t) in s.tracks.iter().enumerate() {
        if let Some(g) = t.group {
            assert!(
                g < nt && s.tracks[g].kind == trakr_core::TrackKind::Group && g != i,
                "step {step} ({what}): bad group"
            );
        }
        for send in &t.sends {
            assert!(
                send.target < nt && s.tracks[send.target].kind == trakr_core::TrackKind::Send,
                "step {step} ({what}): bad send"
            );
        }
    }
    for p in &s.patterns {
        assert!(
            p.automation.iter().all(|a| a.track < nt),
            "step {step} ({what}): automation track out of range"
        );
        assert!(p.number_of_lines >= 1 && p.number_of_lines <= trakr_core::MAX_LINES);
    }
    if let Some((a, b)) = s.loop_range {
        assert!(a <= b && b < s.sequence.len(), "step {step} ({what}): loop range {a}..{b}");
    }
    assert!((app.instrument as usize) < 255);
    assert!(s.slot_meta.len() <= s.sequence.len(), "step {step} ({what}): slot meta outgrew the sequence");
    assert!(s.groove.len() <= trakr_core::song::MAX_GROOVE && s.groove.iter().all(|g| g.is_finite()));
}

fn fuzz_editor(seed: u64) {
    let mut rng = Lcg(seed);
    let mut app = Editor::new(Song::showcase(), None, None);
    let keys = [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Insert,
        KeyCode::Esc,
    ];
    let chars: Vec<char> = "zsxdcvgbhnjmqwertyuiop234567890`abcdef ,./;[]-=".chars().collect();
    for step in 0..12_000 {
        let what: String;
        match rng.below(16) {
            0..=4 => {
                let k = if rng.below(2) == 0 {
                    KeyCode::Char(chars[rng.below(chars.len())])
                } else {
                    keys[rng.below(keys.len())]
                };
                let ev = KeyEvent {
                    code: k,
                    ctrl: rng.below(8) == 0,
                    alt: rng.below(8) == 0,
                    shift: rng.below(4) == 0,
                };
                // Ctrl-S writes the whole showcase song to disk and Ctrl-Q quits: not what this test is about.
                if ev.ctrl && matches!(ev.code, KeyCode::Char('s' | 'q')) {
                    continue;
                }
                what = format!("{ev:?}");
                app.on_key(ev);
            }
            5..=7 => {
                let a = Action::ALL[rng.below(Action::ALL.len())];
                // These touch the file system or audio devices; everything else is fair game.
                if matches!(
                    a,
                    Action::Save | Action::Quit | Action::RecordToggle | Action::RenderToSample | Action::New
                ) {
                    continue;
                }
                what = format!("{a:?}");
                app.do_action(a);
            }
            8 => {
                what = "set_cursor".into();
                app.set_cursor(rng.below(40), rng.below(30), rng.below(12), rng.below(600) as u16);
            }
            9 => {
                what = "pattern lines".into();
                app.set_pattern_lines(rng.below(600) as u16);
            }
            10 => {
                what = "devices".into();
                let t = rng.below(app.song.tracks.len() + 2);
                match rng.below(4) {
                    0 => {
                        let d = &trakr_core::DEVICES[rng.below(trakr_core::DEVICES.len())];
                        app.device_add(t, d.id);
                    }
                    1 => app.device_remove(t, rng.below(4)),
                    2 => app.device_move(t, rng.below(4), rng.below(3) as i32 - 1),
                    _ => app.device_set_param(
                        t,
                        rng.below(4),
                        rng.below(10),
                        rng.below(100_000) as f32 - 50_000.0,
                    ),
                }
            }
            11 => {
                what = "routing".into();
                let n = app.song.tracks.len();
                match rng.below(3) {
                    0 => app.set_track_group(
                        rng.below(n + 1),
                        if rng.below(3) == 0 { None } else { Some(rng.below(n + 1)) },
                    ),
                    1 => app.set_send(
                        rng.below(n + 1),
                        rng.below(n + 1),
                        rng.below(150) as f32 / 100.0,
                        rng.below(2) == 0,
                    ),
                    _ => app.set_track_volume(rng.below(n + 1), rng.below(500) as f32 / 100.0),
                }
            }
            12 => {
                what = "automation / loop".into();
                let n = app.song.tracks.len();
                let target = if rng.below(2) == 0 {
                    trakr_core::AutoTarget::TrackVolume
                } else {
                    trakr_core::AutoTarget::Device { device: rng.below(3), param: rng.below(5) }
                };
                if rng.below(3) == 0 {
                    app.set_loop_edge(rng.below(2) == 0);
                } else {
                    let (l, v) = (rng.below(200) as f32 / 3.0, rng.below(100) as f32 / 100.0);
                    app.automation_edit("fuzz", rng.below(n), target, |a| {
                        a.set_point(l, v);
                    });
                }
            }
            14 => {
                what = "structure".into();
                match rng.below(10) {
                    6 => app.track_set_sub_column(
                        rng.below(app.song.tracks.len() + 1),
                        rng.below(4),
                        rng.below(2) == 0,
                    ),
                    7 => app.track_set_color(
                        rng.below(app.song.tracks.len() + 1),
                        Some([rng.below(256) as u8, 1, 2]),
                    ),
                    8 => {
                        let keep = app.instrument;
                        app.instrument = rng.below(app.song.instruments.len() + 1) as u8;
                        match rng.below(5) {
                            0 => {
                                let d = &trakr_core::DEVICES[rng.below(trakr_core::DEVICES.len())];
                                app.instrument_fx_add(d.id);
                            }
                            1 => app.instrument_fx_remove(rng.below(4)),
                            2 => app.instrument_fx_move(rng.below(4), rng.below(3) as i32 - 1),
                            3 => app.instrument_fx_toggle_bypass(rng.below(4)),
                            _ => app.instrument_fx_set_param(
                                rng.below(4),
                                rng.below(6),
                                rng.below(10_000) as f32 - 5000.0,
                            ),
                        }
                        app.instrument = keep;
                    }
                    9 => {
                        let n = app.song.tracks.len();
                        let tg = if rng.below(2) == 0 {
                            trakr_core::AutoTarget::Bpm
                        } else {
                            trakr_core::AutoTarget::TrackPan
                        };
                        let (a, b) = (rng.below(40) as f32, rng.below(40) as f32);
                        match rng.below(6) {
                            0 => {
                                app.automation_edit("fuzz", rng.below(n), tg, |l| {
                                    l.set_point(a, 0.3);
                                    l.set_point(b, 0.8);
                                });
                            }
                            1 => {
                                app.automation_copy(0, tg, a.min(b), a.max(b));
                                app.automation_paste(0, tg, rng.below(60) as f32);
                            }
                            2 => app.automation_flip(rng.below(n), tg, a.min(b), a.max(b)),
                            3 => app.automation_shift(
                                rng.below(n),
                                tg,
                                a.min(b),
                                a.max(b),
                                rng.below(9) as f32 - 4.0,
                            ),
                            4 => app.automation_scale(
                                rng.below(n),
                                tg,
                                a.min(b),
                                a.max(b),
                                rng.below(40) as f32 / 10.0 - 1.0,
                            ),
                            _ => {
                                let all = trakr_core::AutoCurve::ALL;
                                app.automation_set_curve(rng.below(n), tg, all[rng.below(all.len())])
                            }
                        }
                    }
                    0 => app.slot_toggle_mute(rng.below(8), rng.below(40)),
                    1 => {
                        app.section_set(rng.below(8), if rng.below(3) == 0 { None } else { Some("s".into()) })
                    }
                    2 => app.play_section(),
                    3 => app.play_block(),
                    4 => app.set_groove(
                        (0..rng.below(12)).map(|_| rng.below(300) as f32 / 100.0 - 1.5).collect(),
                    ),
                    _ => app.section_toggle_here(),
                }
            }
            13 => {
                what = "phrases".into();
                let keep = app.instrument;
                app.instrument = rng.below(app.song.instruments.len() + 1) as u8;
                match rng.below(7) {
                    0 => app.phrase_add(),
                    1 => app.phrase_remove(),
                    2 => app.phrase_duplicate(),
                    3 => app.phrase_select(rng.below(4)),
                    4 => app.phrase_set_cursor(rng.below(20), rng.below(12), rng.below(700) as u16),
                    5 => app.edit_phrase("fuzz", |p| {
                        p.number_of_lines = rng.below(800) as u16;
                        p.note_columns = rng.below(30) as u8;
                        p.effect_columns = rng.below(30) as u8;
                    }),
                    _ => {
                        let c = chars[rng.below(chars.len())];
                        app.phrase_key(KeyEvent::new(KeyCode::Char(c)));
                        app.phrase_key(KeyEvent::new(keys[rng.below(keys.len())]));
                    }
                }
                app.instrument = keep;
            }
            _ => {
                what = "undo/redo".into();
                for _ in 0..1 + rng.below(4) {
                    if rng.below(3) == 0 {
                        app.redo();
                    } else {
                        app.undo();
                    }
                }
            }
        }
        check_invariants(&app, step, &what);
    }
    // everything undoable: unwind completely and make sure it still holds together
    while app.can_undo() {
        app.undo();
    }
    check_invariants(&app, usize::MAX, "full undo");
}

#[test]
fn random_editing_keeps_the_song_consistent() {
    for seed in [0xBADC0DE, 1, 2, 3, 0xFEED, 77] {
        fuzz_editor(seed);
    }
}

#[test]
fn default_keymap_matches_the_documented_shortcuts() {
    let app = ed();
    for &a in Action::ALL {
        let text = a.shortcut();
        if text.is_empty() {
            continue;
        }
        let k = KeyEvent::from_text(text).unwrap_or_else(|| panic!("{a:?}: cannot parse shortcut {text}"));
        assert_eq!(app.action_for_key(k), Some(a), "{a:?} ({text})");
        assert_eq!(Action::from_key(k), Some(a), "built-in table disagrees for {a:?}");
        assert_eq!(app.shortcut_for(a), text);
    }
    let redo = KeyEvent { code: KeyCode::Char('z'), ctrl: true, alt: false, shift: true };
    assert_eq!(app.action_for_key(redo), Some(Action::Redo));
}

#[test]
fn rebinding_changes_behavior_and_validates() {
    let mut app = ed();
    app.edit_mode = true;
    put(&mut app, 0, 40);
    assert!(app.set_binding(Action::Undo, KeyEvent::from_text("F12")).is_ok());
    assert_eq!(app.shortcut_for(Action::Undo), "F12");
    app.on_key(KeyEvent::ctrl('z'));
    assert_eq!(note_at(&app, 0, 0), 40, "the old shortcut no longer works");
    app.on_key(KeyEvent::new(KeyCode::F(12)));
    assert_eq!(note_at(&app, 0, 0), NOTE_EMPTY, "the new one does");
    assert!(app.set_binding(Action::Redo, KeyEvent::from_text("F12")).unwrap_err().contains("Undo"));
    assert!(app.set_binding(Action::Redo, KeyEvent::from_text("Z")).is_err(), "typing keys are reserved");
    assert!(app.set_binding(Action::Redo, KeyEvent::from_text("Up")).is_err(), "navigation is reserved");
    assert!(app
        .set_binding(Action::Redo, KeyEvent::from_text("Ctrl-Up"))
        .unwrap_err()
        .contains("Previous slot"));
    app.set_binding(Action::Redo, None).unwrap();
    assert_eq!(app.shortcut_for(Action::Redo), "");
    assert_eq!(app.action_for_key(KeyEvent::ctrl('y')), None);
    app.reset_binding(Action::Redo);
    assert_eq!(app.shortcut_for(Action::Redo), "Ctrl-Y");
    app.reset_all_bindings();
    assert_eq!(app.shortcut_for(Action::Undo), "Ctrl-Z");
}

#[test]
fn custom_keymap_is_saved_with_the_preferences() {
    let dir = temp_dir("keymap");
    let mut app = ed();
    app.set_config_dir(Some(dir.clone()));
    app.set_binding(Action::TransposeUp, KeyEvent::from_text("Ctrl-Shift-U")).unwrap();
    let mut fresh = ed();
    fresh.set_config_dir(Some(dir.clone()));
    fresh.load_prefs();
    assert_eq!(fresh.shortcut_for(Action::TransposeUp), "Ctrl-Shift-U");
    assert_eq!(
        fresh.shortcut_for(Action::TransposeDown),
        "Alt-Down",
        "other bindings stay at their defaults"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_xrns_writes_a_readable_archive() {
    let mut ed = Editor::new(Song::demo(), None, None);
    let dir = std::env::temp_dir().join(format!("trakr-xrns-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.xrns");
    ed.export_xrns(&path);
    assert!(ed.status.starts_with("exported"), "{}", ed.status);
    let back = trakr_xrns::import_path(&path).unwrap();
    assert_eq!(back.tracks.len(), ed.song.tracks.len());
    let _ = std::fs::remove_dir_all(&dir);
}

fn phrase_ed() -> Editor {
    let mut e = Editor::new(Song::demo(), None, None);
    e.instrument = 0;
    e.phrase_add();
    e.edit_mode = true;
    e
}

#[test]
fn phrase_add_enables_the_mode_and_selects_it() {
    let e = phrase_ed();
    let inst = &e.song.instruments[0];
    assert_eq!(inst.phrases.len(), 1);
    assert_eq!(inst.phrase_mode, trakr_core::PhraseMode::Keymap);
    assert_eq!(e.phrase, 0);
}

#[test]
fn phrase_cell_entry_and_undo() {
    let mut e = phrase_ed();
    e.phrase_key(key('z'));
    assert_eq!(e.song.instruments[0].phrases[0].line(0).unwrap().notes[0].note, 48);
    assert_eq!(e.phrase_cursor.line, 1);
    e.phrase_set_cursor(0, 1, 1);
    e.phrase_key(key('a'));
    e.phrase_key(key('1'));
    assert_eq!(e.song.instruments[0].phrases[0].line(1).unwrap().notes[0].instrument, 0xA1);
    // effect column
    let notes = e.phrase_cols().0;
    e.phrase_set_cursor(notes, 0, 2);
    for c in ['0', 'a', '3', '7'] {
        e.phrase_key(key(c));
    }
    let fx = e.song.instruments[0].phrases[0].line(2).unwrap().effects[0];
    assert_eq!((&fx.id, fx.amount), (b"0A", 0x37));
    e.undo();
    assert!(e.song.instruments[0].phrases[0].line(2).is_none_or(|l| l.effects[0].amount != 0x37));
    e.phrase_set_cursor(0, 0, 0);
    e.phrase_key(KeyEvent::new(KeyCode::Delete));
    assert!(e.song.instruments[0].phrases[0].line(0).is_none());
}

#[test]
fn phrase_entry_needs_edit_mode_and_remove_works() {
    let mut e = phrase_ed();
    e.edit_mode = false;
    e.phrase_key(key('z'));
    assert!(e.song.instruments[0].phrases[0].lines.is_empty());
    e.phrase_remove();
    assert!(e.song.instruments[0].phrases.is_empty());
    e.phrase_key(key('z')); // nothing selected: must not panic
}

struct MockOut(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

struct MockSink(std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>);

impl MidiSink for MockSink {
    fn send(&mut self, bytes: &[u8]) {
        self.0.lock().unwrap().push(bytes.to_vec());
    }
    fn port_name(&self) -> &str {
        "mock out"
    }
}

impl MidiBackend for MockOut {
    fn ports(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn open(&mut self, _port: usize) -> anyhow::Result<Box<dyn MidiInput>> {
        anyhow::bail!("no input")
    }
    fn out_ports(&mut self) -> Vec<String> {
        vec!["mock out".into()]
    }
    fn open_out(&mut self, _port: usize) -> anyhow::Result<Box<dyn MidiSink>> {
        Ok(Box::new(MockSink(self.0.clone())))
    }
}

#[test]
fn midi_output_carries_instrument_notes_and_clock_and_can_reconnect() {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (mut engine, handle) = trakr_engine::create(44_100);
    let mut e = Editor::new(Song::demo(), None, Some(handle));
    e.set_midi_backend(Box::new(MockOut(log.clone())));
    assert_eq!(e.midi_out_ports(), vec!["mock out".to_string()]);
    e.instrument_set_midi(Some(trakr_core::MidiOut { channel: 5, program: None }));
    e.set_midi_clock_out(true);
    e.midi_out_connect(0);
    assert_eq!(e.midi_out_port_name(), Some("mock out"));
    e.do_action(Action::PlaySong);
    let mut buf = vec![0.0f32; 2048 * 2];
    for _ in 0..40 {
        engine.render(&mut buf);
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    e.midi_out_disconnect();
    assert_eq!(e.midi_out_port_name(), None);
    {
        let got = log.lock().unwrap();
        assert!(got.iter().any(|m| m[0] == 0x95 && m[2] > 0), "note on, channel 6: {got:?}");
        assert!(got.iter().any(|m| m == &[0xFA]), "start");
        assert!(got.iter().filter(|m| m.as_slice() == [0xF8]).count() > 10, "clock");
    }
    // the source went back to the engine handle, so connecting again works
    e.midi_out_connect(0);
    assert_eq!(e.midi_out_port_name(), Some("mock out"));
    e.midi_out_disconnect();
}

#[test]
fn a_script_is_one_undo_step_and_errors_change_nothing() {
    let mut e = ed();
    e.set_cursor(2, 0, 0, 4);
    let before = e.song.patterns[0].clone();
    assert!(e.run_script(
        "for l = 0, 7 do trakr.set_note(trakr.ctx.pattern, trakr.ctx.track, l, 0, { note = 'C-4', instrument = 0 }) end print('done')"
    ));
    assert_eq!(e.script_output, vec!["done".to_string()]);
    assert_eq!(e.song.patterns[0].line(2, 7).unwrap().notes[0].note, 48);
    assert!(e.dirty);
    e.undo();
    assert_eq!(*e.song.patterns[0], *before, "the whole run undoes at once");
    let again = e.song.patterns[0].clone();
    assert!(!e.run_script("trakr.set_note(0, 0, 0, 0, { note = 48 }); error('nope')"));
    assert!(e.script_error.as_deref().is_some_and(|m| m.contains("nope")));
    assert!(e.status.starts_with("script error"));
    assert_eq!(*e.song.patterns[0], *again, "failed scripts change nothing");
    assert!(!e.run_script_file(std::path::Path::new("/nonexistent/x.lua")));
}

#[test]
fn script_context_and_requests() {
    let mut e = ed();
    e.set_cursor(3, 0, 0, 9);
    e.instrument = 2;
    assert!(e.run_script("assert(trakr.ctx.track == 3 and trakr.ctx.line == 9 and trakr.ctx.instrument == 2); trakr.status('checked')"));
    assert_eq!(e.status, "checked");
    // selection arrives as tracks and lines
    e.set_cursor(1, 0, 0, 2);
    e.select_to(2, 0, 6);
    assert!(e.run_script("local s = trakr.ctx.selection; assert(s.track0 == 1 and s.track1 == 2 and s.line0 == 2 and s.line1 == 6)"));
}

#[test]
fn user_scripts_are_listed_from_the_config_dir() {
    let dir = std::env::temp_dir().join(format!("trakr-scripts-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(dir.join("scripts/b.lua"), "trakr.status('b')").unwrap();
    std::fs::write(dir.join("scripts/a.lua"), "trakr.status('a')").unwrap();
    std::fs::write(dir.join("scripts/readme.txt"), "x").unwrap();
    let mut e = ed();
    e.set_config_dir(Some(dir.clone()));
    let names: Vec<String> = e.user_scripts().into_iter().map(|s| s.0).collect();
    assert_eq!(names, ["a", "b"]);
    let path = e.user_scripts()[1].1.clone();
    assert!(e.run_script_file(&path));
    assert_eq!(e.status, "b");
    let _ = std::fs::remove_dir_all(&dir);
}

struct Scale(f32);

impl trakr_engine::Device for Scale {
    fn id(&self) -> &'static str {
        "clap"
    }
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let g = params.first().copied().unwrap_or(1.0) * self.0;
        for s in buf {
            *s *= g;
        }
    }
}

struct FakeHost {
    loads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    fail: bool,
}

impl trakr_engine::PluginHost for FakeHost {
    fn scan(&mut self) -> Vec<trakr_core::PluginInfo> {
        vec![trakr_core::PluginInfo {
            instrument: false,
            path: "/fake/gain.clap".into(),
            plugin_id: "fake.gain".into(),
            name: "Fake Gain".into(),
            vendor: "v".into(),
        }]
    }
    fn load(
        &mut self,
        path: &str,
        plugin_id: &str,
        _rate: u32,
    ) -> Result<(Box<dyn trakr_engine::Device>, trakr_core::PluginRef), String> {
        self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail {
            return Err("nope".into());
        }
        let p = trakr_core::PluginParam {
            id: 1,
            name: "Gain".into(),
            min: 0.0,
            max: 2.0,
            default: 1.0,
            stepped: false,
        };
        Ok((
            Box::new(Scale(1.0)),
            trakr_core::PluginRef {
                path: path.into(),
                plugin_id: plugin_id.into(),
                name: "Fake Gain".into(),
                params: vec![p],
            },
        ))
    }
}

#[test]
fn plugins_are_added_configured_undone_and_used_when_rendering() {
    let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (_engine, handle) = trakr_engine::create(44_100);
    let mut e = Editor::new(Song::demo(), None, Some(handle));
    assert!(!e.has_plugin_host());
    e.scan_plugins();
    assert!(e.status.contains("cannot host"));
    e.set_plugin_host(Box::new(FakeHost { loads: loads.clone(), fail: false }));
    e.scan_plugins();
    assert_eq!(e.plugins().len(), 1);
    let info = e.plugins()[0].clone();
    assert!(e.device_add_plugin(0, &info));
    let spec = e.song.tracks[0].devices[0].clone();
    assert_eq!((spec.id.as_str(), spec.params.clone()), ("clap:fake.gain", vec![1.0]));
    assert!(
        loads.load(std::sync::atomic::Ordering::SeqCst) >= 2,
        "once to describe, once to build the chain"
    );

    e.device_set_param(0, 0, 0, 5.0);
    assert_eq!(e.song.tracks[0].devices[0].params, vec![2.0], "clamped to the plugin's range");
    e.device_set_param(0, 0, 3, 1.0); // no such parameter: ignored
    e.device_set_param(0, 0, 0, f32::NAN);
    assert_eq!(e.song.tracks[0].devices[0].params, vec![2.0]);

    // rendering the song offline goes through the plugin too
    let with = {
        e.device_set_param(0, 0, 0, 0.0);
        let data = e.render();
        data.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert_eq!(with, 0.0, "the plugin silenced track 1 (the demo's only track with notes)");
    e.undo();
    e.undo();
    e.undo();
    assert!(e.song.tracks[0].devices.is_empty() || e.song.tracks[0].devices[0].params != vec![0.0]);

    // a plugin that fails to load becomes a pass-through and the user is told why
    let mut bad = Editor::new(Song::demo(), None, None);
    bad.set_plugin_host(Box::new(FakeHost { loads, fail: true }));
    assert!(!bad.device_add_plugin(0, &info));
    assert!(bad.status.contains("cannot load") && bad.status.contains("nope"), "{}", bad.status);
}

#[test]
fn song_with_plugin_survives_a_trkr_roundtrip_and_old_files_still_load() {
    let spec = trakr_core::DeviceSpec::from_plugin(trakr_core::PluginRef {
        path: "/x/y.clap".into(),
        plugin_id: "a.b".into(),
        name: "N".into(),
        params: vec![trakr_core::PluginParam {
            id: 3,
            name: "P".into(),
            min: -1.0,
            max: 1.0,
            default: 0.5,
            stepped: true,
        }],
    });
    let mut song = Song::demo();
    song.tracks[0].devices.push(spec.clone());
    let json = serde_json::to_string(&song).unwrap();
    let back: Song = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tracks[0].devices[0], spec);
    // a device saved before plugins existed has no `plugin` field
    let old = r#"{"id":"native.gainer","bypass":false,"params":[0.0,0.0]}"#;
    let d: trakr_core::DeviceSpec = serde_json::from_str(old).unwrap();
    assert!(d.plugin.is_none());
}

fn fill(e: &mut Editor, lines: &[(u16, u8, u8)]) {
    e.edit_mode = true;
    for &(l, note, inst) in lines {
        e.set_cursor(0, 0, 0, l);
        e.edit(|x| {
            x.notes[0].note = note;
            x.notes[0].instrument = inst;
        });
    }
}

fn select(e: &mut Editor, l0: u16, l1: u16) {
    e.set_cursor(0, 0, 0, l0);
    e.select_to(0, 0, l1);
}

#[test]
fn expand_shrink_and_repeat_blocks() {
    let mut e = ed();
    fill(&mut e, &[(0, 48, 0), (1, 50, 0), (2, 52, 0), (3, 53, 0)]);
    select(&mut e, 0, 3);
    e.expand_block();
    let notes: Vec<u8> = (0..8).map(|l| note_at(&e, 0, l)).collect();
    assert_eq!(notes, [48, NOTE_EMPTY, 50, NOTE_EMPTY, 52, NOTE_EMPTY, 53, NOTE_EMPTY]);
    select(&mut e, 0, 7);
    e.shrink_block();
    let notes: Vec<u8> = (0..8).map(|l| note_at(&e, 0, l)).collect();
    assert_eq!(notes, [48, 50, 52, 53, NOTE_EMPTY, NOTE_EMPTY, NOTE_EMPTY, NOTE_EMPTY]);
    select(&mut e, 0, 1);
    e.repeat_block();
    assert_eq!((note_at(&e, 0, 2), note_at(&e, 0, 3), note_at(&e, 0, 62)), (48, 50, 48));
    e.undo();
    assert_eq!(note_at(&e, 0, 2), 52);
    e.edit_mode = false;
    e.expand_block();
    assert!(e.status.contains("edit mode"));
}

#[test]
fn randomize_stays_in_range_and_is_reproducible() {
    let mut a = ed();
    fill(&mut a, &[(0, 48, 0), (1, 48, 0), (2, 48, 0)]);
    let mut b = ed();
    fill(&mut b, &[(0, 48, 0), (1, 48, 0), (2, 48, 0)]);
    for e in [&mut a, &mut b] {
        e.set_random_seed(42);
        select(e, 0, 2);
        e.randomize_volume(40);
    }
    let vols = |e: &Editor| -> Vec<u8> {
        (0..3).map(|l| e.song.patterns[0].line(0, l).unwrap().notes[0].volume).collect()
    };
    assert_eq!(vols(&a), vols(&b));
    assert!(vols(&a).iter().all(|v| *v <= 0x80 && *v >= 0x80 - 40));
    assert!(vols(&a).windows(2).any(|w| w[0] != w[1]), "values differ: {:?}", vols(&a));
}

#[test]
fn find_and_replace_notes_and_instruments() {
    let mut e = ed();
    fill(&mut e, &[(2, 48, 1), (5, 52, 1), (9, 48, 2), (12, 48, 1)]);
    e.set_cursor(0, 0, 0, 0);
    let spec = CellSpec { note: Some(48), ..Default::default() };
    assert!(e.find_next(spec));
    assert_eq!(e.cursor.line, 2);
    assert!(e.find_next(spec) && e.cursor.line == 9);
    assert!(e.find_next(spec) && e.cursor.line == 12);
    assert!(e.find_next(spec) && e.cursor.line == 2, "wraps around");
    assert!(!e.find_next(CellSpec { note: Some(10), ..Default::default() }));
    // replace instrument 1 with 7, only where the note is 48
    let n = e.replace_all(
        CellSpec { note: Some(48), instrument: Some(1), ..Default::default() },
        CellSpec { instrument: Some(7), ..Default::default() },
        Scope::Pattern,
    );
    assert_eq!(n, 2);
    let inst = |l| e.song.patterns[0].line(0, l).unwrap().notes[0].instrument;
    assert_eq!((inst(2), inst(5), inst(9), inst(12)), (7, 1, 2, 7));
    // selection scope only touches the selection
    select(&mut e, 0, 6);
    let n = e.replace_all(
        CellSpec { instrument: Some(7), ..Default::default() },
        CellSpec { note: Some(60), ..Default::default() },
        Scope::Selection,
    );
    assert_eq!(n, 1);
    assert_eq!(note_at(&e, 0, 2), 60);
    assert_eq!(note_at(&e, 0, 12), 48);
    assert_eq!(e.replace_all(CellSpec::default(), CellSpec::default(), Scope::Song), 0);
    e.undo();
    assert_eq!(note_at(&e, 0, 2), 48);
}

#[test]
fn midi_learn_maps_controllers_to_mixer_devices_and_commands() {
    let mut e = Editor::new(Song::demo(), None, None);
    e.device_add(0, "native.gainer");
    // volume knob
    e.midi_learn_start(MapTarget::TrackVolume(0));
    assert!(e.midi_learn.is_some());
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 21, value: 1 });
    assert!(e.midi_learn.is_none());
    assert_eq!(e.prefs.midi_map.len(), 1);
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 21, value: 0 });
    assert_eq!(e.song.tracks[0].volume, 0.0);
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 21, value: 127 });
    assert!((e.song.tracks[0].volume - 2.0).abs() < 0.01, "{}", e.song.tracks[0].volume);
    // another channel is ignored when learned on channel 0... but mappings made on a channel stick to it
    e.song_mut().tracks[0].volume = 1.0;
    e.handle_midi(MidiEvent::Cc { channel: 5, controller: 21, value: 0 });
    assert_eq!(e.song.tracks[0].volume, 1.0);
    // device parameter: gainer gain spans -60..12 dB
    e.midi_learn_start(MapTarget::DeviceParam { track: 0, device: 0, param: 0 });
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 22, value: 127 });
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 22, value: 127 });
    assert!((e.song.tracks[0].devices[0].params[0] - 12.0).abs() < 0.01);
    // a command fires when the controller crosses half
    e.midi_learn_start(MapTarget::Action("ToggleEdit".into()));
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 30, value: 127 });
    assert!(!e.edit_mode);
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 30, value: 127 });
    assert!(e.edit_mode);
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 30, value: 10 });
    assert!(e.edit_mode, "below half does nothing");
    // unmapped CC 7 keeps its built-in meaning; remapping a controller replaces the old mapping
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 7, value: 50 });
    assert!((e.song.tracks[e.cursor.track].volume - 0.5).abs() < 0.01);
    let before = e.prefs.midi_map.len();
    e.midi_learn_start(MapTarget::TrackPan(1));
    e.handle_midi(MidiEvent::Cc { channel: 0, controller: 21, value: 0 });
    assert_eq!(e.prefs.midi_map.len(), before);
    e.midi_unmap(0);
    assert_eq!(e.prefs.midi_map.len(), before - 1);
}

#[test]
fn external_clock_sets_the_tempo_only_when_enabled() {
    let mut e = ed();
    let feed = |e: &mut Editor| {
        for i in 0..100u64 {
            e.handle_midi(MidiEvent::Clock { micros: 1_000 + i * 17_361 }); // 144 BPM
        }
    };
    feed(&mut e);
    assert_eq!(e.song.bpm, 120.0, "off by default");
    e.prefs.midi_sync_in = true;
    feed(&mut e);
    assert!((e.song.bpm - 144.0).abs() <= 1.0, "{}", e.song.bpm);
}

#[test]
fn saving_over_a_file_keeps_a_backup() {
    let dir = std::env::temp_dir().join(format!("trakr-bak-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.trkr");
    let mut e = ed();
    e.set_song_name("first".into());
    e.save_as(path.clone());
    assert!(!dir.join("a.trkr.bak").exists(), "nothing to back up the first time");
    e.set_song_name("second".into());
    e.save_as(path.clone());
    let bak = trakr_format::load_path(&dir.join("a.trkr.bak")).unwrap();
    assert_eq!(bak.name, "first");
    assert_eq!(trakr_format::load_path(&path).unwrap().name, "second");
    e.prefs.backups = false;
    std::fs::remove_file(dir.join("a.trkr.bak")).unwrap();
    e.save_as(path);
    assert!(!dir.join("a.trkr.bak").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stems_and_slot_ranges_render_to_files() {
    let dir = std::env::temp_dir().join(format!("trakr-stems-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Editor::new(Song::demo(), None, None);
    let n = e.export_stems(&dir.join("mix.wav"), ExportFormat::wav(16));
    assert_eq!(n, 1, "only the track with notes has a stem");
    let stem = dir.join("mix-01-Track_01.wav");
    assert!(stem.is_file());
    // a second track's note makes a second stem, and the first stem excludes it
    e.set_cursor(1, 0, 0, 0);
    e.edit_mode = true;
    e.edit(|l| {
        l.notes[0].note = 60;
        l.notes[0].instrument = 0;
    });
    assert_eq!(e.export_stems(&dir.join("mix.wav"), ExportFormat::wav(16)), 2);
    assert!(dir.join("mix-02-Track_02.wav").is_file());
    // slot ranges: two copies of the pattern render about twice as long as one
    e.seq_clone();
    let (one, two) = (dir.join("one.wav"), dir.join("two.wav"));
    e.export_slots(&one, 0, 0, ExportFormat::wav(16));
    e.export_slots(&two, 0, 1, ExportFormat::wav(16));
    let (a, b) = (std::fs::metadata(&one).unwrap().len(), std::fs::metadata(&two).unwrap().len());
    assert!(b as f64 > a as f64 * 1.6, "{a} vs {b}");
    e.export_slots(&one, 5, 9, ExportFormat::wav(16));
    assert!(e.status.contains("nothing"));
    let _ = std::fs::remove_dir_all(&dir);
}

fn osc(addr: &str, args: &[trakr_osc::Arg]) -> trakr_osc::Message {
    trakr_osc::Message { address: addr.into(), args: args.to_vec() }
}

#[test]
fn osc_controls_transport_mixer_devices_and_commands() {
    use trakr_osc::Arg::*;
    let mut e = Editor::new(Song::demo(), None, None);
    e.device_add(0, "native.gainer");
    e.handle_osc(&osc("/trakr/track/1/volume", &[Float(0.0)]));
    assert_eq!(e.song.tracks[0].volume, 0.0);
    e.handle_osc(&osc("/trakr/track/1/volume", &[Float(1.0)]));
    assert!((e.song.tracks[0].volume - 2.0).abs() < 0.01);
    e.handle_osc(&osc("/trakr/track/2/pan", &[Float(1.0)]));
    assert_eq!(e.song.tracks[1].panning, 1.0);
    e.handle_osc(&osc("/trakr/track/2/mute", &[Int(1)]));
    assert_ne!(e.song.tracks[1].mute, trakr_core::MuteState::Active);
    e.handle_osc(&osc("/trakr/track/2/mute", &[Int(0)]));
    assert_eq!(e.song.tracks[1].mute, trakr_core::MuteState::Active);
    e.handle_osc(&osc("/trakr/track/3/solo", &[Bool(true)]));
    assert!(e.song.tracks[2].solo);
    e.handle_osc(&osc("/trakr/device/1/1/1", &[Float(1.0)]));
    assert!((e.song.tracks[0].devices[0].params[0] - 12.0).abs() < 0.01);
    e.handle_osc(&osc("/trakr/bpm", &[Float(150.0)]));
    assert_eq!(e.song.bpm, 150.0);
    e.handle_osc(&osc("/trakr/action", &[Str("ToggleEdit".into())]));
    assert!(e.edit_mode);
    // dangerous or unknown things are refused with a message
    for (a, args) in [
        ("/trakr/action", vec![Str("Quit".into())]),
        ("/trakr/action", vec![Str("Nope".into())]),
        ("/trakr/track/99/volume", vec![Float(0.5)]),
        ("/trakr/track/0/volume", vec![Float(0.5)]),
        ("/trakr/device/1/9/1", vec![Float(0.5)]),
        ("/other", vec![]),
        ("/trakr/bpm", vec![Str("x".into())]),
    ] {
        e.status.clear();
        e.handle_osc(&osc(a, &args));
        assert!(e.status.starts_with("OSC: ignored"), "{a}: {}", e.status);
    }
    assert!(!e.quit);
}

#[test]
fn osc_server_feeds_the_editor_through_tick() {
    let mut e = ed();
    e.osc_start(0, false);
    let addr = e.osc_addr().expect("listening");
    assert!(addr.ip().is_loopback());
    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    client.send_to(&trakr_osc::encode("/trakr/bpm", &[trakr_osc::Arg::Float(99.0)]), addr).unwrap();
    for _ in 0..100 {
        e.tick();
        if e.song.bpm == 99.0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(e.song.bpm, 99.0);
    e.osc_stop();
    assert!(e.osc_addr().is_none());
}

#[test]
fn slot_mutes_sections_and_groove_follow_the_sequence_and_undo() {
    let mut e = ed();
    e.seq_clone();
    e.seq_clone();
    assert_eq!(e.song.sequence.len(), 3);
    e.slot_toggle_mute(1, 2);
    assert!(e.song.slot_muted(1, 2));
    e.seq_goto(1);
    e.seq_clone(); // the copy inherits the mutes
    assert!(e.song.slot_muted(2, 2));
    e.seq_goto(2);
    e.seq_move(-1);
    assert!(e.song.slot_muted(1, 2) && e.song.slot_muted(2, 2));
    e.seq_delete();
    assert!(e.song.slot_meta.len() <= e.song.sequence.len());
    e.undo();
    assert_eq!(e.song.sequence.len(), 4);
    e.slot_toggle_mute(0, 99); // no such track: ignored
    e.slot_toggle_mute(77, 0);
    // sections
    e.seq_goto(1);
    e.section_toggle_here();
    e.seq_goto(3);
    e.section_toggle_here();
    assert_eq!(e.song.sections().len(), 2);
    e.seq_goto(2);
    e.play_section();
    assert_eq!(e.song.loop_range, Some((1, 2)));
    e.seq_goto(0);
    e.play_section();
    assert!(e.status.contains("not in a section"));
    e.seq_goto(1);
    e.section_toggle_here();
    assert_eq!(e.song.sections().len(), 1, "toggling again removes it");
    // groove is clamped and cleaned
    e.set_groove(vec![0.0, 9.0, f32::NAN, -9.0]);
    assert_eq!(e.song.groove, vec![0.0, 0.45, 0.0, -0.45]);
    e.undo();
    assert!(e.song.groove.is_empty());
    // play block needs a selection
    e.play_block();
    assert!(e.status.contains("select"));
    e.set_cursor(0, 0, 0, 2);
    e.select_to(0, 0, 5);
    e.play_block();
    assert!(e.status.contains("02-05"));
}

#[test]
fn formula_text_is_stored_undoable_and_bounded() {
    let (_engine, mut app) = engine_editor();
    assert!(app.device_add(0, "native.formula"));
    app.device_set_text(0, 0, "sin(t) * a");
    assert_eq!(app.song.tracks[0].devices[0].text, "sin(t) * a");
    app.device_set_text(0, 0, &"x".repeat(5000));
    assert_eq!(app.song.tracks[0].devices[0].text.chars().count(), 512);
    app.device_set_text(0, 9, "ignored");
    app.undo();
    assert!(app.song.tracks[0].devices[0].text.chars().count() <= 512);
}

#[test]
fn songs_export_in_every_format_at_the_chosen_rate() {
    let dir = std::env::temp_dir().join(format!("trakr-fmt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Editor::new(Song::demo(), None, None);
    for codec in Codec::ALL {
        let fmt = ExportFormat { codec, quality: 10, sample_rate: Some(48_000), ..Default::default() };
        let path = dir.join("song.tmp");
        e.export_song(&path, fmt);
        let out = dir.join(format!("song.{}", codec.extension()));
        assert!(out.is_file(), "{codec:?}: {}", e.status);
        assert!(std::fs::metadata(&out).unwrap().len() > 1000, "{codec:?}");
    }
    let wav =
        trakr_sample::decode_bytes(std::fs::read(dir.join("song.wav")).unwrap(), Some("wav"), "x").unwrap();
    assert_eq!(wav.sample_rate, 48_000);
    let n = e.export_stems(
        &dir.join("s.flac"),
        ExportFormat { codec: Codec::Flac, bits: 24, ..Default::default() },
    );
    assert_eq!(n, 1);
    assert!(dir.join("s-01-Track_01.flac").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_carry_song_tags_and_a_single_pattern_can_be_rendered() {
    let dir = std::env::temp_dir().join(format!("trakr-tags-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Editor::new(Song::demo(), None, None);
    e.song_mut().name = "Night Drive".into();
    e.song_mut().author = "Ada".into();
    e.song_mut().comments = "first draft".into();
    let tags = e.song_tags(Some("Lead"));
    assert_eq!((tags.title.as_str(), tags.artist.as_str()), ("Night Drive - Lead", "Ada"));
    let flac = ExportFormat { codec: Codec::Flac, ..Default::default() };
    e.export_song(&dir.join("song"), flac);
    let bytes = std::fs::read(dir.join("song.flac")).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("TITLE=Night Drive")
            && text.contains("ARTIST=Ada")
            && text.contains("COMMENT=first draft")
    );
    e.export_pattern(&dir.join("pat"), 0, flac);
    let bytes = std::fs::read(dir.join("pat.flac")).unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("TITLE=Night Drive - pattern 00"));
    assert!(trakr_sample::decode_bytes(bytes, Some("flac"), "x").unwrap().frames() > 1000);
    e.export_pattern(&dir.join("bad"), 99, flac);
    assert!(e.status.contains("no such pattern"));
    // an unnamed song still exports, with empty tags
    let mut plain = Editor::new(Song::demo(), None, None);
    plain.song_mut().name.clear();
    plain.song_mut().author.clear();
    plain.song_mut().comments.clear();
    assert!(plain.song_tags(None).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

struct FakeAudio {
    engine: std::rc::Rc<std::cell::RefCell<Option<trakr_engine::Engine>>>,
    fail: bool,
    last: std::rc::Rc<std::cell::RefCell<Option<AudioSettings>>>,
}

impl AudioControl for FakeAudio {
    fn hosts(&self) -> Vec<String> {
        vec!["Fake".into()]
    }
    fn outputs(&self, _: &str) -> Vec<String> {
        vec!["Out A".into(), "Out B".into()]
    }
    fn inputs(&self, _: &str) -> Vec<String> {
        vec!["In A".into()]
    }
    fn rates(&self, _: &str, _: &str) -> Vec<u32> {
        vec![44_100, 48_000]
    }
    fn restart(&mut self, s: &AudioSettings) -> Result<(trakr_engine::EngineHandle, AudioInfo), String> {
        *self.last.borrow_mut() = Some(s.clone());
        if self.fail {
            return Err("device vanished".into());
        }
        let rate = if s.sample_rate == 0 { 44_100 } else { s.sample_rate };
        let (engine, handle) = trakr_engine::create(rate);
        *self.engine.borrow_mut() = Some(engine);
        Ok((
            handle,
            AudioInfo {
                host: "Fake".into(),
                device: s.output.clone(),
                sample_rate: rate,
                buffer_frames: s.buffer_frames,
                channels: 2,
            },
        ))
    }
}

#[test]
fn switching_the_audio_output_moves_the_song_and_chains_to_the_new_engine() {
    let (_old, mut app) = engine_editor();
    let engine = std::rc::Rc::new(std::cell::RefCell::new(None));
    let last = std::rc::Rc::new(std::cell::RefCell::new(None));
    app.set_audio_control(
        Box::new(FakeAudio { engine: engine.clone(), fail: false, last: last.clone() }),
        None,
    );
    assert!(app.device_add(0, "native.gainer"));
    app.device_set_param(0, 0, 0, -60.0);
    app.prefs.audio = AudioSettings {
        output: "Out B".into(),
        sample_rate: 48_000,
        buffer_frames: 256,
        ..Default::default()
    };
    app.audio_apply();
    assert_eq!(app.audio_info().unwrap().sample_rate, 48_000);
    assert!(app.status.contains("48000"), "{}", app.status);
    assert_eq!(last.borrow().as_ref().unwrap().output, "Out B");
    let mut e = engine.borrow_mut().take().unwrap();
    app.do_action(Action::PlaySong);
    // the new engine got the song and the -60 dB gainer chain
    assert!(render_peak(&mut e, &mut app, 20_000) < 0.01, "chain should be rebuilt on the new engine");
    // a failing switch keeps everything as it was
    app.set_audio_control(
        Box::new(FakeAudio { engine: engine.clone(), fail: true, last }),
        app.audio_info().cloned(),
    );
    app.audio_apply();
    assert!(app.status.contains("audio not changed"), "{}", app.status);
    assert_eq!(app.audio_info().unwrap().sample_rate, 48_000);
}

struct FakeSynth {
    on: bool,
}

impl trakr_engine::Device for FakeSynth {
    fn id(&self) -> &'static str {
        "clap"
    }
    fn is_instrument(&self) -> bool {
        true
    }
    fn note_on(&mut self, _key: u8, _velocity: f32) {
        self.on = true;
    }
    fn note_off(&mut self, _key: u8) {
        self.on = false;
    }
    fn process(&mut self, params: &[f32], buf: &mut [f32]) {
        let level = params.first().copied().unwrap_or(0.5);
        buf.fill(if self.on { level } else { 0.0 });
    }
}

struct SynthHost;

impl trakr_engine::PluginHost for SynthHost {
    fn scan(&mut self) -> Vec<trakr_core::PluginInfo> {
        vec![trakr_core::PluginInfo {
            instrument: true,
            path: "/fake/synth.clap".into(),
            plugin_id: "fake.synth".into(),
            name: "Fake Synth".into(),
            vendor: "v".into(),
        }]
    }
    fn load(
        &mut self,
        path: &str,
        plugin_id: &str,
        _rate: u32,
    ) -> Result<(Box<dyn trakr_engine::Device>, trakr_core::PluginRef), String> {
        let level = trakr_core::PluginParam {
            id: 1,
            name: "Level".into(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            stepped: false,
        };
        Ok((
            Box::new(FakeSynth { on: false }),
            trakr_core::PluginRef {
                path: path.into(),
                plugin_id: plugin_id.into(),
                name: "Fake Synth".into(),
                params: vec![level],
            },
        ))
    }
}

#[test]
fn instrument_plugins_play_notes_save_and_render() {
    let (mut engine, handle) = trakr_engine::create(44_100);
    let mut e = Editor::new(Song::demo(), None, Some(handle));
    e.set_plugin_host(Box::new(SynthHost));
    e.scan_plugins();
    let info = e.plugins()[0].clone();
    assert!(info.instrument);
    // instruments are not effects
    assert!(!e.device_add_plugin(0, &info));
    assert!(e.status.contains("instrument"), "{}", e.status);
    assert!(e.song.tracks[0].devices.is_empty());
    // ... and the instrument panel takes them
    assert!(e.instrument_set_plugin(&info));
    let inst = e.instrument as usize;
    assert_eq!(e.song.instruments[inst].plugin.as_ref().unwrap().params, vec![0.5]);
    e.instrument_plugin_set_param(0, 9.0);
    assert_eq!(e.song.instruments[inst].plugin.as_ref().unwrap().params, vec![1.0], "clamped to the range");
    e.instrument_plugin_set_param(0, f32::NAN);
    assert_eq!(e.song.instruments[inst].plugin.as_ref().unwrap().params, vec![1.0]);
    e.instrument_plugin_set_param(0, 0.25);
    // a live note reaches the plugin on the cursor's track
    e.preview_note(48);
    e.tick();
    let mut out = vec![0.0f32; 2048];
    engine.render(&mut out);
    let peak = out[1000..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.25).abs() < 0.01, "plugin level 0.25 expected, got {peak}");
    // the plugin is part of the saved song and survives a roundtrip
    let json = serde_json::to_string(&*e.song).unwrap();
    let back: Song = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.instruments[inst].plugin.as_ref().unwrap().plugin.as_ref().unwrap().plugin_id,
        "fake.synth"
    );
    // offline rendering uses it too: the demo's notes on instrument 0 now come from the plugin
    e.instrument = 0;
    e.instrument_set_plugin(&info);
    let rendered = e.render();
    assert!(rendered.iter().any(|s| s.abs() > 0.05), "the plugin sounds in the render");
    // undo brings the samples back, and clearing works
    e.instrument_clear_plugin();
    assert!(e.song.instruments[0].plugin.is_none());
    e.undo();
    assert!(e.song.instruments[0].plugin.is_some());
}

#[derive(Default)]
struct GuiLog {
    shown: bool,
    shows: u32,
    state_dirty: bool,
    state: Vec<u8>,
    params: Vec<(usize, f32)>,
    polls: u32,
    loaded_state: Vec<Vec<u8>>,
}

struct FakeGui(std::sync::Arc<std::sync::Mutex<GuiLog>>);

impl trakr_engine::PluginGui for FakeGui {
    fn has_window(&self) -> bool {
        true
    }
    fn show(&mut self) -> Result<(), String> {
        let mut l = self.0.lock().unwrap();
        l.shown = true;
        l.shows += 1;
        Ok(())
    }
    fn hide(&mut self) {
        self.0.lock().unwrap().shown = false;
    }
    fn is_visible(&self) -> bool {
        self.0.lock().unwrap().shown
    }
    fn take_state_dirty(&mut self) -> bool {
        std::mem::take(&mut self.0.lock().unwrap().state_dirty)
    }
    fn save_state(&mut self) -> Option<Vec<u8>> {
        Some(self.0.lock().unwrap().state.clone())
    }
    fn take_param_changes(&mut self, out: &mut Vec<(usize, f32)>) {
        out.append(&mut self.0.lock().unwrap().params);
    }
}

struct GuiHost(std::sync::Arc<std::sync::Mutex<GuiLog>>);

impl trakr_engine::PluginHost for GuiHost {
    fn scan(&mut self) -> Vec<trakr_core::PluginInfo> {
        SynthHost.scan()
    }
    fn load(
        &mut self,
        path: &str,
        id: &str,
        rate: u32,
    ) -> Result<(Box<dyn trakr_engine::Device>, trakr_core::PluginRef), String> {
        SynthHost.load(path, id, rate)
    }
    fn load_with_gui(
        &mut self,
        path: &str,
        id: &str,
        rate: u32,
        state: &[u8],
    ) -> Result<trakr_engine::LoadedPlugin, String> {
        self.0.lock().unwrap().loaded_state.push(state.to_vec());
        let (d, r) = SynthHost.load(path, id, rate)?;
        Ok((d, r, Some(Box::new(FakeGui(self.0.clone())))))
    }
    fn poll(&mut self) {
        self.0.lock().unwrap().polls += 1;
    }
}

#[test]
fn plugin_windows_state_and_parameter_changes_flow_back_into_the_song() {
    let log = std::sync::Arc::new(std::sync::Mutex::new(GuiLog::default()));
    let (_engine, handle) = trakr_engine::create(44_100);
    let mut e = Editor::new(Song::demo(), None, Some(handle));
    e.set_plugin_host(Box::new(GuiHost(log.clone())));
    e.scan_plugins();
    let info = e.plugins()[0].clone();
    assert!(e.instrument_set_plugin(&info));
    let key = PluginKey::Instrument(e.instrument as usize);
    assert!(e.plugin_has_window(key));
    assert!(!e.plugin_window_visible(key));
    assert!(!e.plugin_windows_active());
    e.plugin_window_toggle(key);
    assert!(e.plugin_window_visible(key) && e.plugin_windows_active());
    e.tick();
    assert!(log.lock().unwrap().polls > 0, "the host is polled every tick");
    // the user changes the plugin in its window
    {
        let mut l = log.lock().unwrap();
        l.state = vec![1, 2, 3, 4, 5];
        l.state_dirty = true;
        l.params = vec![(0, 0.9), (7, 1.0)]; // the second index does not exist
    }
    e.tick();
    let spec = e.song.instruments[e.instrument as usize].plugin.clone().unwrap();
    assert_eq!(trakr_core::decode_state(&spec.text), vec![1, 2, 3, 4, 5]);
    assert_eq!(spec.params, vec![0.9]);
    // the saved state is what a rebuilt plugin gets, and a rebuild keeps one window handle per plugin
    e.sent_inst_plugins.clear();
    e.needs_sync = true;
    e.tick();
    assert_eq!(log.lock().unwrap().loaded_state.last().unwrap(), &vec![1, 2, 3, 4, 5]);
    e.plugin_window_toggle(key);
    assert!(!e.plugin_window_visible(key));
    // effect plugins have window keys per chain slot; unknown keys say so
    e.plugin_window_toggle(PluginKey::Device(0, 5));
    assert!(e.status.contains("not loaded"), "{}", e.status);
}

#[test]
fn automation_range_tools_curves_and_tempo_lane() {
    use trakr_core::{AutoCurve, AutoTarget};
    let mut e = ed();
    let tg = AutoTarget::TrackVolume;
    e.automation_edit("t", 0, tg, |a| {
        for (l, v) in [(0.0, 0.0), (2.0, 0.5), (4.0, 1.0)] {
            a.set_point(l, v);
        }
    });
    assert_eq!(e.automation_copy(0, tg, 0.0, 4.0), 3);
    e.automation_paste(0, tg, 8.0);
    assert_eq!(e.automation_lane(0, tg).unwrap().points.len(), 6);
    e.automation_flip(0, tg, 0.0, 4.0);
    assert_eq!(e.automation_lane(0, tg).unwrap().points[0], (0.0, 1.0));
    e.automation_scale(0, tg, 0.0, 4.0, 0.5);
    e.automation_shift(0, tg, 0.0, 4.0, 1.0);
    assert_eq!(e.automation_lane(0, tg).unwrap().points[0].0, 1.0);
    e.automation_set_curve(0, tg, AutoCurve::Smooth);
    assert_eq!(e.automation_lane(0, tg).unwrap().curve, AutoCurve::Smooth);
    e.automation_clear_range(0, tg, 0.0, 16.0);
    assert!(e.automation_lane(0, tg).is_none(), "an emptied lane disappears");
    e.undo();
    assert!(e.automation_lane(0, tg).is_some());
    // the tempo lane is an ordinary lane on track 0
    e.automation_edit("t", 0, AutoTarget::Bpm, |a| {
        a.set_point(0.0, AutoTarget::bpm_value(120.0));
    });
    assert!(e.automation_lane(0, AutoTarget::Bpm).is_some());
    let mut empty = ed();
    assert_eq!(empty.automation_copy(0, tg, 0.0, 4.0), 0);
    empty.automation_paste(0, tg, 0.0);
    assert!(empty.status.contains("nothing copied"));
}

#[test]
fn hidden_sub_columns_are_skipped_by_the_cursor_and_laid_out_tight() {
    use crate::{cell_text_hidden, note_layout};
    let (full_w, full) = note_layout([false; 3]);
    assert_eq!(full_w, 15);
    assert_eq!(full, crate::NOTE_HL);
    let (w, r) = note_layout([false, true, false]);
    assert_eq!(w, 12, "panning hidden: note, instrument, volume, delay");
    assert_eq!(r[5], (0, 0));
    assert_eq!(r[7], (10, 11));
    assert_eq!(note_layout([true; 3]).0, 6);
    let mut e = ed();
    e.edit_mode = true;
    e.edit(|l| {
        l.notes[0].note = 48;
        l.notes[0].instrument = 1;
        l.notes[0].volume = 0x40;
        l.notes[0].panning = 0x10;
        l.notes[0].delay = 0x20;
    });
    let line = e.song.patterns[0].line(0, 0);
    assert_eq!(cell_text_hidden(line, 0, 1, [false; 3]), "C-4 01 40 10 20");
    assert_eq!(cell_text_hidden(line, 0, 1, [false, true, false]), "C-4 01 40 20");
    assert_eq!(cell_text_hidden(line, 0, 1, [true, true, true]), "C-4 01");
    e.track_set_sub_column(0, 1, false);
    assert!(e.song.tracks[0].hide_panning);
    e.set_cursor(0, 0, 4, 0); // second volume digit
    e.on_key(KeyEvent::new(KeyCode::Right));
    assert_eq!(e.cursor.sub, 7, "panning (sub 5 and 6) is skipped");
    e.on_key(KeyEvent::new(KeyCode::Left));
    assert_eq!(e.cursor.sub, 4);
    // a click-like jump into a hidden field lands on a visible one
    e.set_cursor(0, 0, 5, 0);
    assert_eq!(e.cursor.sub, 2);
    // at the very end nothing is hidden-selected either
    e.track_set_sub_column(0, 2, false);
    e.set_cursor(0, 0, 4, 0);
    e.on_key(KeyEvent::new(KeyCode::Right));
    assert!(!e.sub_hidden(0, e.cursor.col, e.cursor.sub));
    // undo brings the column back; colors are undoable too
    e.undo();
    e.undo();
    assert!(!e.song.tracks[0].hide_panning);
    e.track_set_color(0, Some([1, 2, 3]));
    assert_eq!(e.song.tracks[0].color, Some([1, 2, 3]));
    e.undo();
    assert_eq!(e.song.tracks[0].color, None);
    // nonsense is ignored
    e.track_set_sub_column(99, 0, false);
    e.track_set_sub_column(0, 9, false);
    e.track_set_color(99, None);
}

#[test]
fn column_commands_are_typed_with_letters_and_shown_as_letters() {
    use crate::cell_text;
    let mut e = ed();
    e.edit_mode = true;
    e.set_cursor(0, 0, 0, 0);
    e.on_key(key('z')); // C-3 style note, cursor steps down
    e.set_cursor(0, 0, 3, 0); // first volume character
    e.on_key(key('g'));
    assert_eq!(e.cursor.sub, 4, "moved to the digit");
    e.on_key(key('4'));
    let n = e.song.patterns[0].line(0, 0).unwrap().notes[0];
    assert_eq!(n.volume, trakr_core::effects::column_cmd_encode('G', 4).unwrap());
    assert!(
        cell_text(e.song.patterns[0].line(0, 0), 0, 1).contains(" G4 "),
        "{}",
        cell_text(e.song.patterns[0].line(0, 0), 0, 1)
    );
    // plain hex still works in the same field: 4 then 0 makes volume 0x40
    e.set_cursor(0, 0, 3, 1);
    e.on_key(key('4'));
    e.on_key(key('0'));
    assert_eq!(e.song.patterns[0].line(0, 1).unwrap().notes[0].volume, 0x40);
    // letters in the panning column; b/c/d work as letters on the first character
    e.set_cursor(0, 0, 5, 2);
    e.on_key(key('c'));
    e.on_key(key('2'));
    assert_eq!(
        e.song.patterns[0].line(0, 2).unwrap().notes[0].panning,
        trakr_core::effects::column_cmd_encode('C', 2).unwrap()
    );
    // a digit above 8 on the first character that is not a command letter is ignored
    e.set_cursor(0, 0, 3, 3);
    e.on_key(key('a'));
    assert!(e.song.patterns[0].line(0, 3).is_none_or(|l| l.notes[0].volume == EMPTY));
    // a letter on the second character with no command there does nothing
    e.set_cursor(0, 0, 4, 4);
    e.on_key(key('g'));
    assert!(e.song.patterns[0].line(0, 4).is_none_or(|l| l.notes[0].volume == EMPTY));
}

#[test]
fn event_hook_scripts_run_on_load_save_play_and_stop() {
    let dir = std::env::temp_dir().join(format!("trakr-hooks-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("scripts/hooks")).unwrap();
    let mut e = ed();
    e.set_config_dir(Some(dir.clone()));
    // every hook records its event in the song title; the load hook also edits the song
    for ev in ["load", "save", "play", "stop"] {
        std::fs::write(
            dir.join(format!("scripts/hooks/{ev}.lua")),
            format!("trakr.set_song{{ name = trakr.song().name .. '[' .. trakr.ctx.event .. ']' }}\ntrakr.status('hook {ev}')"),
        )
        .unwrap();
    }
    e.new_song();
    assert_eq!(e.song.name, "Untitled[load]");
    assert_eq!(e.status, "hook load");
    e.do_action(Action::PlaySong);
    e.do_action(Action::Stop);
    assert_eq!(e.song.name, "Untitled[load][play][stop]");
    let out = dir.join("t.trkr");
    e.save_as(out.clone());
    assert!(e.song.name.ends_with("[save]"), "the hook runs before the file is written");
    let (saved, _) = crate::load_song(&out).unwrap();
    assert!(saved.name.ends_with("[save]"), "and its edit is in the file");
    // a failing hook reports but does not block the action
    std::fs::write(dir.join("scripts/hooks/save.lua"), "error('boom')").unwrap();
    e.save_as(out.clone());
    assert!(e.status.contains("saved") || e.status.contains("hook failed"), "{}", e.status);
    assert!(out.is_file());
    // hooks do not recurse, and missing hook files are fine
    std::fs::remove_file(dir.join("scripts/hooks/play.lua")).unwrap();
    let before = e.song.name.clone();
    e.do_action(Action::PlaySong);
    assert_eq!(e.song.name, before);
    let mut plain = ed();
    plain.do_action(Action::PlaySong); // no config dir at all
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn osc_feedback_sends_state_and_only_changes() {
    let mut server = trakr_osc::OscServer::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let target = server.local_addr().unwrap().to_string();
    let mut e = ed();
    assert!(!e.osc_feedback_start("not an address"));
    assert!(e.osc_feedback_start(&target));
    assert_eq!(e.osc_feedback_target(), Some(server.local_addr().unwrap()));
    let drain = |server: &mut trakr_osc::OscServer| {
        let mut got = Vec::new();
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            server.poll(&mut got);
        }
        got
    };
    e.tick();
    let first = drain(&mut server);
    let addrs: Vec<&str> = first.iter().map(|m| m.address.as_str()).collect();
    for want in
        ["/trakr/playing", "/trakr/position", "/trakr/bpm", "/trakr/track/1/volume", "/trakr/track/1/mute"]
    {
        assert!(addrs.contains(&want), "{want} in {addrs:?}");
    }
    e.tick();
    assert!(drain(&mut server).is_empty(), "nothing changed, nothing sent");
    e.song_mut().bpm = 150.0;
    e.song_mut().tracks[0].volume = 0.5;
    e.tick();
    let changed = drain(&mut server);
    assert_eq!(changed.len(), 2, "{changed:?}");
    let bpm = changed.iter().find(|m| m.address == "/trakr/bpm").unwrap();
    assert_eq!(bpm.args[0].as_f32(), Some(150.0));
    let vol = changed.iter().find(|m| m.address == "/trakr/track/1/volume").unwrap();
    assert_eq!(vol.args[0].as_f32(), Some(0.5));
    e.osc_feedback_stop();
    e.song_mut().bpm = 90.0;
    e.tick();
    assert!(drain(&mut server).is_empty());
}

#[test]
fn instruments_save_load_through_files_and_the_bank() {
    let dir = std::env::temp_dir().join(format!("trakr-bank-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Editor::new(Song::demo(), None, None);
    e.set_config_dir(Some(dir.join("cfg")));
    e.instrument = 0;
    let name = e.song.instruments[0].name.clone();
    let frames = e.song.instruments[0].samples[0].frames();
    let before = e.song.instruments.len();
    for ext in ["trki", "xrni"] {
        let path = dir.join(format!("one.{ext}"));
        assert!(e.instrument_save_file(&path), "{}", e.status);
        assert!(e.instrument_load_file(&path), "{}", e.status);
    }
    assert_eq!(e.song.instruments.len(), before + 2);
    let last = e.song.instruments.last().unwrap();
    assert_eq!(last.name, name);
    assert_eq!(e.instrument as usize, before + 1, "the loaded instrument is selected");
    assert!((last.samples[0].frames() as i64 - frames as i64).abs() <= 1);
    e.undo();
    assert_eq!(e.song.instruments.len(), before + 1);
    // the bank lives in the settings folder
    assert!(e.instrument_bank().is_empty());
    e.instrument = 0;
    assert!(e.instrument_save_to_bank(), "{}", e.status);
    let bank = e.instrument_bank();
    assert_eq!(bank.len(), 1);
    assert!(e.instrument_load_file(&bank[0].1));
    // failures are explained and change nothing
    let n = e.song.instruments.len();
    assert!(!e.instrument_load_file(&dir.join("missing.trki")));
    assert!(e.status.contains("failed"));
    std::fs::write(dir.join("bad.xrni"), b"junk").unwrap();
    assert!(!e.instrument_load_file(&dir.join("bad.xrni")));
    assert_eq!(e.song.instruments.len(), n);
    let mut none = Editor::new(Song::default(), None, None);
    assert!(!none.instrument_save_to_bank());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn instrument_fx_chain_edits_are_undoable_and_reach_the_engine() {
    let (mut engine, handle) = trakr_engine::create(44_100);
    let mut e = Editor::new(Song::demo(), None, Some(handle));
    e.instrument = 0;
    assert!(e.instrument_fx_add("native.gainer"));
    assert!(e.instrument_fx_add("native.filter"));
    assert!(!e.instrument_fx_add("native.lfo"), "modulators belong on tracks");
    assert!(!e.instrument_fx_add("native.nope"));
    assert_eq!(e.song.instruments[0].fx.len(), 2);
    e.instrument_fx_set_param(0, 0, -60.0);
    e.instrument_fx_set_param(0, 0, f32::NAN);
    e.instrument_fx_set_param(0, 9, 1.0);
    assert_eq!(e.song.instruments[0].fx[0].params[0], -60.0);
    e.do_action(Action::PlaySong);
    let silenced = render_peak(&mut engine, &mut e, 20_000);
    assert!(silenced < 0.05, "the -60 dB gainer silences instrument 0: {silenced}");
    e.instrument_fx_toggle_bypass(0);
    e.instrument_fx_toggle_bypass(1);
    let heard = render_peak(&mut engine, &mut e, 20_000);
    assert!(heard > silenced * 5.0 && heard > 0.05, "bypassed chain lets the sound through: {heard}");
    e.instrument_fx_move(0, 1);
    assert_eq!(e.song.instruments[0].fx[1].id, "native.gainer");
    e.instrument_fx_remove(0);
    assert_eq!(e.song.instruments[0].fx.len(), 1);
    e.undo();
    assert_eq!(e.song.instruments[0].fx.len(), 2);
    assert!(e.song.instruments[0].fx.len() <= 16);
    for _ in 0..30 {
        e.instrument_fx_add("native.gainer");
    }
    assert_eq!(e.song.instruments[0].fx.len(), 16);
    let json = serde_json::to_string(&*e.song).unwrap();
    let back: Song = serde_json::from_str(&json).unwrap();
    assert_eq!(back.instruments[0].fx.len(), 16, "FX chains are saved with the song");
}
