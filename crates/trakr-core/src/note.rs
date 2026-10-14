use serde::{Deserialize, Serialize};

/// Notes are 0..=119 (C-0..B-9). Internal 48 = C-4 = MIDI 60.
pub const NOTE_OFF: u8 = 120;
pub const NOTE_EMPTY: u8 = 121;
/// Empty marker for instrument / volume / panning columns.
pub const EMPTY: u8 = 0xFF;

const NAMES: [&str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

/// Display name such as `C-4`, `F#2`, `OFF`, `---`.
pub fn note_name(n: u8) -> String {
    match n {
        NOTE_OFF => "OFF".into(),
        0..=119 => format!("{}{}", NAMES[(n % 12) as usize], n / 12),
        _ => "---".into(),
    }
}

/// Parse `C-4`, `c#4`, `OFF`, `---`. Returns `None` for garbage.
pub fn parse_note(s: &str) -> Option<u8> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("OFF") {
        return Some(NOTE_OFF);
    }
    if s.is_empty() || s.starts_with("--") || s.starts_with("..") {
        return Some(NOTE_EMPTY);
    }
    let b = s.as_bytes();
    if b.len() != 3 {
        return None;
    }
    let base = match b[0].to_ascii_uppercase() {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let sharp = match b[1] {
        b'#' => 1,
        b'-' => 0,
        _ => return None,
    };
    let oct = (b[2] as char).to_digit(10)? as u8;
    Some(oct * 12 + base + sharp)
}

/// Frequency in Hz (A-4 = internal 57 = 440 Hz).
pub fn note_to_freq(n: u8) -> f64 {
    440.0 * 2f64.powf((n as f64 - 57.0) / 12.0)
}

/// Effect command, e.g. id `*b"0A"` with amount `0x40`. `[0, 0]` means empty.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectCmd {
    pub id: [u8; 2],
    pub amount: u8,
}

impl EffectCmd {
    pub fn is_empty(&self) -> bool {
        self.id == [0, 0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteColumn {
    pub note: u8,
    pub instrument: u8,
    pub volume: u8,
    pub panning: u8,
    /// Sub-line delay in 1/256 of a line. 0 = none.
    pub delay: u8,
    /// Per-note-column "sample effect" (the small fx slot next to each note column).
    pub effect: EffectCmd,
}

impl Default for NoteColumn {
    fn default() -> Self {
        Self {
            note: NOTE_EMPTY,
            instrument: EMPTY,
            volume: EMPTY,
            panning: EMPTY,
            delay: 0,
            effect: EffectCmd::default(),
        }
    }
}

impl NoteColumn {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_roundtrip() {
        for n in 0..=119u8 {
            assert_eq!(parse_note(&note_name(n)), Some(n));
        }
        assert_eq!(parse_note("OFF"), Some(NOTE_OFF));
        assert_eq!(parse_note("---"), Some(NOTE_EMPTY));
    }

    #[test]
    fn a4_is_440() {
        assert!((note_to_freq(parse_note("A-4").unwrap()) - 440.0).abs() < 1e-9);
    }
}
