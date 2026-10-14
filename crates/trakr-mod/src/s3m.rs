//! Scream Tracker 3 (`.s3m`) importer.
//!
//! Channels that are enabled in the header become tracks (left channels pan left, right channels right).
//! Only the PCM channels are read; Adlib instruments and channels are skipped with a warning.
//! Timing follows the MOD importer: LPB 4, `ZK` carries the speed and `ZT` the BPM that keeps a row's real
//! length (`6 * tempo / speed`). Effects: A speed, T tempo, C break, D volume slide, E/F pitch slides,
//! G glide, H vibrato, J arpeggio, O sample offset, Q retrigger, R tremolo, SCx cut, SDx delay.

use super::{fx, slide_amount, ModError};
use std::sync::Arc;
use trakr_core::{
    EffectCmd, Instrument, LoopMode, Pattern, Sample, Song, Track, TrackKind, MAX_NOTE_COLUMNS, NOTE_EMPTY,
    NOTE_OFF,
};

struct Reader<'a> {
    b: &'a [u8],
}

impl<'a> Reader<'a> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.b.get(at).copied()
    }
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_le_bytes([*self.b.get(at)?, *self.b.get(at + 1)?]))
    }
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.b.get(at..at + 4)?.try_into().ok()?))
    }
    fn text(&self, at: usize, len: usize) -> String {
        let s = self.b.get(at..(at + len).min(self.b.len())).unwrap_or(&[]);
        String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string()
    }
}

fn short() -> ModError {
    ModError::Format("s3m file is cut short")
}

