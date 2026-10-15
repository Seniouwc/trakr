use crate::automation::Automation;
use crate::note::{EffectCmd, NoteColumn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_NOTE_COLUMNS: usize = 12;
pub const MAX_EFFECT_COLUMNS: usize = 8;
pub const MAX_LINES: u16 = 512;
pub const DEFAULT_LINES: u16 = 64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub notes: [NoteColumn; MAX_NOTE_COLUMNS],
    pub effects: [EffectCmd; MAX_EFFECT_COLUMNS],
}

impl Line {
    pub fn is_empty(&self) -> bool {
        self.notes.iter().all(NoteColumn::is_empty) && self.effects.iter().all(EffectCmd::is_empty)
    }
}

/// One track's lines within one pattern. Sparse: empty lines are not stored.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PatternTrack {
    pub lines: BTreeMap<u16, Line>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub number_of_lines: u16,
    pub tracks: Vec<PatternTrack>,
    /// Parameter curves that play with this pattern.
    #[serde(default)]
    pub automation: Vec<Automation>,
}

impl Pattern {
    pub fn new(num_tracks: usize, lines: u16) -> Self {
        Self {
            name: String::new(),
            number_of_lines: lines.clamp(1, MAX_LINES),
            tracks: vec![PatternTrack::default(); num_tracks],
            automation: Vec::new(),
        }
    }

    pub fn line(&self, track: usize, line: u16) -> Option<&Line> {
        self.tracks.get(track)?.lines.get(&line)
    }

    /// Mutable access, creating the line (and growing the track list) if needed.
    pub fn line_mut(&mut self, track: usize, line: u16) -> &mut Line {
        if self.tracks.len() <= track {
            self.tracks.resize(track + 1, PatternTrack::default());
        }
        self.tracks[track].lines.entry(line).or_default()
    }

    /// Change the length and move every line, delay-free, to its proportional place (a 32 line pattern
    /// resized to 64 spreads its lines to even positions). When several lines land on one slot (shrinking)
    /// the earliest one stays. Automation points scale with the pattern.
    pub fn resize_scaled(&mut self, lines: u16) {
        let old = self.number_of_lines.max(1) as u32;
        let new = lines.clamp(1, MAX_LINES);
        let map = |i: u16| ((i as u32 * new as u32) / old).min(new as u32 - 1) as u16;
        for t in &mut self.tracks {
            let mut moved = BTreeMap::new();
            for (i, l) in std::mem::take(&mut t.lines) {
                if i < self.number_of_lines {
                    moved.entry(map(i)).or_insert(l);
                }
            }
            t.lines = moved;
        }
        let k = new as f32 / old as f32;
        for a in &mut self.automation {
            a.points.iter_mut().for_each(|p| p.0 = (p.0 * k).min(new as f32 - 1e-3));
        }
        self.number_of_lines = new;
    }

    /// Drop lines that became empty and any lines beyond `number_of_lines`.
    pub fn compact(&mut self) {
        let n = self.number_of_lines;
        for t in &mut self.tracks {
            t.lines.retain(|&i, l| i < n && !l.is_empty());
        }
    }
}

#[cfg(test)]
mod resize_tests {
    use super::*;

    #[test]
    fn scaled_resize_moves_lines_proportionally() {
        let mut p = Pattern::new(1, 8);
        p.line_mut(0, 2).notes[0].note = 10;
        p.line_mut(0, 5).notes[0].note = 20;
        let mut a = Automation::new(0, crate::automation::AutoTarget::Bpm);
        a.points = vec![(0.0, 0.0), (4.0, 1.0)];
        p.automation.push(a);
        p.resize_scaled(16);
        assert_eq!(p.number_of_lines, 16);
        assert_eq!(p.line(0, 4).unwrap().notes[0].note, 10);
        assert_eq!(p.line(0, 10).unwrap().notes[0].note, 20);
        assert_eq!(p.automation[0].points[1].0, 8.0);
        p.resize_scaled(2);
        assert_eq!(p.number_of_lines, 2);
        assert_eq!(p.tracks[0].lines.len(), 2, "10 -> 1 and 4 -> 0");
        p.resize_scaled(1);
        assert_eq!(p.tracks[0].lines.len(), 1, "collisions keep the earliest");
        assert_eq!(p.line(0, 0).unwrap().notes[0].note, 10);
    }
}
