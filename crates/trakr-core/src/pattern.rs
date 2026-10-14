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

    /// Drop lines that became empty and any lines beyond `number_of_lines`.
    pub fn compact(&mut self) {
        let n = self.number_of_lines;
        for t in &mut self.tracks {
            t.lines.retain(|&i, l| i < n && !l.is_empty());
        }
    }
}
