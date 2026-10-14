//! ProTracker-style `.mod` importer (4, 6, 8, ... channel Amiga modules).
//!
//! Layout: 20-byte title, 31 sample headers (30 bytes), song length, restart byte, 128-entry order table,
//! 4-byte tag, then 64-row patterns of 4 bytes per channel, then 8-bit signed sample data.
//!
//! Mapping notes:
//! - Amiga period index 0 (PT C-1) becomes internal note 24, so PT C-2 (the sample's native 8363 Hz) is note 36.
//! - Timing keeps each line's real duration: LPB is 4, TPL is the first speed command, and tempo `F20..FFF`
//!   becomes `BPM = 6 * tempo / speed`.
//! - Effects without an equivalent are skipped and counted in the warnings.

use std::sync::Arc;
use thiserror::Error;
use trakr_core::{
    EffectCmd, Instrument, LoopMode, Pattern, Sample, Song, Track, TrackKind, EMPTY, MAX_NOTE_COLUMNS,
    NOTE_EMPTY,
};

#[derive(Debug, Error)]
pub enum ModError {
    #[error("not a recognised module: {0}")]
    Format(&'static str),
    #[error("file ends early (needed {needed} bytes, have {have})")]
    Truncated { needed: usize, have: usize },
}

/// Amiga period table for finetune 0, C-1 to B-3.
const PERIODS: [u16; 36] = [
    856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269,
    254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113,
];

const HEADER: usize = 20 + 31 * 30 + 2 + 128;
const TAG_AT: usize = HEADER;
const PATTERNS_AT: usize = HEADER + 4;
const NATIVE_RATE: u32 = 8363;
const NATIVE_NOTE: u8 = 36;

fn channels_for(tag: &[u8]) -> Option<usize> {
    match tag {
        b"M.K." | b"M!K!" | b"FLT4" | b"4CHN" => Some(4),
        b"6CHN" => Some(6),
        b"8CHN" | b"OCTA" | b"CD81" | b"FLT8" => Some(8),
        [a, b, b'C', b'H'] | [a, b, b'C', b'N'] if a.is_ascii_digit() && b.is_ascii_digit() => {
            Some(((a - b'0') * 10 + (b - b'0')) as usize)
        }
        _ => None,
    }
}

fn period_to_note(period: u16) -> Option<u8> {
    if period == 0 {
        return None;
    }
    // Nearest table entry (finetuned modules use slightly different periods).
    PERIODS
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| (**p as i32 - period as i32).abs())
        .map(|(i, _)| 24 + i as u8)
}

fn fx(id: &[u8; 2], amount: u8) -> EffectCmd {
    EffectCmd { id: *id, amount }
}

/// Convert a MOD slide speed (Amiga period units per tick) to trakr's 1/16 semitone per tick.
fn slide_amount(xx: u8) -> u8 {
    ((xx as f32) * 0.65).round().clamp(1.0, 255.0) as u8
}

