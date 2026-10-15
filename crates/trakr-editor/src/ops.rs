//! Song operations: blocks and clipboard, lines, sequence, patterns, tracks, transport.

use super::*;
use trakr_core::{
    EffectCmd, MuteState, NoteColumn, PatternTrack, TrackKind, MAX_EFFECT_COLUMNS, MAX_LINES,
    MAX_NOTE_COLUMNS, NOTE_EMPTY,
};

/// One pattern cell: a note column or an effect column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Note(NoteColumn),
    Fx(EffectCmd),
}

impl Cell {
    pub(crate) fn emptied_pub(self) -> Cell {
        self.emptied()
    }

    fn emptied(self) -> Cell {
        match self {
            Cell::Note(_) => Cell::Note(NoteColumn::default()),
            Cell::Fx(_) => Cell::Fx(EffectCmd::default()),
        }
    }
}

/// Copied block, `rows[line][column]`.
#[derive(Clone, Debug)]
pub struct Clipboard {
    pub rows: Vec<Vec<Cell>>,
}

fn read_cell(p: &Pattern, track: usize, col: usize, notes: usize, line: u16) -> Cell {
    let l = p.line(track, line);
    if col < notes {
        Cell::Note(l.map(|l| l.notes[col]).unwrap_or_default())
    } else {
        Cell::Fx(l.map(|l| l.effects[col - notes]).unwrap_or_default())
    }
}

fn write_cell(p: &mut Pattern, track: usize, col: usize, notes: usize, line: u16, cell: Cell) {
    let l = p.line_mut(track, line);
    match cell {
        Cell::Note(n) if col < notes => l.notes[col] = n,
        Cell::Fx(e) if col >= notes => l.effects[col - notes] = e,
        _ => {}
    }
}

fn prune(p: &mut Pattern) {
    for t in &mut p.tracks {
        t.lines.retain(|_, l| !l.is_empty());
    }
}

fn lerp_u8(a: u8, b: u8, i: usize, n: usize) -> u8 {
    (a as f32 + (b as f32 - a as f32) * i as f32 / n as f32).round() as u8
}

/// Keep stored track indices valid after a track was inserted at `at` (`delta = 1`) or removed (`-1`):
/// group routing, send feeds and automation lanes follow their tracks; references to a removed track go away.
fn shift_track_refs(song: &mut Song, at: usize, delta: i32) {
    let fix = |i: usize| -> Option<usize> {
        if delta > 0 {
            Some(if i >= at { i + 1 } else { i })
        } else if i == at {
            None
        } else {
            Some(if i > at { i - 1 } else { i })
        }
    };
    for tr in &mut song.tracks {
        tr.group = tr.group.and_then(fix);
        tr.sends.retain_mut(|s| match fix(s.target) {
            Some(t) => {
                s.target = t;
                true
            }
            None => false,
        });
    }
    for p in &mut song.patterns {
        if p.automation.is_empty() {
            continue;
        }
        let p = Arc::make_mut(p);
        p.automation.retain_mut(|a| match fix(a.track) {
            Some(t) => {
                a.track = t;
                true
            }
            None => false,
        });
    }
}

impl Editor {
    // ---- flattened columns and selection --------------------------------

    pub fn total_cols(&self) -> usize {
        (0..self.song.tracks.len()).map(|t| self.cols(t).0 + self.cols(t).1).sum()
    }

    pub fn flat_index(&self, track: usize, col: usize) -> usize {
        (0..track).map(|t| self.cols(t).0 + self.cols(t).1).sum::<usize>() + col
    }

    /// (track, column) for a flattened column index, clamped to the last column.
    pub fn unflat(&self, mut f: usize) -> (usize, usize) {
        for t in 0..self.song.tracks.len() {
            let n = self.cols(t).0 + self.cols(t).1;
            if f < n {
                return (t, f);
            }
            f -= n;
        }
        let t = self.song.tracks.len() - 1;
        (t, self.cols(t).0 + self.cols(t).1 - 1)
    }

    pub(crate) fn begin_selection(&mut self) {
        if self.anchor.is_none() {
            self.anchor = Some((self.flat_index(self.cursor.track, self.cursor.col), self.cursor.line));
        }
    }

