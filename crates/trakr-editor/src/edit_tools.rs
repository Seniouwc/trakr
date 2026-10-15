//! Bulk pattern tools: randomize, expand/shrink, repeat, find and replace.

use super::ops::Cell;
use super::*;
use trakr_core::{EffectCmd, NOTE_EMPTY};

/// What to look for (and, used as a replacement, what to write). `None` fields match anything / change nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellSpec {
    pub note: Option<u8>,
    pub instrument: Option<u8>,
    pub effect: Option<[u8; 2]>,
}

impl CellSpec {
    pub fn is_empty(&self) -> bool {
        self.note.is_none() && self.instrument.is_none() && self.effect.is_none()
    }
}

/// Where replace-all works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Selection,
    Pattern,
    Song,
}

fn note_matches(n: &trakr_core::NoteColumn, f: &CellSpec) -> bool {
    let hit_note = f.note.is_none_or(|x| n.note == x);
    let hit_inst = f.instrument.is_none_or(|x| n.instrument == x);
    let hit_fx = f.effect.is_none_or(|x| n.effect.id == x);
    // a spec with only an effect still matches note columns' small effect slot
    hit_note && hit_inst && hit_fx && !(f.note.is_none() && f.instrument.is_none() && n.effect.is_empty())
}

impl Editor {
    fn next_rand(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    /// Seed the random tools (tests, scripts that want reproducible results).
    pub fn set_random_seed(&mut self, seed: u64) {
        self.rng = seed | 1;
    }

    /// Nudge the volume column of every note in the block by up to `amount` (out of 128) either way.
    /// Notes without a volume start from full.
    pub fn randomize_volume(&mut self, amount: u8) {
        if !self.need_edit_pub() {
            return;
        }
        let b = self.target_block_pub();
        let mut rows = self.read_block_pub(b);
        let mut changed = 0;
        for row in &mut rows {
            for c in row {
                if let Cell::Note(n) = c {
                    if n.note < NOTE_OFF {
                        let cur = if n.volume == EMPTY { 0x80 } else { n.volume.min(0x80) } as i32;
                        let d = (self.next_rand() % (2 * amount as u32 + 1)) as i32 - amount as i32;
                        n.volume = (cur + d).clamp(0, 0x80) as u8;
                        changed += 1;
                    }
                }
            }
        }
        if changed == 0 {
            self.status = "no notes in the block to randomize".into();
            return;
        }
        self.write_block_pub(b.col0, b.line0, &rows);
        self.status = format!("randomized {changed} volumes");
    }

    /// Double the block's length: each line is followed by an empty one (lines below are overwritten).
    pub fn expand_block(&mut self) {
        if !self.need_edit_pub() {
            return;
        }
        let b = self.target_block_pub();
        let rows = self.read_block_pub(b);
        let out: Vec<Vec<Cell>> =
            rows.iter().flat_map(|r| [r.clone(), r.iter().map(|c| c.emptied_pub()).collect()]).collect();
        self.write_block_pub(b.col0, b.line0, &out);
    }

    /// Halve the block's length: keep every second line, then clear what is left of the old range.
    pub fn shrink_block(&mut self) {
        if !self.need_edit_pub() {
            return;
        }
        let b = self.target_block_pub();
        let rows = self.read_block_pub(b);
        let mut out: Vec<Vec<Cell>> = rows.iter().step_by(2).cloned().collect();
        while out.len() < rows.len() {
            out.push(rows[0].iter().map(|c| c.emptied_pub()).collect());
        }
        self.write_block_pub(b.col0, b.line0, &out);
    }

    /// Repeat the block below itself until the pattern ends.
    pub fn repeat_block(&mut self) {
        if !self.need_edit_pub() {
            return;
        }
        let Some(b) = self.selection() else {
            self.status = "select a block (Shift+arrows) to repeat".into();
            return;
        };
        let rows = self.read_block_pub(b);
        let len = rows.len();
        let total = self.lines_in_pattern() as usize;
        let start = b.line1 as usize + 1;
        if start >= total {
            return;
        }
        let out: Vec<Vec<Cell>> = (0..total - start).map(|i| rows[i % len].clone()).collect();
        self.write_block_pub(b.col0, start as u16, &out);
    }

    /// Move the cursor to the next cell (after the cursor, wrapping) matching `spec` in the current
    /// pattern. Returns whether one was found.
    pub fn find_next(&mut self, spec: CellSpec) -> bool {
        if spec.is_empty() {
            self.status = "nothing to look for".into();
            return false;
        }
        let Some(pi) = self.pattern_index() else { return false };
        let pat = self.song.patterns[pi].clone();
        let total = self.total_cols();
        let lines = pat.number_of_lines as usize;
        let start = self.cursor.line as usize * total + self.flat_index(self.cursor.track, self.cursor.col);
        for step in 1..=lines * total {
            let idx = (start + step) % (lines * total);
            let (line, flat) = (idx / total, idx % total);
            let (t, c) = self.unflat(flat);
            let notes = self.cols(t).0;
            let Some(l) = pat.line(t, line as u16) else { continue };
            let hit = if c < notes {
                note_matches(&l.notes[c], &spec)
            } else {
                spec.note.is_none()
                    && spec.instrument.is_none()
                    && spec.effect.is_some_and(|id| l.effects[c - notes].id == id)
            };
            if hit {
                self.set_cursor(t, c, 0, line as u16);
                return true;
            }
        }
        self.status = "not found".into();
        false
    }

    /// Replace every cell matching `find` with `with` (only the fields `with` sets change).
    /// Returns how many cells changed; one undo step.
    pub fn replace_all(&mut self, find: CellSpec, with: CellSpec, scope: Scope) -> usize {
        if find.is_empty() || with.is_empty() {
            self.status = "fill in what to find and what to replace it with".into();
            return 0;
        }
        let sel = if scope == Scope::Selection { self.selection() } else { None };
        if scope == Scope::Selection && sel.is_none() {
            self.status = "select a block first".into();
            return 0;
        }
        let patterns: Vec<usize> = match scope {
            Scope::Song => (0..self.song.patterns.len()).collect(),
            _ => self.pattern_index().into_iter().collect(),
        };
        // work out the track/column cells in range first (the selection is in flattened columns)
        let cols: Vec<(usize, usize, usize)> = (0..self.total_cols())
            .map(|f| {
                let (t, c) = self.unflat(f);
                (f, t, c)
            })
            .collect();
        self.checkpoint();
        let mut changed = 0;
        let notes_of: Vec<usize> = (0..self.song.tracks.len()).map(|t| self.cols(t).0).collect();
        let song = Arc::make_mut(&mut self.song);
        for pi in patterns {
            let mut touched = false;
            let pat = Arc::make_mut(&mut song.patterns[pi]);
            for &(f, t, c) in &cols {
                if sel.is_some_and(|b| f < b.col0 || f > b.col1) {
                    continue;
                }
                let Some(track) = pat.tracks.get_mut(t) else { continue };
                for (&line, l) in track.lines.iter_mut() {
                    if sel.is_some_and(|b| line < b.line0 || line > b.line1) {
                        continue;
                    }
                    if c < notes_of[t] {
                        let n = &mut l.notes[c];
                        if note_matches(n, &find) {
                            if let Some(x) = with.note {
                                n.note = x;
                            }
                            if let Some(x) = with.instrument {
                                n.instrument = x;
                            }
                            if let Some(x) = with.effect {
                                if !n.effect.is_empty() {
                                    n.effect.id = x;
                                }
                            }
                            changed += 1;
                            touched = true;
                        }
                    } else if let Some(id) = find.effect {
                        let e = &mut l.effects[c - notes_of[t]];
                        if find.note.is_none() && find.instrument.is_none() && e.id == id {
                            if let Some(x) = with.effect {
                                *e = EffectCmd { id: x, amount: e.amount };
                                changed += 1;
                                touched = true;
                            }
                        }
                    }
                }
            }
            let _ = touched;
        }
        if changed == 0 {
            self.undo.pop();
            self.status = "nothing matched".into();
        } else {
            self.status = format!("replaced {changed} cell(s)");
            self.sync();
        }
        changed
    }
}

#[allow(dead_code)]
const _: u8 = NOTE_EMPTY;
