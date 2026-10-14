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

#[test]
fn track_limit_is_enforced() {
    let mut app = ed();
    for _ in 0..100 {
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
