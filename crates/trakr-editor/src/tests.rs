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
