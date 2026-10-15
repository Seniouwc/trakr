//! Phrases: small note sequences stored on an instrument and played by a single trigger note.

use crate::pattern::{Line, MAX_EFFECT_COLUMNS, MAX_LINES, MAX_NOTE_COLUMNS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How an instrument's phrases are triggered by pattern notes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhraseMode {
    /// Notes play the sample directly.
    #[default]
    Off,
    /// A note inside a phrase's key range starts that phrase, transposed relative to its base note.
    Keymap,
    /// Notes C-0 to B-0 pick a phrase (note number = phrase number) and play it as written; every other
    /// note plays the picked phrase transposed to that note.
    Program,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Phrase {
    pub name: String,
    pub number_of_lines: u16,
    /// Lines per beat of the phrase itself (its speed is independent of the song's LPB).
    pub lpb: u8,
    pub note_columns: u8,
    pub effect_columns: u8,
    pub looping: bool,
    /// Loop region start line; the end is exclusive and `0` or `<= start` means the pattern end.
    pub loop_start: u16,
    pub loop_end: u16,
    /// The trigger note that plays the phrase untransposed.
    pub base_note: u8,
    /// Trigger notes (inclusive) that start this phrase in `Keymap` mode.
    pub key_range: (u8, u8),
    /// Swing: every second line is lengthened and the others shortened by this fraction of a line (0..0.75).
    #[serde(default)]
    pub shuffle: f32,
    pub lines: BTreeMap<u16, Line>,
}

impl Phrase {
    pub fn new(name: impl Into<String>, lines: u16) -> Self {
        Self {
            name: name.into(),
            number_of_lines: lines.clamp(1, MAX_LINES),
            lpb: 4,
            note_columns: 1,
            effect_columns: 1,
            looping: true,
            loop_start: 0,
            loop_end: 0,
            base_note: 48,
            key_range: (0, 119),
            shuffle: 0.0,
            lines: BTreeMap::new(),
        }
    }

    /// Re-establish invariants after hostile input (file data, scripts).
    pub fn sanitize(&mut self) {
        self.number_of_lines = self.number_of_lines.clamp(1, MAX_LINES);
        self.lpb = self.lpb.clamp(1, 32);
        self.note_columns = self.note_columns.clamp(1, MAX_NOTE_COLUMNS as u8);
        self.effect_columns = self.effect_columns.clamp(0, MAX_EFFECT_COLUMNS as u8);
        self.base_note = self.base_note.min(119);
        self.shuffle = if self.shuffle.is_finite() { self.shuffle.clamp(0.0, 0.75) } else { 0.0 };
        self.key_range = (self.key_range.0.min(119), self.key_range.1.min(119));
        let n = self.number_of_lines;
        self.lines.retain(|&i, l| i < n && !l.is_empty());
    }

    /// First line after the loop region (exclusive end), always in `1..=number_of_lines`.
    pub fn effective_loop_end(&self) -> u16 {
        if self.loop_end > self.loop_start && self.loop_end <= self.number_of_lines {
            self.loop_end
        } else {
            self.number_of_lines.max(1)
        }
    }

    pub fn effective_loop_start(&self) -> u16 {
        self.loop_start.min(self.effective_loop_end().saturating_sub(1))
    }

    pub fn line(&self, i: u16) -> Option<&Line> {
        self.lines.get(&i)
    }

    pub fn line_mut(&mut self, i: u16) -> &mut Line {
        self.lines.entry(i).or_default()
    }

    pub fn covers(&self, note: u8) -> bool {
        (self.key_range.0..=self.key_range.1).contains(&note)
    }
}