pub fn import(bytes: &[u8]) -> Result<(Song, Vec<String>), ModError> {
    let r = Reader { b: bytes };
    if bytes.len() < 0x60 || &bytes[0x2C..0x30] != b"SCRM" {
        return Err(ModError::Format("not a Scream Tracker 3 module"));
    }
    let mut warnings = Vec::new();
    let ord_num = r.u16(0x20).ok_or_else(short)? as usize;
    let ins_num = r.u16(0x22).ok_or_else(short)? as usize;
    let pat_num = r.u16(0x24).ok_or_else(short)? as usize;
    let signed = r.u16(0x2A).ok_or_else(short)? == 1;
    let speed0 = r.u8(0x31).ok_or_else(short)?.clamp(1, 255);
    let tempo0 = r.u8(0x32).ok_or_else(short)?.max(32);
    if ord_num > 256 || ins_num > 255 || pat_num > 255 {
        return Err(ModError::Format("implausible s3m header"));
    }
    // Channels: setting < 16 is a PCM channel (0-7 left, 8-15 right); 128 and up are disabled.
    let mut channel_track = [usize::MAX; 32];
    let mut tracks = Vec::new();
    #[allow(clippy::needless_range_loop)] // `ch` is also the channel number
    for ch in 0..32 {
        let setting = r.u8(0x40 + ch).ok_or_else(short)?;
        if setting < 16 {
            channel_track[ch] = tracks.len();
            let mut t = Track::new(format!("Channel {:02}", ch + 1), TrackKind::Sequencer);
            t.panning = if setting < 8 { -0.5 } else { 0.5 };
            tracks.push(t);
        } else if setting < 128 {
            warnings.push(format!("channel {} is an Adlib channel and is skipped", ch + 1));
        }
        if tracks.len() >= MAX_NOTE_COLUMNS * 4 {
            break;
        }
    }
    if tracks.is_empty() {
        return Err(ModError::Format("s3m has no PCM channels"));
    }
    let orders: Vec<u8> = bytes.get(0x60..0x60 + ord_num).ok_or_else(short)?.to_vec();
    let ins_ptrs: Vec<usize> = (0..ins_num)
        .map(|i| r.u16(0x60 + ord_num + i * 2).map(|p| p as usize * 16))
        .collect::<Option<_>>()
        .ok_or_else(short)?;
    let pat_ptrs: Vec<usize> = (0..pat_num)
        .map(|i| r.u16(0x60 + ord_num + ins_num * 2 + i * 2).map(|p| p as usize * 16))
        .collect::<Option<_>>()
        .ok_or_else(short)?;

    let mut song = Song {
        name: r.text(0, 28),
        bpm: (6.0 * tempo0 as f32 / speed0 as f32).round().clamp(32.0, 255.0),
        lpb: 4,
        tpl: speed0.min(48),
        ..Song::default()
    };
    song.patterns.clear();
    song.tracks = tracks;
    song.tracks.push(Track::new("Master", TrackKind::Master));
    song.tracks.last_mut().expect("just pushed").visible_note_columns = 0;
    let ntracks = song.tracks.len();

    // Instruments (samples).
    let mut frames_of = vec![0usize; ins_num];
    for (i, &at) in ins_ptrs.iter().enumerate() {
        let name = r.text(at + 0x30, 28);
        let mut inst = Instrument::new(if name.is_empty() { format!("Sample {:02}", i + 1) } else { name });
        let kind = r.u8(at);
        if at == 0 || kind.is_none() {
            song.instruments.push(inst);
            continue;
        }
        if kind != Some(1) {
            if kind.is_some_and(|k| k > 1) {
                warnings.push(format!("instrument {:02} is an Adlib instrument and is skipped", i + 1));
            }
            song.instruments.push(inst);
            continue;
        }
        let data_at =
            (((r.u8(at + 0x0D).unwrap_or(0) as usize) << 16) | r.u16(at + 0x0E).unwrap_or(0) as usize) * 16;
        let len = r.u32(at + 0x10).unwrap_or(0) as usize;
        let (loop_start, loop_end) =
            (r.u32(at + 0x14).unwrap_or(0) as usize, r.u32(at + 0x18).unwrap_or(0) as usize);
        let volume = r.u8(at + 0x1C).unwrap_or(64).min(64);
        let flags = r.u8(at + 0x1F).unwrap_or(0);
        let c2spd = r.u32(at + 0x20).unwrap_or(8363).clamp(100, 400_000);
        let (stereo, wide) = (flags & 2 != 0, flags & 4 != 0);
        let bytes_per = if wide { 2 } else { 1 };
        let channels = if stereo { 2 } else { 1 };
        let avail = bytes.len().saturating_sub(data_at);
        let frames = len.min(avail / (bytes_per * channels)).min(50_000_000);
        if frames < len {
            warnings.push(format!("sample {:02} is cut short in the file", i + 1));
        }
        if frames > 2 {
            let read = |ch: usize, f: usize| -> f32 {
                let o = data_at + (ch * frames + f) * bytes_per;
                if wide {
                    let v = u16::from_le_bytes([bytes[o], bytes[o + 1]]);
                    if signed {
                        v as i16 as f32 / 32768.0
                    } else {
                        (v as f32 - 32768.0) / 32768.0
                    }
                } else if signed {
                    bytes[o] as i8 as f32 / 128.0
                } else {
                    (bytes[o] as f32 - 128.0) / 128.0
                }
            };
            let mut data = Vec::with_capacity(frames * channels);
            for f in 0..frames {
                for ch in 0..channels {
                    data.push(read(ch, f));
                }
            }
            let mut s = Sample::new(inst.name.clone(), c2spd, channels as u8, data);
            s.base_note = 48;
            s.volume = volume as f32 / 64.0;
            if flags & 1 != 0 && loop_end > loop_start + 1 && loop_start < frames {
                s.loop_mode = LoopMode::Forward;
                s.loop_start = loop_start;
                s.loop_end = loop_end.min(frames);
            }
            frames_of[i] = frames;
            inst.samples.push(s);
        }
        song.instruments.push(inst);
    }

    // Patterns.
    let mut speed = speed0;
    let mut tempo = tempo0;
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    let mut last_inst = [0u8; 32];
    for &pp in &pat_ptrs {
        let mut pat = Pattern::new(ntracks, 64);
        if pp != 0 {
            if let Some(packed) = r.u16(pp) {
                let end = (pp + packed as usize).min(bytes.len());
                let mut at = pp + 2;
                let mut row = 0u16;
                while at < end && row < 64 {
                    let what = bytes[at];
                    at += 1;
                    if what == 0 {
                        row += 1;
                        continue;
                    }
                    let ch = (what & 31) as usize;
                    let (mut note, mut inst, mut vol, mut cmd, mut info) = (255u8, 0u8, 255u8, 0u8, 0u8);
                    if what & 0x20 != 0 {
                        if at + 2 > end {
                            break;
                        }
                        note = bytes[at];
                        inst = bytes[at + 1];
                        at += 2;
                    }
                    if what & 0x40 != 0 {
                        if at >= end {
                            break;
                        }
                        vol = bytes[at];
                        at += 1;
                    }
                    if what & 0x80 != 0 {
                        if at + 2 > end {
                            break;
                        }
                        cmd = bytes[at];
                        info = bytes[at + 1];
                        at += 2;
                    }
                    let Some(&track) = channel_track.get(ch).filter(|t| **t != usize::MAX) else { continue };
                    let line = pat.line_mut(track, row);
                    let cell = &mut line.notes[0];
                    if note == 254 {
                        cell.note = NOTE_OFF;
                    } else if note < 254 {
                        let n = (note >> 4) as u32 * 12 + (note & 15).min(11) as u32;
                        cell.note = n.min(119) as u8;
                    }
                    if inst > 0 {
                        last_inst[ch] = inst;
                    }
                    if note < 254 && last_inst[ch] > 0 {
                        cell.instrument = last_inst[ch] - 1;
                    }
                    if vol <= 64 {
                        cell.volume = vol * 2;
                    }
                    let has_note = cell.note != NOTE_EMPTY && cell.note != NOTE_OFF;
                    let cur_frames =
                        frames_of.get((last_inst[ch] as usize).wrapping_sub(1)).copied().unwrap_or(0).max(1);
                    let (hi, lo) = (info >> 4, info & 15);
                    let mut out: Option<EffectCmd> = None;
                    match cmd {
                        0 => {}
                        1 if info > 0 => {
                            speed = info.min(48);
                            let bpm = (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8;
                            line.effects[1] = fx(b"ZT", bpm);
                            out = Some(fx(b"ZK", speed));
                        }
                        20 if info >= 32 => {
                            tempo = info;
                            let bpm = (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8;
                            out = Some(fx(b"ZT", bpm));
                        }
                        3 => out = Some(fx(b"ZB", hi * 10 + lo)),
                        4 if info != 0 && hi != 0xF && lo != 0xF => out = Some(fx(b"0W", info)),
                        5 if info != 0 && hi < 0xE => out = Some(fx(b"0D", slide_amount(info))),
                        6 if info != 0 && hi < 0xE => out = Some(fx(b"0U", slide_amount(info))),
                        7 => out = Some(fx(b"0G", slide_amount(info.max(1)))),
                        8 => out = Some(fx(b"0V", info)),
                        10 if info != 0 => out = Some(fx(b"0A", info)),
                        15 => {
                            out = Some(fx(b"0S", ((info as usize * 256 * 256) / cur_frames).min(255) as u8))
                        }
                        17 if lo > 0 => out = Some(fx(b"0R", lo)),
                        18 => out = Some(fx(b"0T", info)),
                        19 => match hi {
                            0xC => out = Some(fx(b"0C", lo)),
                            0xD => out = Some(fx(b"0Q", lo)),
                            _ => *skipped.entry(format!("S{hi:X}x")).or_default() += 1,
                        },
                        2 | 4 | 5 | 6 | 20 => {}
                        c => {
                            *skipped
                                .entry(format!(
                                    "{}{:02X}",
                                    (b'A' + (c.saturating_sub(1)).min(25)) as char,
                                    info
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
        }
        song.patterns.push(Arc::new(pat));
    }
    if song.patterns.is_empty() {
        song.patterns.push(Arc::new(Pattern::new(ntracks, 64)));
    }
    song.sequence = orders
        .iter()
        .take_while(|&&o| o != 255)
        .filter(|&&o| o != 254)
        .map(|&o| o as usize)
        .filter(|&o| o < song.patterns.len())
        .collect();
    if song.sequence.is_empty() {
        song.sequence.push(0);
    }
    if song.bpm.is_nan() {
        song.bpm = 125.0;
    }
    for (name, n) in skipped {
        warnings.push(format!("effect {name} is not supported ({n} use(s) skipped)"));
    }
    Ok((song, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small module: 2 PCM channels (one left, one right), one looping sample, a few notes and effects.
    fn module() -> Vec<u8> {
        let mut v = vec![0u8; 0x60];
        v[..4].copy_from_slice(b"demo");
        v[0x1C] = 0x1A;
        v[0x1D] = 16;
        v[0x20..0x22].copy_from_slice(&3u16.to_le_bytes()); // orders: 0, 254, 255
        v[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
        v[0x24..0x26].copy_from_slice(&1u16.to_le_bytes());
        v[0x2A..0x2C].copy_from_slice(&2u16.to_le_bytes()); // unsigned samples
        v[0x2C..0x30].copy_from_slice(b"SCRM");
        v[0x31] = 4; // speed
        v[0x32] = 150; // tempo
        for c in 0..32 {
            v[0x40 + c] = 255;
        }
        v[0x40] = 0;
        v[0x41] = 8;
        v[0x42] = 16 + 1; // an Adlib channel
        v.extend_from_slice(&[0, 254, 255]); // order list
        let ins_ptr_at = v.len();
        v.extend_from_slice(&[0, 0]); // instrument parapointer (fixed up below)
        let pat_ptr_at = v.len();
        v.extend_from_slice(&[0, 0]);
        while v.len() % 16 != 0 {
            v.push(0);
        }
        // instrument header
        let ins_at = v.len();
        let mut ins = vec![0u8; 0x50];
        ins[0] = 1;
        ins[0x10..0x14].copy_from_slice(&64u32.to_le_bytes());
        ins[0x14..0x18].copy_from_slice(&8u32.to_le_bytes());
        ins[0x18..0x1C].copy_from_slice(&40u32.to_le_bytes());
        ins[0x1C] = 32;
        ins[0x1F] = 1; // loop
        ins[0x20..0x24].copy_from_slice(&16_726u32.to_le_bytes());
        ins[0x30..0x30 + 4].copy_from_slice(b"bass");
        ins[0x4C..0x50].copy_from_slice(b"SCRS");
        v.extend_from_slice(&ins);
        let data_at = v.len();
        v.extend((0..64u32).map(|i| (128.0 + 100.0 * (i as f32 * 0.4).sin()) as u8));
        while v.len() % 16 != 0 {
            v.push(0);
        }
        // pattern
        let pat_at = v.len();
        let mut p: Vec<u8> = Vec::new();
        // row 0: ch0 note C-4 (0x40), inst 1, volume 40; effect A06 (speed 6) and ch1 note OFF
        p.extend_from_slice(&[0x20 | 0x40 | 0x80, 0x40, 1, 40, 1, 6]);
        p.extend_from_slice(&[0x21 | 0x20, 254, 0]);
        p.push(0);
        // row 1: ch0 effect D08 (volume slide down 8)
        p.extend_from_slice(&[0x80, 4, 0x08]);
        p.push(0);
        // row 2: ch1 note E-5 inst 1 + Oxx offset
        p.extend_from_slice(&[0x21 | 0x20 | 0x80, 0x54, 1, 15, 0x20]);
        p.push(0);
        // row 3: pattern break and an unsupported command (K)
        p.extend_from_slice(&[0x80, 3, 0x10]);
        p.extend_from_slice(&[0x81, 11, 0x44]);
        p.push(0);
        let len = (p.len() + 2) as u16;
        v.extend_from_slice(&len.to_le_bytes());
        v.extend_from_slice(&p);
        while v.len() % 16 != 0 {
            v.push(0);
        }
        v[ins_ptr_at..ins_ptr_at + 2].copy_from_slice(&((ins_at / 16) as u16).to_le_bytes());
        v[pat_ptr_at..pat_ptr_at + 2].copy_from_slice(&((pat_at / 16) as u16).to_le_bytes());
        // sample data parapointer (in the instrument header, MemSeg)
        let seg = (data_at / 16) as u32;
        v[ins_at + 0x0D] = (seg >> 16) as u8;
        v[ins_at + 0x0E..ins_at + 0x10].copy_from_slice(&(seg as u16).to_le_bytes());
        v
    }

    #[test]
    fn imports_structure_samples_and_notes() {
        let (song, warnings) = import(&module()).unwrap();
        assert_eq!(song.name, "demo");
        assert_eq!(song.tracks.len(), 3, "two PCM channels and the master");
        assert!(song.tracks[0].panning < 0.0 && song.tracks[1].panning > 0.0);
        assert_eq!(song.sequence, vec![0], "marker and end entries are skipped");
        assert_eq!(song.tpl, 4);
        assert_eq!(song.bpm, 150.0 * 6.0 / 4.0);
        let inst = &song.instruments[0];
        assert_eq!(inst.name, "bass");
        let s = &inst.samples[0];
        assert_eq!((s.sample_rate, s.frames(), s.base_note), (16_726, 64, 48));
        assert_eq!((s.loop_mode, s.loop_start, s.loop_end), (LoopMode::Forward, 8, 40));
        assert_eq!(s.volume, 0.5);
        assert!(s.peak(0, 64) > 0.5, "unsigned data was recentred");

        let p = &song.patterns[0];
        let a = p.line(0, 0).unwrap();
        assert_eq!((a.notes[0].note, a.notes[0].instrument, a.notes[0].volume), (48, 0, 80));
        assert_eq!(a.effects[0], fx(b"ZK", 6));
        assert_eq!(a.effects[1], fx(b"ZT", 150));
        assert_eq!(p.line(1, 0).unwrap().notes[0].note, NOTE_OFF);
        assert_eq!(p.line(0, 1).unwrap().effects[0], fx(b"0W", 8));
        let c = p.line(1, 2).unwrap().notes[0];
        assert_eq!((c.note, c.effect.id), (5 * 12 + 4, *b"0S"));
        assert_eq!(p.line(0, 3).unwrap().effects[0], fx(b"ZB", 10), "break row is BCD");
        assert!(warnings.iter().any(|w| w.contains("Adlib")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("K44")), "{warnings:?}");
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
        assert!(import(&[0u8; 200]).is_err());
        assert!(import(&[]).is_err());
        let good = module();
        let mut seed = 99u32;
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
                assert!(song.tracks.len() >= 2 && song.bpm.is_finite());
            }
        }
    }
}
