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

/// Default steps per octave.
pub const DEFAULT_EDO: u16 = 12;
/// Largest supported division of the octave.
pub const MAX_EDO: u16 = 240;

/// The (note, micro) pair that sounds step `step` of an `edo`-step octave, counting from C-0 (step 0).
/// `None` when the step is outside the note range 0..=119. For `edo` 12 this is `(step, 0)`.
pub fn step_to_note(step: i32, edo: u16) -> Option<(u8, i8)> {
    let edo = edo.clamp(1, MAX_EDO) as i64;
    let cents = step as i64 * 1200 * 128; // in 1/128 cent
    let unit = edo * 100 * 128; // one semitone in the same unit, times edo
    let semis = (cents + unit / 2).div_euclid(unit); // nearest semitone
    let rem = cents - semis * unit; // remainder, in 1/128 cent * edo
    let micro = (rem as f64 / edo as f64 / 100.0).round() as i64; // 1/128 semitone
    if !(0..=119).contains(&semis) {
        return None;
    }
    Some((semis as u8, micro.clamp(-64, 63) as i8))
}

/// The step number (counted from C-0) nearest to `note` + `micro` in an `edo`-step octave.
pub fn note_to_step(note: u8, micro: i8, edo: u16) -> i32 {
    let edo = edo.clamp(1, MAX_EDO) as f64;
    ((note as f64 + micro as f64 / 128.0) * edo / 12.0).round() as i32
}

/// Display text for a note cell in an `edo`-step song: ordinary names for 12, otherwise the step number
/// (octave = step / edo), three characters wide up to step 999.
pub fn tuned_name(note: u8, micro: i8, edo: u16) -> String {
    if edo == 12 || note >= NOTE_OFF {
        return note_name(note);
    }
    format!("{:03}", note_to_step(note, micro, edo).clamp(0, 999))
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
    /// Extra pitch in 1/128 semitone (-128..=127 is about +-1 semitone), added to `note`. Microtonal songs
    /// (`Song::edo` != 12) use it to reach steps between the 12 equal notes. 0 for ordinary notes.
    #[serde(default)]
    pub micro: i8,
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
            micro: 0,
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
    fn edo_steps_map_to_notes_and_back() {
        for edo in [5u16, 12, 19, 24, 31, 51, 72] {
            let max = 119 * edo as i32 / 12;
            for step in 0..=max {
                let (n, m) = step_to_note(step, edo).unwrap();
                assert_eq!(note_to_step(n, m, edo), step, "edo {edo} step {step}");
            }
        }
        assert_eq!(step_to_note(48, 12), Some((48, 0)));
        assert!(step_to_note(-3, 51).is_none() && step_to_note(10_000, 51).is_none());
        // 51 EDO: step 51 is exactly one octave above step 0
        assert_eq!(step_to_note(51, 51), Some((12, 0)));
        // step 1 of 51 is 23.53 cents: C-0 + 0 semitones, micro = 23.53/100*128 = 30
        assert_eq!(step_to_note(1, 51), Some((0, 30)));
        assert_eq!(tuned_name(12, 0, 51), "051");
        assert_eq!(tuned_name(48, 0, 12), "C-4");
    }

    #[test]
    fn a4_is_440() {
        assert!((note_to_freq(parse_note("A-4").unwrap()) - 440.0).abs() < 1e-9);
    }
}
