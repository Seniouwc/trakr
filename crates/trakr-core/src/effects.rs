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
    ("ZK", "Set ticks per line", "xx = ticks per line"),
];
