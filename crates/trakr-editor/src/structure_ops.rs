//! Song structure: per-slot track mutes, sections, groove, loop-a-block, song info.

use super::*;
use trakr_core::song::MAX_GROOVE;

impl Editor {
    /// Mute or unmute `track` while sequence slot `slot` plays.
    pub fn slot_toggle_mute(&mut self, slot: usize, track: usize) {
        if slot >= self.song.sequence.len() || track >= self.song.tracks.len().min(64) {
            return;
        }
        self.checkpoint();
        let song = self.song_mut();
        song.normalize_slot_meta();
        song.slot_meta[slot].mutes ^= 1 << track;
        let muted = song.slot_meta[slot].mutes >> track & 1 != 0;
        self.status = format!(
            "{} {} in slot {slot:02X}",
            self.song.tracks[track].name,
            if muted { "muted" } else { "unmuted" }
        );
        self.sync();
    }

    /// Start (or rename) a section at `slot`; `None` removes the marker.
    pub fn section_set(&mut self, slot: usize, name: Option<String>) {
        if slot >= self.song.sequence.len() {
            return;
        }
        self.checkpoint_key("section_name");
        let song = self.song_mut();
        song.normalize_slot_meta();
        song.slot_meta[slot].section = name;
        self.sync();
    }

    /// Add or remove a section marker at the current slot.
    pub fn section_toggle_here(&mut self) {
        let slot = self.seq_pos;
        let name = match self.song.slot_meta.get(slot).and_then(|m| m.section.clone()) {
            Some(_) => None,
            None => Some(format!("Section {}", self.song.sections().len() + 1)),
        };
        self.last_key = None;
        self.section_set(slot, name);
    }

    /// Loop the section the current slot belongs to, and play it.
    pub fn play_section(&mut self) {
        let Some((a, b, name)) = self.song.section_of(self.seq_pos) else {
            self.status = "this slot is not in a section: use Section starts here".into();
            return;
        };
        self.checkpoint();
        self.song_mut().loop_range = Some((a, b));
        self.sync();
        self.send(Command::Play { seq_pos: a, line: 0, loop_pattern: false });
        self.status = format!("looping section {name} (slots {a:02X}-{b:02X})");
    }

    /// Loop the selected lines of the current pattern (Stop ends it).
    pub fn play_block(&mut self) {
        let Some(b) = self.selection() else {
            self.status = "select the lines to loop first (Shift+arrows)".into();
            return;
        };
        let seq_pos = self.seq_pos;
        self.send(Command::PlayBlock { seq_pos, first: b.line0, last: b.line1 });
        self.status = format!("looping lines {:02X}-{:02X}", b.line0, b.line1);
    }

    /// Set the groove (shuffle) amounts: fractions of a line, each line of the cycle starts that much late.
    pub fn set_groove(&mut self, groove: Vec<f32>) {
        let g: Vec<f32> = groove
            .into_iter()
            .take(MAX_GROOVE)
            .map(|v| if v.is_finite() { v.clamp(-0.45, 0.45) } else { 0.0 })
            .collect();
        if g == self.song.groove {
            return;
        }
        self.checkpoint_key("groove");
        self.song_mut().groove = g;
        self.sync();
    }

    pub fn set_song_author(&mut self, author: String) {
        self.checkpoint_key("song_author");
        self.song_mut().author = author;
        self.sync();
    }

    pub fn set_song_comments(&mut self, comments: String) {
        self.checkpoint_key("song_comments");
        self.song_mut().comments = comments;
        self.sync();
    }
}