/// Import a module. Returns the song and a list of things that could not be represented.
pub fn import(bytes: &[u8]) -> Result<(Song, Vec<String>), ModError> {
    if bytes.len() < PATTERNS_AT {
        return Err(ModError::Format("too short for a module header"));
    }
    let channels = channels_for(&bytes[TAG_AT..TAG_AT + 4]).ok_or(ModError::Format("unknown channel tag"))?;
    if channels > MAX_NOTE_COLUMNS * 4 || channels == 0 {
        return Err(ModError::Format("unsupported channel count"));
    }
    let mut warnings = Vec::new();
    let title = String::from_utf8_lossy(&bytes[..20]).trim_end_matches('\0').trim().to_string();
    let song_len = (bytes[20 + 31 * 30] as usize).clamp(1, 128);
    let order = &bytes[20 + 31 * 30 + 2..20 + 31 * 30 + 2 + 128];
    let pattern_count = order.iter().copied().max().unwrap_or(0) as usize + 1;
    let pattern_bytes = 64 * channels * 4;
    let samples_at = PATTERNS_AT + pattern_count * pattern_bytes;
    if bytes.len() < samples_at {
        return Err(ModError::Truncated { needed: samples_at, have: bytes.len() });
    }

    // Sample headers and data.
    struct Head {
        name: String,
        len: usize,
        finetune: i8,
        volume: u8,
        loop_start: usize,
        loop_len: usize,
    }
    let mut heads = Vec::new();
    for i in 0..31 {
        let h = &bytes[20 + i * 30..20 + (i + 1) * 30];
        let ft = (h[24] & 0x0F) as i8;
        heads.push(Head {
            name: String::from_utf8_lossy(&h[..22]).trim_end_matches('\0').trim().to_string(),
            len: u16::from_be_bytes([h[22], h[23]]) as usize * 2,
            finetune: if ft > 7 { ft - 16 } else { ft },
            volume: h[25].min(64),
            loop_start: u16::from_be_bytes([h[26], h[27]]) as usize * 2,
            loop_len: u16::from_be_bytes([h[28], h[29]]) as usize * 2,
        });
    }

    let mut song = Song { name: title, bpm: 125.0, lpb: 4, tpl: 6, ..Song::default() };
    song.patterns.clear();
    song.tracks = (0..channels)
        .map(|c| {
            let mut t = Track::new(format!("Channel {:02}", c + 1), TrackKind::Sequencer);
            // Amiga hard panning: channels go left, right, right, left.
            t.panning = if matches!(c % 4, 0 | 3) { -0.6 } else { 0.6 };
            t
        })
        .collect();
    song.tracks.push(Track::new("Master", TrackKind::Master));
    song.tracks.last_mut().expect("just pushed").visible_note_columns = 0;

    let mut at = samples_at;
    let mut frames_of = [0usize; 31];
    for (i, h) in heads.iter().enumerate() {
        let take = h.len.min(bytes.len().saturating_sub(at));
        if take < h.len {
            warnings.push(format!("sample {:02} is cut short in the file", i + 1));
        }
        let mut inst =
            Instrument::new(if h.name.is_empty() { format!("Sample {:02}", i + 1) } else { h.name.clone() });
        if take > 2 {
            let data: Vec<f32> = bytes[at..at + take].iter().map(|&b| (b as i8) as f32 / 128.0).collect();
            let mut s = Sample::new(inst.name.clone(), NATIVE_RATE, 1, data);
            s.base_note = NATIVE_NOTE;
            s.fine_tune = (h.finetune as i32 * 16).clamp(-128, 127) as i8;
            s.volume = h.volume as f32 / 64.0;
            if h.loop_len > 2 && h.loop_start + h.loop_len <= take + 2 {
                s.loop_mode = LoopMode::Forward;
                s.loop_start = h.loop_start.min(take);
                s.loop_end = (h.loop_start + h.loop_len).min(take);
            }
            frames_of[i] = take;
            inst.samples.push(s);
        }
        at += take;
        song.instruments.push(inst);
    }

    // Patterns.
    let ntracks = song.tracks.len();
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    let mut first_speed: Option<u8> = None;
    let mut speed = 6u8;
    let mut tempo = 125u8;
    let mut last_sample = [0usize; 64];
    for p in 0..pattern_count {
        let mut pat = Pattern::new(ntracks, 64);
        for row in 0..64usize {
            #[allow(clippy::needless_range_loop)] // `ch` indexes the file bytes and the track list too
            for ch in 0..channels.min(64) {
                let o = PATTERNS_AT + p * pattern_bytes + (row * channels + ch) * 4;
                let b = &bytes[o..o + 4];
                let period = (((b[0] & 0x0F) as u16) << 8) | b[1] as u16;
                let sample = ((b[0] & 0xF0) | (b[2] >> 4)) as usize;
                let (effect, param) = (b[2] & 0x0F, b[3]);
                if period == 0 && sample == 0 && effect == 0 && param == 0 {
                    continue;
                }
                let line = pat.line_mut(ch, row as u16);
                let cell = &mut line.notes[0];
                if let Some(n) = period_to_note(period) {
                    cell.note = n;
                    if sample > 0 {
                        last_sample[ch] = sample;
                    }
                    if last_sample[ch] > 0 {
                        cell.instrument = (last_sample[ch] - 1) as u8;
                    }
                } else if sample > 0 {
                    last_sample[ch] = sample;
                }
                let has_note = cell.note != NOTE_EMPTY;
                let cur_frames = frames_of.get(last_sample[ch].wrapping_sub(1)).copied().unwrap_or(0).max(1);
                let (hi, lo) = (param >> 4, param & 15);
                let mut out: Option<EffectCmd> = None;
                match effect {
                    0x0 if param != 0 => out = Some(fx(b"0A", param)),
                    0x1 if param != 0 => out = Some(fx(b"0U", slide_amount(param))),
                    0x2 if param != 0 => out = Some(fx(b"0D", slide_amount(param))),
                    0x3 => out = Some(fx(b"0G", slide_amount(param.max(1)))),
                    0x4 => out = Some(fx(b"0V", param)),
                    0x7 => out = Some(fx(b"0T", param)),
                    0x8 => {
                        if has_note {
                            cell.panning = ((param as u16 * 0x80) / 255) as u8;
                        }
                    }
                    0x9 => out = Some(fx(b"0S", ((param as usize * 256 * 256) / cur_frames).min(255) as u8)),
                    0xA if param != 0 => out = Some(fx(b"0W", param)),
                    0xC => cell.volume = param.min(64) * 2,
                    0xD => out = Some(fx(b"ZB", hi * 10 + lo)),
                    0xE => match hi {
                        0x9 if lo > 0 => out = Some(fx(b"0R", lo)),
                        0xC => out = Some(fx(b"0C", lo)),
                        0xD => out = Some(fx(b"0Q", lo)),
                        _ => *skipped.entry(format!("E{hi:X}x")).or_default() += 1,
                    },
                    0xF if param != 0 => {
                        if param < 32 {
                            speed = param;
                            first_speed.get_or_insert(param);
                        } else {
                            tempo = param;
                        }
                        let bpm = (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8;
                        out = Some(fx(b"ZT", bpm));
                    }
                    0x0 | 0x1 | 0x2 | 0xA | 0xF => {}
                    0x5 | 0x6 | 0xB => *skipped.entry(format!("{effect:X}xx")).or_default() += 1,
                    _ => {}
                }
                if let Some(e) = out {
                    // Per-note slot when there is a note, else a track effect column.
                    if has_note
                        && matches!(&e.id, b"0S" | b"0G" | b"0A" | b"0V" | b"0T" | b"0R" | b"0C" | b"0Q")
                    {
                        cell.effect = e;
                    } else {
                        line.effects[0] = e;
                    }
                }
            }
        }
        song.patterns.push(Arc::new(pat));
    }
    song.sequence = order[..song_len].iter().map(|&p| p as usize).collect();
    if let Some(s) = first_speed {
        song.tpl = s;
    }
    for (name, n) in skipped {
        warnings.push(format!("effect {name} is not supported ({n} uses skipped)"));
    }
    let _ = (EMPTY, NOTE_EMPTY);
    Ok((song, warnings))
}

pub fn import_path(path: &std::path::Path) -> Result<(Song, Vec<String>), Box<dyn std::error::Error>> {
    Ok(import(&std::fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 4-channel module: one pattern, one looping 100-frame sample, a few notes and effects.
    fn module() -> Vec<u8> {
        let mut v = vec![0u8; PATTERNS_AT];
        v[..4].copy_from_slice(b"test");
        // sample 1: 50 words (100 bytes), finetune +2, volume 32, loop from word 10 for 20 words
        let h = 20;
        v[h..h + 3].copy_from_slice(b"saw");
        v[h + 22..h + 24].copy_from_slice(&50u16.to_be_bytes());
        v[h + 24] = 2;
        v[h + 25] = 32;
        v[h + 26..h + 28].copy_from_slice(&10u16.to_be_bytes());
        v[h + 28..h + 30].copy_from_slice(&20u16.to_be_bytes());
        v[20 + 31 * 30] = 2; // song length
        v[20 + 31 * 30 + 2] = 0; // order: pattern 0, 0
        v[TAG_AT..TAG_AT + 4].copy_from_slice(b"M.K.");
        let mut pat = vec![0u8; 64 * 4 * 4];
        let mut cell = |row: usize, ch: usize, period: u16, sample: u8, effect: u8, param: u8| {
            let o = (row * 4 + ch) * 4;
            pat[o] = (sample & 0xF0) | ((period >> 8) as u8 & 0x0F);
            pat[o + 1] = period as u8;
            pat[o + 2] = ((sample & 0x0F) << 4) | effect;
            pat[o + 3] = param;
        };
        cell(0, 0, 428, 1, 0xF, 0x03); // PT C-2 on ch 0, speed 3
        cell(0, 1, 214, 1, 0xC, 0x20); // PT C-3, set volume 32/64
        cell(2, 0, 0, 0, 0x1, 0x08); // slide up
        cell(4, 2, 508, 0, 0x9, 0x10); // PT A-1... sample offset (reuses sample 1? none yet -> note without sample)
        cell(6, 3, 428, 1, 0xB, 0x01); // position jump: unsupported
        cell(8, 0, 0, 0, 0xD, 0x12); // pattern break to row 12
        v.extend_from_slice(&pat);
        v.extend((0..100).map(|i| (i as i8).wrapping_mul(2) as u8)); // sample data
        v
    }

    #[test]
    fn imports_structure_notes_and_samples() {
        let (song, warnings) = import(&module()).unwrap();
        assert_eq!(song.name, "test");
        assert_eq!(song.tracks.len(), 5, "4 channels + master");
        assert_eq!(song.sequence, vec![0, 0]);
        assert_eq!(song.instruments.len(), 31);
        assert_eq!(song.tpl, 3, "first speed command sets TPL");
        let inst = &song.instruments[0];
        assert_eq!(inst.name, "saw");
        let s = &inst.samples[0];
        assert_eq!((s.sample_rate, s.frames(), s.base_note), (8363, 100, 36));
        assert_eq!(s.fine_tune, 32);
        assert_eq!(s.volume, 0.5);
        assert_eq!((s.loop_mode, s.loop_start, s.loop_end), (LoopMode::Forward, 20, 60));
        assert!(song.instruments[1].samples.is_empty());

        let p = &song.patterns[0];
        let c = p.line(0, 0).unwrap().notes[0];
        assert_eq!((c.note, c.instrument), (36, 0), "PT C-2 is internal note 36");
        assert_eq!(p.line(0, 0).unwrap().effects[0], fx(b"ZT", 250));
        let hi = p.line(1, 0).unwrap().notes[0];
        assert_eq!((hi.note, hi.volume), (48, 64), "an octave up; Cxx 32 -> volume column 0x40");
        assert_eq!(p.line(0, 2).unwrap().effects[0].id, *b"0U");
        let brk = p.line(0, 8).unwrap().effects[0];
        assert_eq!((brk.id, brk.amount), (*b"ZB", 12), "Dxx is BCD");
        assert!(warnings.iter().any(|w| w.contains("Bxx")), "{warnings:?}");
    }

    #[test]
    fn pans_like_an_amiga_and_rejects_garbage() {
        let (song, _) = import(&module()).unwrap();
        assert!(song.tracks[0].panning < 0.0 && song.tracks[1].panning > 0.0);
        assert!(song.tracks[2].panning > 0.0 && song.tracks[3].panning < 0.0);
        assert!(import(&[0u8; 100]).is_err());
        let mut bad = module();
        bad[TAG_AT..TAG_AT + 4].copy_from_slice(b"????");
        assert!(matches!(import(&bad), Err(ModError::Format(_))));
        let mut short = module();
        short.truncate(PATTERNS_AT + 10);
        assert!(matches!(import(&short), Err(ModError::Truncated { .. })));
    }

    #[test]
    fn more_channels_and_period_matching() {
        assert_eq!(channels_for(b"8CHN"), Some(8));
        assert_eq!(channels_for(b"12CH"), Some(12));
        assert_eq!(channels_for(b"abcd"), None);
        assert_eq!(period_to_note(856), Some(24));
        assert_eq!(period_to_note(113), Some(59));
        assert_eq!(period_to_note(425), Some(36), "finetuned periods snap to the nearest note");
        assert_eq!(period_to_note(0), None);
    }

    #[test]
    fn imported_module_plays() {
        let (song, _) = import(&module()).unwrap();
        assert!(song.sequence.iter().all(|&i| i < song.patterns.len()));
        assert!(song.patterns.iter().all(|p| p.number_of_lines == 64));
        let out = trakr_engine::render_song(Arc::new(song), 44_100, 20.0);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.05, "the module should be audible, peak {peak}");
        assert!(out.len() / 2 > 44_100, "and last longer than a second");
    }
}
