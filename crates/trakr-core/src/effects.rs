//! Reference table of effect commands the engine understands. Ids are two ASCII bytes; the amount
//! is one byte shown as two hex digits. Used by the help window and docs.

/// (id, name, description). `x`/`y` are the high/low nibbles of the amount.
pub const EFFECTS: &[(&str, &str, &str)] = &[
    ("ZT", "Set BPM", "xx = BPM (32..255)"),
    ("ZL", "Set LPB", "xx = lines per beat"),
    ("ZB", "Pattern break", "jump to line xx of the next slot"),
    ("0S", "Sample offset", "start the note xx/256 into the sample"),
    ("0B", "Backwards", "xx = 01 plays the note backwards"),
    ("0C", "Cut", "cut the note after xx ticks"),
    ("0Q", "Delay", "delay the note by xx ticks"),
    ("0R", "Retrigger", "retrigger every xx ticks"),
    ("0U", "Pitch slide up", "xx/16 semitone per tick; 00 stops the slide"),
    ("0D", "Pitch slide down", "xx/16 semitone per tick; 00 stops the slide"),
    ("0G", "Glide to note", "slide the playing note to this line's note at xx/16 semitone per tick"),
    ("0V", "Vibrato", "x = speed, y = depth; 00 stops"),
    ("0T", "Tremolo", "x = speed, y = depth; 00 stops"),
    ("0A", "Arpeggio", "cycle note, +x, +y semitones every tick; 00 stops"),
    ("0W", "Volume slide", "x = up, y = down, in 1/64 per tick; 00 stops"),
    ("0I", "Fade in", "ramp the note from silence to full over xx ticks"),
    ("0O", "Fade out", "ramp the note to silence over xx ticks, then end it"),
    ("0M", "Note volume", "set the playing note's volume to xx/80 (hex); 80 = full"),
    ("0N", "Auto pan", "x = speed, y = depth; 00 stops"),
    ("0Y", "Maybe", "in a note column: the note plays with probability xx/FF"),
    ("0L", "Track volume", "set the track's volume to xx/80 (hex) until play or stop; 80 = unity"),
    ("0P", "Track pan", "set the track's pan, 00 = left, 80 = center, FF = right, until play or stop"),
    ("0X", "Stop track", "cut every note playing on the track"),
    (
        "0H",
        "Track width",
        "set the track's stereo width, 00 = mono, 80 = unchanged, FF = wide, until play or stop",
    ),
    ("0K", "Pan slide", "x = left, y = right, in 1/64 per tick; 00 stops"),
    (
        "0E",
        "Envelope position",
        "x = envelope (0 all, 1 volume, 2 pan, 3 pitch, 4 cutoff), y = position in sixteenths of its length",
    ),
    (
        "ZJ",
        "Jump to slot",
        "after this line continue at line 0 of sequence slot xx (decimal slot number shown in hex)",
    ),
    ("ZK", "Set ticks per line", "xx = ticks per line"),
    ("M1", "Macro 1", "move the instrument's macro 1 knob (M2 to M4 likewise): 00 = 0, FF = 1"),
];

/// Letter commands that fit in a note column's volume and panning bytes. Values `0x81 + 16 * letter + digit`
/// (`0x81..=0xF0`) are commands; `0x00..=0x80` are plain volumes (or pannings) and `0xFF` is empty.
pub const COLUMN_CMD_LETTERS: [char; 7] = ['G', 'U', 'D', 'I', 'O', 'B', 'C'];

/// What each column command does, for the help window.
pub const COLUMN_CMDS: &[(char, &str, &str)] = &[
    ('G', "Glide", "glide to this note, digit x = speed (x * 4 / 16 semitone per tick)"),
    ('U', "Slide up", "pitch slide up, digit x = speed (x * 2 / 16 semitone per tick)"),
    ('D', "Slide down", "pitch slide down, digit x = speed (x * 2 / 16 semitone per tick)"),
    ('I', "Fade in", "fade the note in over x + 1 ticks"),
    ('O', "Fade out", "fade the note out over x + 1 ticks"),
    ('B', "Backwards", "play the note backwards (digit ignored)"),
    ('C', "Cut", "cut the note after x ticks"),
];

