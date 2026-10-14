//! FastTracker 2 (`.xm`) importer.
//!
//! Each XM instrument becomes an instrument whose samples are mapped to keys like the original keymap.
//! Delta-coded 8 and 16 bit samples are decoded; relative note and finetune set the sample's pitch.
//! Timing follows the MOD importer (`ZK` for speed, `ZT` for `6 * BPM / speed`). Envelopes, autovibrato,
//! fadeout, ping-pong volume column commands and a few effects are not imported and are reported.

use super::{fx, slide_amount, ModError};
use std::sync::Arc;
use trakr_core::{
    EffectCmd, Instrument, LoopMode, Pattern, Sample, Song, Track, TrackKind, MAX_NOTE_COLUMNS, NOTE_EMPTY,
    NOTE_OFF,
};

fn short() -> ModError {
    ModError::Format("xm file is cut short")
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.0.get(at).copied()
    }
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes([*self.0.get(at)?, *self.0.get(at + 1)?]))
    }
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.0.get(at..at + 4)?.try_into().ok()?))
    }
    fn text(&self, at: usize, len: usize) -> String {
        let s = self.0.get(at..(at + len).min(self.0.len())).unwrap_or(&[]);
        String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string()
    }
}

pub fn import(bytes: &[u8]) -> Result<(Song, Vec<String>), ModError> {
    let r = R(bytes);
    if !bytes.starts_with(b"Extended Module: ") || bytes.len() < 80 {
        return Err(ModError::Format("not a FastTracker 2 module"));
    }
    let mut warnings = Vec::new();
    let header_size = r.u32(60).ok_or_else(short)? as usize;
    let song_len = (r.u16(64).ok_or_else(short)? as usize).clamp(1, 256);
    let channels = r.u16(68).ok_or_else(short)? as usize;
    let n_patterns = r.u16(70).ok_or_else(short)? as usize;
    let n_instruments = r.u16(72).ok_or_else(short)? as usize;
    let speed0 = (r.u16(76).ok_or_else(short)? as u8).clamp(1, 48);
    let bpm0 = r.u16(78).ok_or_else(short)?.clamp(32, 255) as f32;
    if channels == 0 || channels > MAX_NOTE_COLUMNS * 4 || n_patterns > 256 || n_instruments > 128 {
        return Err(ModError::Format("implausible xm header"));
    }
    let orders = bytes.get(80..80 + 256).ok_or_else(short)?;

    let mut song = Song {
        name: r.text(17, 20),
        bpm: (6.0 * bpm0 / speed0 as f32).round().clamp(32.0, 255.0),
        lpb: 4,
        tpl: speed0,
        ..Song::default()
    };
    song.patterns.clear();
    song.tracks =
        (0..channels).map(|c| Track::new(format!("Channel {:02}", c + 1), TrackKind::Sequencer)).collect();
    song.tracks.push(Track::new("Master", TrackKind::Master));
    song.tracks.last_mut().expect("just pushed").visible_note_columns = 0;
    let ntracks = song.tracks.len();

    // Patterns come first in the file; they refer to instruments by number, so remember where they end
    // and decode them after the instruments (the sample offset command needs sample lengths).
    let mut at = 60 + header_size;
    let mut pattern_spans = Vec::new();
    for _ in 0..n_patterns {
        let hlen = r.u32(at).ok_or_else(short)? as usize;
        let rows = r.u16(at + 5).ok_or_else(short)? as usize;
        let packed = r.u16(at + 7).ok_or_else(short)? as usize;
        let data = at + hlen;
        if hlen < 9 || rows == 0 || rows > 256 || data + packed > bytes.len() {
            return Err(short());
        }
        pattern_spans.push((rows, data, data + packed));
        at = data + packed;
    }

    // Instruments.
    let mut frames_of = vec![0usize; n_instruments];
    let mut env_warned = false;
    #[allow(clippy::needless_range_loop)]
    for ii in 0..n_instruments {
        let isize = r.u32(at).ok_or_else(short)? as usize;
        let name = r.text(at + 4, 22);
        let n_samples = r.u16(at + 27).ok_or_else(short)? as usize;
        let mut inst =
            Instrument::new(if name.is_empty() { format!("Instrument {:02}", ii + 1) } else { name });
        if isize < 29 || n_samples > 16 {
            return Err(ModError::Format("implausible xm instrument"));
        }
        let mut next = at + isize;
        if n_samples > 0 {
            let sh_size = r.u32(at + 29).ok_or_else(short)? as usize;
            if sh_size < 40 {
                return Err(ModError::Format("implausible xm sample header"));
            }
            let keymap: Vec<u8> = bytes.get(at + 33..at + 33 + 96).ok_or_else(short)?.to_vec();
            if (r.u8(at + 233).unwrap_or(0) | r.u8(at + 234).unwrap_or(0)) & 1 != 0 && !env_warned {
                warnings.push("volume/panning envelopes are not imported".into());
                env_warned = true;
            }
            let heads = next;
            let mut data_at = heads + n_samples * sh_size;
            for si in 0..n_samples {
                let h = heads + si * sh_size;
                let len = r.u32(h).ok_or_else(short)? as usize;
                let (ls, ll) =
                    (r.u32(h + 4).ok_or_else(short)? as usize, r.u32(h + 8).ok_or_else(short)? as usize);
                let volume = r.u8(h + 12).ok_or_else(short)?.min(64);
                let finetune = r.u8(h + 13).ok_or_else(short)? as i8;
                let ty = r.u8(h + 14).ok_or_else(short)?;
                let pan = r.u8(h + 15).ok_or_else(short)?;
                let rel = r.u8(h + 16).ok_or_else(short)? as i8;
                let sname = r.text(h + 18, 22);
                let wide = ty & 0x10 != 0;
                let bpf = if wide { 2 } else { 1 };
                let avail = bytes.len().saturating_sub(data_at);
                let take = len.min(avail);
                if take < len {
                    warnings.push(format!("instrument {:02} sample {} is cut short", ii + 1, si + 1));
                }
                let frames = take / bpf;
                if frames > 2 {
                    let mut acc: i32 = 0;
                    let data: Vec<f32> = (0..frames)
                        .map(|f| {
                            if wide {
                                let v =
                                    i16::from_le_bytes([bytes[data_at + f * 2], bytes[data_at + f * 2 + 1]]);
                                acc = (acc + v as i32) as i16 as i32;
                                acc as f32 / 32768.0
                            } else {
                                acc = (acc + bytes[data_at + f] as i8 as i32) as i8 as i32;
                                acc as f32 / 128.0
                            }
                        })
                        .collect();
                    let mut s =
                        Sample::new(if sname.is_empty() { inst.name.clone() } else { sname }, 8363, 1, data);
                    s.base_note = (48 - rel as i32).clamp(0, 119) as u8;
                    s.fine_tune = finetune;
                    s.volume = volume as f32 / 64.0;
                    s.panning = (pan as f32 - 128.0) / 128.0;
                    let (ls, ll) = (ls / bpf, ll / bpf);
                    match ty & 3 {
                        1 | 2 if ll > 1 && ls < frames => {
                            s.loop_mode = if ty & 3 == 2 { LoopMode::PingPong } else { LoopMode::Forward };
                            s.loop_start = ls;
                            s.loop_end = (ls + ll).min(frames);
                        }
                        _ => {}
                    }
                    // keys that play this sample (XM note n is internal n - 1)
                    let keys: Vec<usize> = (0..96).filter(|&n| keymap[n] as usize == si).collect();
                    s.key_range = match (keys.first(), keys.last()) {
                        (Some(&lo), Some(&hi)) => (lo as u8, hi as u8),
                        _ if n_samples == 1 => (0, 119),
                        _ => (1, 0), // not on any key
                    };
                    if si == 0 || frames_of[ii] == 0 {
                        frames_of[ii] = frames;
                    }
                    inst.samples.push(s);
                }
                data_at += take;
            }
            next = data_at;
        }
        song.instruments.push(inst);
        at = next;
    }

    // Patterns.
    let mut speed = speed0;
    let mut tempo = bpm0 as u8;
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    let mut last_inst = [0u8; 64];
    for &(rows, data, end) in &pattern_spans {
        let mut pat = Pattern::new(ntracks, rows as u16);
        let mut p = data;
        for row in 0..rows {
            #[allow(clippy::needless_range_loop)]
            for ch in 0..channels {
                if p >= end {
                    break;
                }
                let first = bytes[p];
                let (mut note, mut inst, mut vol, mut eff, mut param) = (0u8, 0u8, 0u8, 0u8, 0u8);
                if first & 0x80 != 0 {
                    p += 1;
                    for (bit, slot) in
                        [(1, &mut note), (2, &mut inst), (4, &mut vol), (8, &mut eff), (16, &mut param)]
                    {
                        if first & bit != 0 {
                            if p >= end {
                                break;
                            }
                            *slot = bytes[p];
                            p += 1;
                        }
                    }
                } else {
                    if p + 5 > end {
                        break;
                    }
                    (note, inst, vol, eff, param) =
                        (bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3], bytes[p + 4]);
                    p += 5;
                }
                if note == 0 && inst == 0 && vol == 0 && eff == 0 && param == 0 {
                    continue;
                }
                let line = pat.line_mut(ch, row as u16);
                let cell = &mut line.notes[0];
                if (1..=96).contains(&note) {
                    cell.note = note - 1;
                } else if note == 97 {
                    cell.note = NOTE_OFF;
                }
                if inst > 0 {
                    last_inst[ch] = inst;
                }
                if (1..=96).contains(&note) && (1..=128).contains(&last_inst[ch]) {
                    cell.instrument = last_inst[ch] - 1;
                }
                match vol {
                    0x10..=0x50 => cell.volume = (vol - 0x10) * 2,
                    0xC0..=0xCF => cell.panning = ((vol & 15) as u16 * 0x80 / 15) as u8,
                    0 => {}
                    v => *skipped.entry(format!("volume column {:X}x", v >> 4)).or_default() += 1,
                }
                let has_note = cell.note != NOTE_EMPTY && cell.note != NOTE_OFF;
                let frames =
                    frames_of.get((last_inst[ch] as usize).wrapping_sub(1)).copied().unwrap_or(0).max(1);
                let (hi, lo) = (param >> 4, param & 15);
                let slide = |n: u8| (n / 4).max(1);
                let mut out: Option<EffectCmd> = None;
                match eff {
                    0 if param != 0 => out = Some(fx(b"0A", param)),
                    1 if param != 0 => out = Some(fx(b"0U", slide(param))),
                    2 if param != 0 => out = Some(fx(b"0D", slide(param))),
                    3 => {
                        out = Some(fx(b"0G", slide_amount(param.max(1)).min(slide(param.max(4)).max(1) * 4)))
                    }
                    4 => out = Some(fx(b"0V", param)),
                    7 => out = Some(fx(b"0T", param)),
                    8 => cell.panning = ((param as u16 * 0x80) / 255) as u8,
                    9 => out = Some(fx(b"0S", ((param as usize * 256 * 256) / frames).min(255) as u8)),
                    0xA if param != 0 => out = Some(fx(b"0W", param)),
                    0xC => cell.volume = param.min(64) * 2,
                    0xD => out = Some(fx(b"ZB", hi * 10 + lo)),
                    0xE => match hi {
                        9 if lo > 0 => out = Some(fx(b"0R", lo)),
                        0xC => out = Some(fx(b"0C", lo)),
                        0xD => out = Some(fx(b"0Q", lo)),
                        _ => *skipped.entry(format!("E{hi:X}x")).or_default() += 1,
                    },
                    0xF if param != 0 => {
                        if param < 32 {
                            speed = param.min(48);
                            line.effects[1] = fx(
                                b"ZT",
                                (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8,
                            );
                            out = Some(fx(b"ZK", speed));
                        } else {
                            tempo = param;
                            out = Some(fx(
                                b"ZT",
                                (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8,
                            ));
                        }
                    }
                    20 => {
                        if cell.note == NOTE_EMPTY {
                            cell.note = NOTE_OFF;
                        }
                    }
                    0 | 1 | 2 | 0xA | 0xF => {}
                    e => {
                        *skipped
                            .entry(format!(
                                "{}xx",
                                char::from_digit(e as u32, 36).unwrap_or('?').to_ascii_uppercase()
                            ))
                            .or_default() += 1
                    }
                }
                if let Some(e) = out {
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
    if song.patterns.is_empty() {
        song.patterns.push(Arc::new(Pattern::new(ntracks, 64)));
    }
    song.sequence =
        orders[..song_len].iter().map(|&o| o as usize).filter(|&o| o < song.patterns.len()).collect();
    if song.sequence.is_empty() {
        song.sequence.push(0);
    }
    for (name, n) in skipped {
        warnings.push(format!("effect {name} is not supported ({n} use(s) skipped)"));
    }
    Ok((song, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two channels, one pattern of 8 rows, one instrument with two key-split samples (8-bit and 16-bit).
    fn module() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"Extended Module: ");
        v.extend_from_slice(&{
            let mut t = [0u8; 20];
            t[..4].copy_from_slice(b"song");
            t
        });
        v.push(0x1A);
        v.extend_from_slice(&[0u8; 20]); // tracker name
        v.extend_from_slice(&0x0104u16.to_le_bytes());
        v.extend_from_slice(&276u32.to_le_bytes()); // header size
        v.extend_from_slice(&2u16.to_le_bytes()); // song length
        v.extend_from_slice(&0u16.to_le_bytes()); // restart
        v.extend_from_slice(&2u16.to_le_bytes()); // channels
        v.extend_from_slice(&1u16.to_le_bytes()); // patterns
        v.extend_from_slice(&1u16.to_le_bytes()); // instruments
        v.extend_from_slice(&1u16.to_le_bytes()); // flags
        v.extend_from_slice(&5u16.to_le_bytes()); // speed
        v.extend_from_slice(&150u16.to_le_bytes()); // bpm
        let mut orders = [0u8; 256];
        orders[1] = 0;
        v.extend_from_slice(&orders);
        // pattern
        let mut data: Vec<u8> = Vec::new();
        // row 0: ch0 C-4 (49) inst 1 vol 0x30, ch1 note C-5 (61) with only note+inst flags (compressed)
        data.extend_from_slice(&[49, 1, 0x30, 0, 0]);
        data.extend_from_slice(&[0x80 | 1 | 2, 61, 1]);
        // row 1: ch0 effect F03 (speed 3) compressed; ch1 empty
        data.extend_from_slice(&[0x80 | 8 | 16, 0x0F, 0x03]);
        data.extend_from_slice(&[0x80]);
        // row 2: ch0 key off (97) + volume pan C8; ch1 effect C20 (set volume 32)
        data.extend_from_slice(&[0x80 | 1 | 4, 97, 0xC8]);
        data.extend_from_slice(&[0x80 | 8 | 16, 0x0C, 32]);
        // row 3: ch0 effect 4 vibrato 0x48; ch1 unsupported effect 'H' (17)
        data.extend_from_slice(&[0x80 | 8 | 16, 4, 0x48]);
        data.extend_from_slice(&[0x80 | 8 | 16, 17, 0x01]);
        // rows 4-7 empty
        for _ in 4..8 {
            data.extend_from_slice(&[0x80, 0x80]);
        }
        v.extend_from_slice(&9u32.to_le_bytes());
        v.push(0);
        v.extend_from_slice(&8u16.to_le_bytes());
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(&data);
        // instrument with 2 samples
        let mut ins = vec![0u8; 263];
        ins[..4].copy_from_slice(&263u32.to_le_bytes());
        ins[4..9].copy_from_slice(b"split");
        ins[27..29].copy_from_slice(&2u16.to_le_bytes());
        ins[29..33].copy_from_slice(&40u32.to_le_bytes());
        for n in 48..96 {
            ins[33 + n] = 1; // upper keys play the second sample
        }
        ins[233] = 1; // volume envelope on: reported, not imported
        v.extend_from_slice(&ins);
        let sample_head =
            |len: u32, ls: u32, ll: u32, vol: u8, ft: i8, ty: u8, pan: u8, rel: i8, name: &[u8]| {
                let mut h = vec![0u8; 40];
                h[..4].copy_from_slice(&len.to_le_bytes());
                h[4..8].copy_from_slice(&ls.to_le_bytes());
                h[8..12].copy_from_slice(&ll.to_le_bytes());
                h[12] = vol;
                h[13] = ft as u8;
                h[14] = ty;
                h[15] = pan;
                h[16] = rel as u8;
                h[18..18 + name.len()].copy_from_slice(name);
                h
            };
        v.extend_from_slice(&sample_head(32, 8, 16, 64, 5, 1, 255, -2, b"low"));
        v.extend_from_slice(&sample_head(64, 0, 0, 32, 0, 0x10, 128, 0, b"high"));
        // data: delta coded. low: a constant +4 step; high (16-bit): +1000 per frame for 32 frames
        v.extend((0..32).map(|_| 4u8));
        for _ in 0..32 {
            v.extend_from_slice(&1000i16.to_le_bytes());
        }
        v
    }

    #[test]
    fn imports_notes_samples_and_keymaps() {
        let (song, warnings) = import(&module()).unwrap();
        assert_eq!(song.name, "song");
        assert_eq!((song.tpl, song.bpm), (5, 6.0 * 150.0 / 5.0));
        assert_eq!(song.tracks.len(), 3);
        assert_eq!(song.sequence, vec![0, 0]);
        let inst = &song.instruments[0];
        assert_eq!((inst.name.as_str(), inst.samples.len()), ("split", 2));
        let (lo, hi) = (&inst.samples[0], &inst.samples[1]);
        assert_eq!((lo.name.as_str(), lo.frames(), lo.sample_rate), ("low", 32, 8363));
        assert_eq!(lo.base_note, 50, "relative note -2 lowers the pitch reference");
        assert_eq!((lo.fine_tune, lo.volume), (5, 1.0));
        assert_eq!((lo.loop_mode, lo.loop_start, lo.loop_end), (LoopMode::Forward, 8, 24));
        assert!(lo.panning > 0.9);
        assert_eq!(lo.key_range, (0, 47));
        assert_eq!(hi.key_range, (48, 95));
        // delta decoding: the low sample rises by 4/128 per frame, the high one by 1000/32768
        assert!((lo.data[3] - 16.0 / 128.0).abs() < 1e-6, "{}", lo.data[3]);
        assert!((hi.data[2] - 3000.0 / 32768.0).abs() < 1e-6);
        assert_eq!(inst.sample_for(60).unwrap().name, "high");
        assert_eq!(inst.sample_for(10).unwrap().name, "low");

        let p = &song.patterns[0];
        assert_eq!(p.number_of_lines, 8);
        let a = p.line(0, 0).unwrap().notes[0];
        assert_eq!((a.note, a.instrument, a.volume), (48, 0, 64));
        assert_eq!(p.line(1, 0).unwrap().notes[0].note, 60);
        let b = p.line(0, 1).unwrap();
        assert_eq!(b.effects[0], fx(b"ZK", 3));
        assert_eq!(b.effects[1], fx(b"ZT", 255), "300 BPM is clamped to the largest ZT value");
        let c = p.line(0, 2).unwrap().notes[0];
        assert_eq!((c.note, c.panning), (NOTE_OFF, 0x40 + 0x04));
        assert_eq!(p.line(1, 2).unwrap().notes[0].volume, 64, "Cxx 32 -> volume column 0x40");
        assert!(warnings.iter().any(|w| w.contains("envelopes")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("effect H")), "{warnings:?}");
    }

    #[test]
    fn imported_module_renders_audio() {
        let (song, _) = crate::import_any(&module()).unwrap(); // content sniffing picks this importer
        assert_eq!(song.name, import(&module()).unwrap().0.name);
        let out = trakr_engine::render_song(std::sync::Arc::new(song), 44_100, 3.0);
        assert!(out.len() > 1000 && out.iter().all(|s| s.is_finite()), "{} frames", out.len() / 2);
        assert!(out.iter().any(|s| s.abs() > 0.01), "silent");
    }

    #[test]
    fn rejects_garbage_and_survives_corruption() {
        assert!(import(&[0u8; 300]).is_err());
        assert!(import(b"Extended Module: ").is_err());
        let good = module();
        let mut seed = 5u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        for _ in 0..3000 {
            let mut b = good.clone();
            for _ in 0..1 + rnd() % 8 {
                let i = rnd() as usize % b.len();
                b[i] = rnd() as u8;
            }
            if rnd() % 4 == 0 {
                b.truncate(rnd() as usize % b.len());
            }
            if let Ok((song, _)) = import(&b) {
                assert!(!song.patterns.is_empty() && !song.sequence.is_empty());
                assert!(song.sequence.iter().all(|&p| p < song.patterns.len()));
                assert!(song.bpm.is_finite() && song.tracks.len() >= 2);
            }
        }
    }
}