    /// Extend the selection to a cell (mouse drag), anchoring at the current cursor if there is none.
    pub fn select_to(&mut self, track: usize, col: usize, line: u16) {
        self.begin_selection();
        self.cursor.track = track.min(self.song.tracks.len() - 1);
        self.cursor.col = col;
        self.cursor.line = line;
        self.clamp_cursor();
    }

    pub fn selection(&self) -> Option<Block> {
        let (af, al) = self.anchor?;
        let cf = self.flat_index(self.cursor.track, self.cursor.col);
        Some(Block {
            col0: af.min(cf),
            col1: af.max(cf),
            line0: al.min(self.cursor.line),
            line1: al.max(self.cursor.line),
        })
    }

    pub fn select_none(&mut self) {
        self.anchor = None;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some((0, 0));
        let last = self.total_cols() - 1;
        let (t, c) = self.unflat(last);
        self.cursor.track = t;
        self.cursor.col = c;
        self.cursor.line = self.lines_in_pattern() - 1;
    }

    /// The selection, or the single cell under the cursor.
    fn target_block(&self) -> Block {
        self.selection().unwrap_or_else(|| {
            let f = self.flat_index(self.cursor.track, self.cursor.col);
            Block { col0: f, col1: f, line0: self.cursor.line, line1: self.cursor.line }
        })
    }

    pub(crate) fn need_edit_pub(&mut self) -> bool {
        self.need_edit()
    }

    pub(crate) fn target_block_pub(&self) -> Block {
        self.target_block()
    }

    pub(crate) fn read_block_pub(&self, b: Block) -> Vec<Vec<Cell>> {
        self.read_block(b)
    }

    pub(crate) fn write_block_pub(&mut self, f0: usize, l0: u16, rows: &[Vec<Cell>]) {
        self.write_block(f0, l0, rows)
    }

    fn need_edit(&mut self) -> bool {
        if !self.edit_mode {
            self.status = "edit mode is off: press Space".into();
        }
        self.edit_mode
    }

    fn read_block(&self, b: Block) -> Vec<Vec<Cell>> {
        let Some(p) = self.pattern_index().and_then(|i| self.song.patterns.get(i)) else { return vec![] };
        (b.line0..=b.line1)
            .map(|l| {
                (b.col0..=b.col1)
                    .map(|f| {
                        let (t, c) = self.unflat(f);
                        read_cell(p, t, c, self.cols(t).0, l)
                    })
                    .collect()
            })
            .collect()
    }

    /// Write `rows` with its top-left at (flattened column `f0`, line `l0`). Cells that fall outside
    /// the pattern or land on a column of the other kind are skipped.
    fn write_block(&mut self, f0: usize, l0: u16, rows: &[Vec<Cell>]) {
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint();
        let total = self.total_cols();
        let cols: Vec<(usize, usize, usize)> =
            (0..total).map(|f| self.unflat(f)).map(|(t, c)| (t, c, self.cols(t).0)).collect();
        let song = Arc::make_mut(&mut self.song);
        let p = Arc::make_mut(&mut song.patterns[pi]);
        for (r, row) in rows.iter().enumerate() {
            let line = l0 as usize + r;
            if line >= p.number_of_lines as usize {
                break;
            }
            for (c, cell) in row.iter().enumerate() {
                if let Some(&(t, col, notes)) = cols.get(f0 + c) {
                    write_cell(p, t, col, notes, line as u16, *cell);
                }
            }
        }
        prune(p);
        self.sync();
    }

    // ---- block operations ------------------------------------------------

    pub fn copy(&mut self) {
        let b = self.target_block();
        self.clipboard = Some(Clipboard { rows: self.read_block(b) });
        self.status = "copied".into();
    }

    pub fn cut(&mut self) {
        if !self.need_edit() {
            return;
        }
        self.copy();
        self.clear_block();
        self.status = "cut".into();
    }

    pub fn paste(&mut self) {
        if !self.need_edit() {
            return;
        }
        let Some(cb) = self.clipboard.clone() else {
            self.status = "clipboard is empty".into();
            return;
        };
        let f0 = self.flat_index(self.cursor.track, self.cursor.col);
        self.write_block(f0, self.cursor.line, &cb.rows);
        self.move_lines(cb.rows.len() as i32);
        self.status = "pasted".into();
    }