/// Split a volume or panning byte into (command letter index, digit) when it is a command.
pub fn column_cmd_decode(b: u8) -> Option<(usize, u8)> {
    if !(0x81..=0xF0).contains(&b) {
        return None;
    }
    let v = b - 0x81;
    Some(((v / 16) as usize, v % 16))
}

/// The byte for command `letter` (`G`, `U`, ...; case does not matter) with digit `x` (0..=15).
pub fn column_cmd_encode(letter: char, x: u8) -> Option<u8> {
    let i = COLUMN_CMD_LETTERS.iter().position(|c| *c == letter.to_ascii_uppercase())?;
    Some(0x81 + (i as u8) * 16 + x.min(15))
}

/// Text for a volume/panning byte: two hex digits, a letter command like `G4`, or `..` when empty.
pub fn column_text(b: u8, empty: u8) -> String {
    if b == empty {
        return "..".into();
    }
    match column_cmd_decode(b) {
        Some((i, x)) => format!("{}{:X}", COLUMN_CMD_LETTERS[i], x),
        None => format!("{b:02X}"),
    }
}

/// The per-tick effect (or trigger option) a column command stands for: `Fx` for effects that run on the
/// note, `Backwards` and `Cut` for the ones the sequencer handles when it starts the note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnCmd {
    Fx([u8; 2], u8),
    Backwards,
    Cut(u8),
}

pub fn column_cmd(b: u8) -> Option<ColumnCmd> {
    let (i, x) = column_cmd_decode(b)?;
    Some(match COLUMN_CMD_LETTERS[i] {
        'G' => ColumnCmd::Fx(*b"0G", x * 4),
        'U' => ColumnCmd::Fx(*b"0U", x * 2),
        'D' => ColumnCmd::Fx(*b"0D", x * 2),
        'I' => ColumnCmd::Fx(*b"0I", x + 1),
        'O' => ColumnCmd::Fx(*b"0O", x + 1),
        'B' => ColumnCmd::Backwards,
        _ => ColumnCmd::Cut(x),
    })
}

#[cfg(test)]
mod column_tests {
    use super::*;

    #[test]
    fn column_commands_roundtrip_and_never_collide_with_volumes() {
        assert_eq!(column_cmd_decode(0x80), None, "0x80 is full volume");
        assert_eq!(column_cmd_decode(0xFF), None);
        for (i, l) in COLUMN_CMD_LETTERS.iter().enumerate() {
            for x in 0..16u8 {
                let b = column_cmd_encode(*l, x).unwrap();
                assert!((0x81..=0xF0).contains(&b));
                assert_eq!(column_cmd_decode(b), Some((i, x)));
                assert_eq!(column_text(b, 0xFF), format!("{l}{x:X}"));
            }
        }
        assert_eq!(column_cmd_encode('g', 4), column_cmd_encode('G', 4));
        assert_eq!(column_cmd_encode('x', 4), None);
        assert_eq!(column_text(0x40, 0xFF), "40");
        assert_eq!(column_text(0xFF, 0xFF), "..");
        assert_eq!(column_cmd(column_cmd_encode('U', 3).unwrap()), Some(ColumnCmd::Fx(*b"0U", 6)));
        assert_eq!(column_cmd(column_cmd_encode('C', 2).unwrap()), Some(ColumnCmd::Cut(2)));
        assert_eq!(column_cmd(column_cmd_encode('B', 0).unwrap()), Some(ColumnCmd::Backwards));
        assert_eq!(column_cmd(0x40), None);
        assert_eq!(COLUMN_CMDS.len(), COLUMN_CMD_LETTERS.len());
    }
}
