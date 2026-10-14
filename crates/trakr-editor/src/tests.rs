use super::*;
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
    app.set_recorder_factory(Box::new(|| {
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
    app.export_sample(&path, 0, 0, WavFormat::Pcm16);
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