    /// Paste without erasing: cells that are empty in the clipboard leave what is already in the pattern.
    pub fn paste_mix(&mut self) {
        if !self.need_edit() {
            return;
        }
        let Some(cb) = self.clipboard.clone() else {
            self.status = "clipboard is empty".into();
            return;
        };
        let f0 = self.flat_index(self.cursor.track, self.cursor.col);
        let rows = self.merge_over_existing(f0, self.cursor.line, &cb.rows);
        self.write_block(f0, self.cursor.line, &rows);
        self.move_lines(cb.rows.len() as i32);
        self.status = "pasted (mixed with what was there)".into();
    }

    /// Paste the clipboard again and again from the cursor to the end of the selection, or of the pattern.
    pub fn paste_flood(&mut self) {
        if !self.need_edit() {
            return;
        }
        let Some(cb) = self.clipboard.clone().filter(|c| !c.rows.is_empty()) else {
            self.status = "clipboard is empty".into();
            return;
        };
        let (start, end, f0) = match self.anchor {
            Some(_) => {
                let b = self.target_block();
                (b.line0, b.line1, b.col0)
            }
            None => (
                self.cursor.line,
                self.lines_in_pattern().saturating_sub(1),
                self.flat_index(self.cursor.track, self.cursor.col),
            ),
        };
        if end < start {
            return;
        }
        let total = (end - start + 1) as usize;
        let rows: Vec<Vec<Cell>> = (0..total).map(|i| cb.rows[i % cb.rows.len()].clone()).collect();
        self.write_block(f0, start, &rows);
        self.status = format!("pasted {total} line(s)");
    }

    /// `rows` with every empty cell replaced by what the pattern holds at that position.
    fn merge_over_existing(&self, f0: usize, l0: u16, rows: &[Vec<Cell>]) -> Vec<Vec<Cell>> {
        let Some(p) = self.pattern_index().and_then(|i| self.song.patterns.get(i)) else {
            return rows.to_vec();
        };
        rows.iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, cell)| {
                        let empty = *cell == cell.emptied();
                        let f = f0 + c;
                        if !empty || f >= self.total_cols() {
                            return *cell;
                        }
                        let (t, col) = self.unflat(f);
                        let line = l0 as usize + r;
                        if line >= p.number_of_lines as usize {
                            return *cell;
                        }
                        read_cell(p, t, col, self.cols(t).0, line as u16)
                    })
                    .collect()
            })
            .collect()
    }

    /// Clear the selection, or just the field under the cursor when nothing is selected.
    pub fn clear(&mut self) {
        if !self.need_edit() {
            return;
        }
        if self.anchor.is_some() {
            self.clear_block();
        } else {
            self.clear_field();
            self.step_down();
        }
    }

    fn clear_block(&mut self) {
        let b = self.target_block();
        let rows: Vec<Vec<Cell>> =
            self.read_block(b).into_iter().map(|r| r.into_iter().map(Cell::emptied).collect()).collect();
        self.write_block(b.col0, b.line0, &rows);
    }

    fn clear_field(&mut self) {
        let Cursor { track, col, sub, .. } = self.cursor;
        let (notes, _) = self.cols(track);
        self.edit(|l| {
            if col < notes {
                let n = &mut l.notes[col];
                match sub {
                    0 => *n = NoteColumn::default(),
                    1 | 2 => n.instrument = EMPTY,
                    3 | 4 => n.volume = EMPTY,
                    5 | 6 => n.panning = EMPTY,
                    _ => n.delay = 0,
                }
            } else {
                l.effects[col - notes] = Default::default();
            }
        });
    }

    pub fn transpose(&mut self, delta: i32) {
        if !self.need_edit() {
            return;
        }
        let b = self.target_block();
        let rows: Vec<Vec<Cell>> = self
            .read_block(b)
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|c| match c {
                        Cell::Note(mut n) if n.note < NOTE_OFF => {
                            n.note = (n.note as i32 + delta).clamp(0, NOTE_OFF as i32 - 1) as u8;
                            Cell::Note(n)
                        }
                        other => other,
                    })
                    .collect()
            })
            .collect();
        self.write_block(b.col0, b.line0, &rows);
    }

    pub fn reverse(&mut self) {
        if !self.need_edit() {
            return;
        }
        let b = self.target_block();
        let mut rows = self.read_block(b);
        rows.reverse();
        self.write_block(b.col0, b.line0, &rows);
    }

    /// Linearly fill volume, panning and effect amounts between the first and last line of the
    /// selection, in every column where both end values are set.
    pub fn interpolate(&mut self) {
        if !self.need_edit() {
            return;
        }
        let Some(b) = self.selection() else {
            self.status = "select a block (Shift+arrows) to interpolate".into();
            return;
        };
        let mut rows = self.read_block(b);
        let n = rows.len().saturating_sub(1);
        if n < 2 {
            return;
        }
        for c in 0..rows[0].len() {
            let (first, last) = (rows[0][c], rows[n][c]);
            match (first, last) {
                (Cell::Note(a), Cell::Note(z)) => {
                    for (r, row) in rows.iter_mut().enumerate().take(n).skip(1) {
                        let Cell::Note(m) = &mut row[c] else { continue };
                        if a.volume != EMPTY && z.volume != EMPTY {
                            m.volume = lerp_u8(a.volume, z.volume, r, n);
                        }
                        if a.panning != EMPTY && z.panning != EMPTY {
                            m.panning = lerp_u8(a.panning, z.panning, r, n);
                        }
                    }
                }
                (Cell::Fx(a), Cell::Fx(z)) if !a.is_empty() && a.id == z.id => {
                    for (r, row) in rows.iter_mut().enumerate().take(n).skip(1) {
                        row[c] = Cell::Fx(EffectCmd { id: a.id, amount: lerp_u8(a.amount, z.amount, r, n) });
                    }
                }
                _ => {}
            }
        }
        self.write_block(b.col0, b.line0, &rows);
    }

    /// Insert an empty line at the cursor in the current track, pushing later lines down.
    pub fn insert_line(&mut self) {
        self.shift_lines(1);
    }

    /// Delete the line at the cursor in the current track, pulling later lines up.
    pub fn delete_line(&mut self) {
        self.shift_lines(-1);
    }

    fn shift_lines(&mut self, dir: i32) {
        if !self.need_edit() {
            return;
        }
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint();
        let (t, at) = (self.cursor.track, self.cursor.line);
        let song = Arc::make_mut(&mut self.song);
        let p = Arc::make_mut(&mut song.patterns[pi]);
        let n = p.number_of_lines;
        if p.tracks.len() <= t {
            p.tracks.resize(t + 1, PatternTrack::default());
        }
        let old = std::mem::take(&mut p.tracks[t].lines);
        for (l, line) in old {
            if dir > 0 {
                if l >= at {
                    if l + 1 < n {
                        p.tracks[t].lines.insert(l + 1, line);
                    }
                } else {
                    p.tracks[t].lines.insert(l, line);
                }
            } else if l == at {
            } else if l > at {
                p.tracks[t].lines.insert(l - 1, line);
            } else {
                p.tracks[t].lines.insert(l, line);
            }
        }
        self.sync();
    }

    // ---- sequence and patterns --------------------------------------------

    pub(crate) fn after_structure_change(&mut self) {
        self.clamp_cursor();
        self.sync();
    }

    pub fn seq_goto(&mut self, slot: usize) {
        if slot < self.song.sequence.len() {
            self.seq_pos = slot;
            self.anchor = None;
            self.clamp_cursor();
        }
    }

    pub fn seq_step(&mut self, delta: i32) {
        let n = self.song.sequence.len() as i32;
        self.seq_goto((self.seq_pos as i32 + delta).rem_euclid(n) as usize);
    }

    pub fn seq_insert(&mut self) {
        self.checkpoint();
        let tracks = self.song.tracks.len();
        let lines = self.lines_in_pattern();
        let at = self.seq_pos + 1;
        let song = self.song_mut();
        song.patterns.push(Arc::new(Pattern::new(tracks, lines)));
        let idx = song.patterns.len() - 1;
        song.insert_slot(at, idx, trakr_core::SlotMeta::default());
        self.seq_pos = at;
        self.cursor.line = 0;
        self.after_structure_change();
    }

    pub fn seq_clone(&mut self) {
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint();
        let at = self.seq_pos + 1;
        let song = self.song_mut();
        let copy = (*song.patterns[pi]).clone();
        song.patterns.push(Arc::new(copy));
        let idx = song.patterns.len() - 1;
        let mutes = song.slot_meta.get(at - 1).map_or(0, |m| m.mutes);
        song.insert_slot(at, idx, trakr_core::SlotMeta { mutes, section: None, repeat: 0 });
        self.seq_pos = at;
        self.after_structure_change();
    }

    pub fn seq_delete(&mut self) {
        if self.song.sequence.len() <= 1 {
            self.status = "cannot delete the last slot".into();
            return;
        }
        self.checkpoint();
        let at = self.seq_pos;
        let song = self.song_mut();
        song.remove_slot(at);
        let n = song.sequence.len();
        song.loop_range = song.loop_range.map(|(a, b)| (a.min(n - 1), b.min(n - 1)));
        self.after_structure_change();
    }

    pub fn seq_move(&mut self, delta: i32) {
        let to = self.seq_pos as i32 + delta;
        if to < 0 || to as usize >= self.song.sequence.len() {
            return;
        }
        self.checkpoint();
        let from = self.seq_pos;
        self.song_mut().swap_slots(from, to as usize);
        self.seq_pos = to as usize;
        self.after_structure_change();
    }

    /// Point the current slot at the next/previous pattern in the pool.
    pub fn pattern_step(&mut self, delta: i32) {
        let n = self.song.patterns.len() as i32;
        let cur = self.pattern_index().unwrap_or(0) as i32;
        let to = (cur + delta).clamp(0, n - 1);
        if to == cur {
            return;
        }
        self.checkpoint();
        let at = self.seq_pos;
        self.song_mut().sequence[at] = to as usize;
        self.after_structure_change();
    }

    pub fn set_pattern_lines(&mut self, lines: u16) {
        let Some(pi) = self.pattern_index() else { return };
        let lines = lines.clamp(1, MAX_LINES);
        if self.song.patterns[pi].number_of_lines == lines {
            return;
        }
        self.checkpoint_key("pattern_lines");
        Arc::make_mut(&mut self.song_mut().patterns[pi]).number_of_lines = lines;
        self.after_structure_change();
    }

    pub fn set_pattern_name(&mut self, name: String) {
        let Some(pi) = self.pattern_index() else { return };
        self.checkpoint();
        Arc::make_mut(&mut self.song_mut().patterns[pi]).name = name;
        self.after_structure_change();
    }

    // ---- tracks -------------------------------------------------------------

    pub fn track_add(&mut self) {
        self.track_add_kind(TrackKind::Sequencer);
    }

    /// Add a track of `kind`: sequencer tracks go before the master, groups just before it, sends last.
    /// Indices stored elsewhere (groups, sends, automation lanes) follow the insertion.
    pub fn track_add_kind(&mut self, kind: TrackKind) {
        if self.song.tracks.len() >= trakr_engine::MAX_METER_TRACKS {
            self.status = format!("track limit reached ({})", trakr_engine::MAX_METER_TRACKS);
            return;
        }
        self.checkpoint();
        let song = self.song_mut();
        let idx = match kind {
            TrackKind::Sequencer => {
                song.tracks.iter().position(|t| t.kind != TrackKind::Sequencer).unwrap_or(song.tracks.len())
            }
            TrackKind::Group | TrackKind::Master => song
                .tracks
                .iter()
                .position(|t| matches!(t.kind, TrackKind::Master | TrackKind::Send))
                .unwrap_or(song.tracks.len()),
            TrackKind::Send => song.tracks.len(),
        };
        let count = song.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let name = match kind {
            TrackKind::Sequencer => format!("Track {count:02}"),
            TrackKind::Group => format!("Group {count:02}"),
            TrackKind::Send => format!("Send {count:02}"),
            TrackKind::Master => "Master".into(),
        };
        let mut track = Track::new(name, kind);
        if kind != TrackKind::Sequencer {
            track.visible_note_columns = 0; // buses carry effect commands only
        }
        song.tracks.insert(idx, track);
        shift_track_refs(song, idx, 1);
        for p in &mut song.patterns {
            if p.tracks.len() >= idx {
                Arc::make_mut(p).tracks.insert(idx, PatternTrack::default());
            }
        }
        self.cursor.track = idx;
        self.cursor.col = 0;
        self.cursor.sub = 0;
        self.after_structure_change();
    }

    /// Remove the track under the cursor (not the master, and not the last sequencer track).
    pub fn track_remove(&mut self) {
        let t = self.cursor.track;
        let song = &self.song;
        let seq_tracks = song.tracks.iter().filter(|t| t.kind == TrackKind::Sequencer).count();
        let kind = song.tracks[t].kind;
        if kind == TrackKind::Master || (kind == TrackKind::Sequencer && seq_tracks <= 1) {
            self.status = "cannot remove this track".into();
            return;
        }
        self.checkpoint();
        let song = self.song_mut();
        song.tracks.remove(t);
        shift_track_refs(song, t, -1);
        for p in &mut song.patterns {
            if p.tracks.len() > t {
                Arc::make_mut(p).tracks.remove(t);
            }
        }
        self.anchor = None;
        self.after_structure_change();
    }

    /// Route a track's output into a group track (`None` = master). Ignored if it would form a loop.
    pub fn set_track_group(&mut self, track: usize, group: Option<usize>) {
        let tracks = &self.song.tracks;
        if track >= tracks.len() || tracks[track].kind == TrackKind::Master {
            return;
        }
        if let Some(g) = group {
            if g >= tracks.len() || tracks[g].kind != TrackKind::Group || g == track {
                self.status = "tracks can only be routed into group tracks".into();
                return;
            }
            // Walk up from the target: reaching `track` again would be a cycle.
            let mut at = Some(g);
            let mut hops = 0;
            while let Some(cur) = at {
                if cur == track || hops > tracks.len() {
                    self.status = "that routing would loop back on itself".into();
                    return;
                }
                at = tracks[cur].group;
                hops += 1;
            }
        }
        self.checkpoint();
        self.song_mut().tracks[track].group = group;
        self.sync();
    }

    /// Set how much of `track` feeds the send track `target` (0 removes the feed).
    pub fn set_send(&mut self, track: usize, target: usize, level: f32, pre_fader: bool) {
        let ok = self.song.tracks.get(target).is_some_and(|t| t.kind == TrackKind::Send)
            && self
                .song
                .tracks
                .get(track)
                .is_some_and(|t| t.kind != TrackKind::Send && t.kind != TrackKind::Master);
        if !ok {
            return;
        }
        self.checkpoint_key("send_level");
        let sends = &mut self.song_mut().tracks[track].sends;
        sends.retain(|s| s.target != target);
        if level > 0.0 {
            sends.push(trakr_core::SendLevel { target, level: level.clamp(0.0, 4.0), pre_fader });
        }
        self.sync();
    }

    pub fn track_rename(&mut self, name: String) {
        self.checkpoint();
        let t = self.cursor.track;
        self.song_mut().tracks[t].name = name;
        self.after_structure_change();
    }

    pub fn track_toggle_mute(&mut self) {
        self.checkpoint();
        let t = self.cursor.track;
        let tr = &mut self.song_mut().tracks[t];
        tr.mute = if tr.mute == MuteState::Active { MuteState::Muted } else { MuteState::Active };
        self.after_structure_change();
    }

    pub fn set_columns(&mut self, note_delta: i32, fx_delta: i32) {
        self.checkpoint();
        let t = self.cursor.track;
        let tr = &mut self.song_mut().tracks[t];
        tr.visible_note_columns =
            (tr.visible_note_columns as i32 + note_delta).clamp(0, MAX_NOTE_COLUMNS as i32) as u8;
        tr.visible_effect_columns =
            (tr.visible_effect_columns as i32 + fx_delta).clamp(0, MAX_EFFECT_COLUMNS as i32) as u8;
        self.anchor = None;
        self.after_structure_change();
    }

    /// Write a linear volume ramp into the volume column over the selected lines (0 -> full for a
    /// fade in, full -> 0 for a fade out). Works on empty lines too: the volume column changes a
    /// sounding note. Needs a selection of at least two lines.
    pub fn fade_block(&mut self, fade_in: bool) {
        if !self.need_edit() {
            return;
        }
        let Some(b) = self.selection().filter(|b| b.line1 > b.line0) else {
            self.status = "select at least two lines (Shift+arrows) to fade".into();
            return;
        };
        let mut rows = self.read_block(b);
        let n = rows.len() - 1;
        for (r, row) in rows.iter_mut().enumerate() {
            let t = r as f32 / n as f32;
            let t = if fade_in { t } else { 1.0 - t };
            for cell in row.iter_mut() {
                if let Cell::Note(note) = cell {
                    note.volume = (0x80 as f32 * t).round() as u8;
                }
            }
        }
        self.write_block(b.col0, b.line0, &rows);
        self.status = if fade_in { "faded in".into() } else { "faded out".into() };
    }

    // ---- mixer ------------------------------------------------------------------

    pub fn set_track_volume(&mut self, track: usize, volume: f32) {
        if track < self.song.tracks.len() {
            self.checkpoint_key("track_volume");
            let v = volume.clamp(0.0, 4.0);
            self.song_mut().tracks[track].volume = v;
            if self.auto_record {
                // the same mapping the engine uses for volume lanes: 0..1 spans -60..+6 dB
                let db = 20.0 * v.max(1e-6).log10();
                self.record_automation(
                    track,
                    trakr_core::AutoTarget::TrackVolume,
                    ((db + 60.0) / 66.0).clamp(0.0, 1.0),
                );
            }
            self.sync();
        }
    }

    pub fn set_track_panning(&mut self, track: usize, panning: f32) {
        if track < self.song.tracks.len() {
            self.checkpoint_key("track_pan");
            let p = panning.clamp(-1.0, 1.0);
            self.song_mut().tracks[track].panning = p;
            if self.auto_record {
                self.record_automation(track, trakr_core::AutoTarget::TrackPan, (p + 1.0) * 0.5);
            }
            self.sync();
        }
    }

    pub fn track_toggle_mute_at(&mut self, track: usize) {
        if track < self.song.tracks.len() {
            self.checkpoint();
            let tr = &mut self.song_mut().tracks[track];
            tr.mute = if tr.mute == MuteState::Active { MuteState::Muted } else { MuteState::Active };
            self.sync();
        }
    }

    pub fn track_toggle_solo(&mut self, track: usize) {
        if track < self.song.tracks.len() {
            self.checkpoint();
            let tr = &mut self.song_mut().tracks[track];
            tr.solo = !tr.solo;
            self.sync();
        }
    }

    // ---- loop range ---------------------------------------------------------------------

    pub fn loop_range(&self) -> Option<(usize, usize)> {
        self.song.loop_range
    }

    /// Set the start (`start = true`) or end of the song loop range to the current slot.
    pub fn set_loop_edge(&mut self, start: bool) {
        self.checkpoint();
        let here = self.seq_pos;
        let last = self.song.sequence.len() - 1;
        let (a, b) = self.song.loop_range.unwrap_or((here, here));
        let range = if start { (here, b.max(here).min(last)) } else { (a.min(here), here) };
        self.song_mut().loop_range = Some(range);
        self.status = format!("loop range: slots {:02X} to {:02X}", range.0, range.1);
        self.sync();
    }

    pub fn clear_loop(&mut self) {
        if self.song.loop_range.is_some() {
            self.checkpoint();
            self.song_mut().loop_range = None;
            self.status = "loop range cleared".into();
            self.sync();
        }
    }

    // ---- song settings --------------------------------------------------------

    pub fn set_tempo(&mut self, bpm: f32, lpb: u8, tpl: u8) {
        self.checkpoint_key("tempo");
        let s = self.song_mut();
        s.bpm = bpm.clamp(32.0, 999.0);
        s.lpb = lpb.clamp(1, 32);
        s.tpl = tpl.clamp(1, 16);
        self.after_structure_change();
    }

    pub fn set_song_name(&mut self, name: String) {
        self.song_mut().name = name;
        self.dirty = true;
    }

    // ---- actions ---------------------------------------------------------------

    pub fn do_action(&mut self, a: Action) {
        use Action::*;
        match a {
            New => self.new_song(),
            Save => self.save(),
            Quit => self.quit = true,
            Undo => self.undo(),
            Redo => self.redo(),
            Cut => self.cut(),
            Copy => self.copy(),
            Paste => self.paste(),
            Clear => self.clear(),
            SelectAll => self.select_all(),
            SelectNone => self.select_none(),
            ToggleEdit => {
                self.edit_mode = !self.edit_mode;
                self.status = if self.edit_mode { "edit mode on".into() } else { "edit mode off".into() };
            }
            TransposeUp => self.transpose(1),
            TransposeDown => self.transpose(-1),
            TransposeOctaveUp => self.transpose(12),
            TransposeOctaveDown => self.transpose(-12),
            Reverse => self.reverse(),
            Interpolate => self.interpolate(),
            InsertLine => self.insert_line(),
            DeleteLine => self.delete_line(),
            PlaySong => {
                self.fire_hook("play");
                self.send(Command::Play { seq_pos: 0, line: 0, loop_pattern: false })
            }
            PlayPattern => {
                let seq_pos = self.seq_pos;
                self.send(Command::Play { seq_pos, line: 0, loop_pattern: true });
            }
            PlayFromCursor => {
                let (seq_pos, line) = (self.seq_pos, self.cursor.line);
                self.send(Command::Play { seq_pos, line, loop_pattern: false });
            }
            Stop => {
                self.send(Command::Stop);
                self.fire_hook("stop");
            }
            ToggleFollow => {
                self.follow = !self.follow;
                self.status =
                    if self.follow { "follow playback on".into() } else { "follow playback off".into() };
            }
            SeqPrev => self.seq_step(-1),
            SeqNext => self.seq_step(1),
            SeqInsert => self.seq_insert(),
            SeqClone => self.seq_clone(),
            SeqDelete => self.seq_delete(),
            SeqMoveUp => self.seq_move(-1),
            SeqMoveDown => self.seq_move(1),
            PatternPrev => self.pattern_step(-1),
            PatternNext => self.pattern_step(1),
            TrackAdd => self.track_add(),
            TrackAddGroup => self.track_add_kind(TrackKind::Group),
            TrackAddSend => self.track_add_kind(TrackKind::Send),
            TrackRemove => self.track_remove(),
            TrackMute => self.track_toggle_mute(),
            NoteColumnAdd => self.set_columns(1, 0),
            NoteColumnRemove => self.set_columns(-1, 0),
            EffectColumnAdd => self.set_columns(0, 1),
            EffectColumnRemove => self.set_columns(0, -1),
            OctaveDown => self.octave = self.octave.saturating_sub(1),
            OctaveUp => self.octave = (self.octave + 1).min(8),
            StepDown => self.edit_step = self.edit_step.saturating_sub(1),
            StepUp => self.edit_step = (self.edit_step + 1).min(64),
            InstrumentRemove => self.instrument_remove(),
            FadeInBlock => self.fade_block(true),
            FadeOutBlock => self.fade_block(false),
            TrackSolo => {
                let t = self.cursor.track;
                self.track_toggle_solo(t);
            }
            LoopStart => self.set_loop_edge(true),
            LoopEnd => self.set_loop_edge(false),
            LoopClear => self.clear_loop(),
            Randomize => self.randomize_volume(32),
            PasteMix => self.paste_mix(),
            PasteFlood => self.paste_flood(),
            ExpandBlock => self.expand_block(),
            ShrinkBlock => self.shrink_block(),
            RepeatBlock => self.repeat_block(),
            OpenPalette => self.palette_requested = true,
            OpenFind => self.find_requested = true,
            OpenPreferences => self.prefs_requested = true,
            PlayBlock => self.play_block(),
            SectionToggle => self.section_toggle_here(),
            PlaySection => self.play_section(),
            SlotMuteToggle => {
                let (s, t) = (self.seq_pos, self.cursor.track);
                self.slot_toggle_mute(s, t);
            }
            RecordToggle => self.record_toggle(),
            RenderToSample => self.render_to_sample(),
            InstrumentPrev => {
                self.instrument = self.instrument.saturating_sub(1);
                self.sample = 0;
            }
            InstrumentNext => {
                let max = self.song.instruments.len().saturating_sub(1).min(0xFE) as u8;
                self.instrument = (self.instrument + 1).min(max);
                self.sample = 0;
            }
        }
    }
}

#[allow(dead_code)]
const _: u8 = NOTE_EMPTY;
