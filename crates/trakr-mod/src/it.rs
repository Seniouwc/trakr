//! Impulse Tracker (`.it`) importer.
//!
//! Format details are from memory of the public file description and are only checked against modules
//! built by the tests below, not against files from real trackers. Channels used by the patterns become
//! tracks. Instrument mode (instruments with a note-to-sample table and envelopes) and sample mode both
//! work; compressed samples (IT 2.14 and 2.15) are decoded. Timing follows the other module importers:
//! LPB 4, `ZK` carries the speed and `ZT` the BPM that keeps a row's real length (`6 * tempo / speed`).
//! Not imported: filters, NNA/duplicate checks, pitch envelopes, autovibrato, channel volume, global
//! volume slides, MIDI macros, and most of the `S` commands.

use super::{fx, slide_amount, ModError};
use std::sync::Arc;
use trakr_core::{
    Envelope, Instrument, LoopMode, Nna, Pattern, Sample, Song, Track, TrackKind, MAX_NOTE_COLUMNS, NOTE_OFF,
};

struct Reader<'a> {
    b: &'a [u8],
}

impl Reader<'_> {
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
        let end = s.iter().position(|c| *c == 0).unwrap_or(s.len());
        String::from_utf8_lossy(&s[..end]).trim().to_string()
    }
}

fn short() -> ModError {
    ModError::Format("it file is cut short")
}

/// Reads bits least-significant first from a byte slice; reading past the end yields zeros.
struct Bits<'a> {
    b: &'a [u8],
    at: usize,
    bit: u32,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let byte = self.b.get(self.at).copied().unwrap_or(0);
            v |= (((byte >> self.bit) & 1) as u32) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.at += 1;
            }
        }
        v
    }

    fn exhausted(&self) -> bool {
        self.at >= self.b.len()
    }
}

/// Decode IT 2.14 (`it215` false) or 2.15 compressed sample data: `frames` values of 8 or 16 bits, in
/// blocks of up to 0x8000 values, each block prefixed with its compressed size.
pub(crate) fn decompress(data: &[u8], frames: usize, wide: bool, it215: bool) -> Vec<i32> {
    let mut out = Vec::with_capacity(frames);
    let mut at = 0usize;
    let (start_width, max_width) = if wide { (17u32, 17u32) } else { (9, 9) };
    while out.len() < frames {
        let Some(size) = data.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize) else {
            break;
        };
        at += 2;
        let block = data.get(at..(at + size).min(data.len())).unwrap_or(&[]);
        at += size;
        let mut bits = Bits { b: block, at: 0, bit: 0 };
        let mut left = (frames - out.len()).min(if wide { 0x4000 } else { 0x8000 });
        let mut width = start_width;
        let (mut d1, mut d2) = (0i32, 0i32);
        while left > 0 && !bits.exhausted() {
            let value = bits.read(width);
            if width < 7 {
                // method 1: the value 1 << (width - 1) announces a new width
                if value == 1 << (width - 1) {
                    let nw = bits.read(if wide { 4 } else { 3 }) + 1;
                    width = if nw < width { nw } else { nw + 1 };
                    continue;
                }
            } else if width < max_width {
                // method 2: a short range just above the representable border announces a new width
                let border =
                    if wide { (0xFFFFu32 >> (17 - width)) - 8 } else { (0xFFu32 >> (9 - width)) - 4 };
                if value > border && value <= border + if wide { 16 } else { 8 } {
                    let v = value - border;
                    width = if v < width { v } else { v + 1 };
                    continue;
                }
            } else if width == max_width {
                // method 3: a set top bit announces a new width in the low byte
                if value & (1 << (max_width - 1)) != 0 {
                    width = (value + 1) & 0xFF;
                    if width == 0 || width > max_width {
                        break; // corrupt stream
                    }
                    continue;
                }
            } else {
                break; // corrupt stream
            }
            let v = if wide {
                if width < 16 {
                    let shift = 16 - width;
                    (((value << shift) as u16) as i16 >> shift) as i32
                } else {
                    value as u16 as i16 as i32
                }
            } else if width < 8 {
                let shift = 8 - width;
                (((value << shift) as u8) as i8 >> shift) as i32
            } else {
                value as u8 as i8 as i32
            };
            d1 = d1.wrapping_add(v);
            d2 = d2.wrapping_add(d1);
            out.push(if it215 { d2 } else { d1 });
            left -= 1;
        }
        if left > 0 && size == 0 {
            break;
        }
    }
    out.resize(frames, 0);
    out
}

struct SampleHeader {
    name: String,
    flags: u8,
    volume: u8,
    global_volume: u8,
    cvt: u8,
    length: usize,
    loop_start: usize,
    loop_end: usize,
    c5: u32,
    pointer: usize,
}

fn read_sample_header(r: &Reader, at: usize) -> Option<SampleHeader> {
    if r.b.get(at..at + 4)? != b"IMPS" {
        return None;
    }
    Some(SampleHeader {
        name: r.text(at + 0x14, 26),
        global_volume: r.u8(at + 0x11)?,
        flags: r.u8(at + 0x12)?,
        volume: r.u8(at + 0x13)?,
        cvt: r.u8(at + 0x2E)?,
        length: r.u32(at + 0x30)? as usize,
        loop_start: r.u32(at + 0x34)? as usize,
        loop_end: r.u32(at + 0x38)? as usize,
        c5: r.u32(at + 0x3C)?,
        pointer: r.u32(at + 0x48)? as usize,
    })
}

fn decode_sample(bytes: &[u8], h: &SampleHeader, warnings: &mut Vec<String>) -> Option<Sample> {
    if h.flags & 1 == 0 || h.length == 0 {
        return None;
    }
    let wide = h.flags & 2 != 0;
    let channels = if h.flags & 4 != 0 { 2 } else { 1 };
    let frames = h.length.min(50_000_000);
    let signed = h.cvt & 1 != 0;
    let big = h.cvt & 2 != 0;
    let delta = h.cvt & 4 != 0;
    let compressed = h.flags & 8 != 0;
    let bps = if wide { 2 } else { 1 };
    let src = bytes.get(h.pointer..)?;
    // one value per (channel, frame): all of the left channel first, then the right
    let mut vals = vec![0f32; frames * channels];
    for ch in 0..channels {
        let raw: Vec<i32> = if compressed {
            // each channel is its own run of blocks; find where this channel's data starts
            let mut off = 0usize;
            if ch == 1 {
                let mut produced = 0;
                while produced < frames {
                    let Some(sz) = src.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
                    else {
                        break;
                    };
                    off += 2 + sz;
                    produced += if wide { 0x4000 } else { 0x8000 };
                }
            }
            decompress(src.get(off..).unwrap_or(&[]), frames, wide, h.cvt & 4 != 0)
        } else {
            let base = ch * frames * bps;
            (0..frames)
                .map(|f| {
                    let o = base + f * bps;
                    match src.get(o..o + bps) {
                        Some(b) if wide => {
                            let v = if big {
                                u16::from_be_bytes([b[0], b[1]])
                            } else {
                                u16::from_le_bytes([b[0], b[1]])
                            };
                            if signed {
                                v as i16 as i32
                            } else {
                                v as i32 - 32768
                            }
                        }
                        Some(b) => {
                            if signed {
                                b[0] as i8 as i32
                            } else {
                                b[0] as i32 - 128
                            }
                        }
                        None => 0,
                    }
                })
                .collect()
        };
        let mut acc = 0i32;
        for (f, v) in raw.into_iter().enumerate() {
            let v = if delta && !compressed {
                acc = acc.wrapping_add(v);
                if wide {
                    acc as i16 as i32
                } else {
                    acc as i8 as i32
                }
            } else {
                v
            };
            let scale = if wide { 32768.0 } else { 128.0 };
            vals[f * channels + ch] = (v as f32 / scale).clamp(-1.0, 1.0);
        }
    }
    if !compressed && src.len() < frames * bps * channels {
        warnings.push(format!("sample {:?} is cut short in the file", h.name));
    }
    let mut s = Sample::new(h.name.clone(), h.c5.clamp(100, 400_000), channels as u8, vals);
    s.base_note = 60;
    s.volume = (h.volume.min(64) as f32 / 64.0) * (h.global_volume.min(64) as f32 / 64.0);
    if h.flags & 0x10 != 0 && h.loop_end > h.loop_start + 1 && h.loop_start < frames {
        s.loop_mode = if h.flags & 0x40 != 0 { LoopMode::PingPong } else { LoopMode::Forward };
        s.loop_start = h.loop_start;
        s.loop_end = h.loop_end.min(frames);
    }
    Some(s)
}

/// One IT envelope (82 bytes at `at`) as seconds/value points, plus its sustain and loop.
fn read_envelope(r: &Reader, at: usize, tick_secs: f32, pan: bool) -> Option<Envelope> {
    let flags = r.u8(at)?;
    let n = (r.u8(at + 1)? as usize).min(25);
    if flags & 1 == 0 || n == 0 {
        return None;
    }
    let (lb, le, sb, se) =
        (r.u8(at + 2)? as usize, r.u8(at + 3)? as usize, r.u8(at + 4)? as usize, r.u8(at + 5)? as usize);
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        let y = r.u8(at + 6 + i * 3)? as i8;
        let tick = r.u16(at + 7 + i * 3)? as f32;
        let v = if pan { (y as f32 + 32.0) / 64.0 } else { y as f32 / 64.0 };
        points.push((tick * tick_secs, v.clamp(0.0, 1.0)));
    }
    // ticks must ascend for the engine
    for i in 1..points.len() {
        if points[i].0 < points[i - 1].0 {
            points[i].0 = points[i - 1].0;
        }
    }
    Some(Envelope {
        enabled: true,
        sustain: (flags & 4 != 0 && sb == se && sb < n).then_some(sb),
        loop_points: (flags & 2 != 0 && le > lb && le < n).then_some((lb, le)),
        points,
        curve: Default::default(),
    })
}

pub fn import(bytes: &[u8]) -> Result<(Song, Vec<String>), ModError> {
    let r = Reader { b: bytes };
    if bytes.len() < 0xC0 || &bytes[0..4] != b"IMPM" {
        return Err(ModError::Format("not an Impulse Tracker module"));
    }
    let mut warnings = Vec::new();
    let ord_num = r.u16(0x20).ok_or_else(short)? as usize;
    let ins_num = r.u16(0x22).ok_or_else(short)? as usize;
    let smp_num = r.u16(0x24).ok_or_else(short)? as usize;
    let pat_num = r.u16(0x26).ok_or_else(short)? as usize;
    let flags = r.u16(0x2C).ok_or_else(short)?;
    let speed0 = r.u8(0x32).ok_or_else(short)?.clamp(1, 255);
    let tempo0 = r.u8(0x33).ok_or_else(short)?.max(32);
    if ord_num > 256 || ins_num > 255 || smp_num > 255 || pat_num > 255 {
        return Err(ModError::Format("implausible it header"));
    }
    let use_instruments = flags & 4 != 0;
    let orders = bytes.get(0xC0..0xC0 + ord_num).ok_or_else(short)?.to_vec();
    let base = 0xC0 + ord_num;
    let ptr = |i: usize| r.u32(base + i * 4).map(|p| p as usize);
    let ins_ptrs: Vec<usize> = (0..ins_num).map(ptr).collect::<Option<_>>().ok_or_else(short)?;
    let smp_ptrs: Vec<usize> =
        (0..smp_num).map(|i| ptr(ins_num + i)).collect::<Option<_>>().ok_or_else(short)?;
    let pat_ptrs: Vec<usize> =
        (0..pat_num).map(|i| ptr(ins_num + smp_num + i)).collect::<Option<_>>().ok_or_else(short)?;

    // Which channels the patterns use decides the tracks.
    let mut used = [false; 64];
    for &pp in &pat_ptrs {
        if pp == 0 {
            continue;
        }
        let (Some(len), Some(_rows)) = (r.u16(pp), r.u16(pp + 2)) else { continue };
        let end = (pp + 8 + len as usize).min(bytes.len());
        let mut at = pp + 8;
        let mut mask = [0u8; 64];
        while at < end {
            let cv = bytes[at];
            at += 1;
            if cv == 0 {
                continue;
            }
            let ch = ((cv - 1) & 63) as usize;
            used[ch] = true;
            let m = if cv & 128 != 0 {
                if at >= end {
                    break;
                }
                mask[ch] = bytes[at];
                at += 1;
                mask[ch]
            } else {
                mask[ch]
            };
            at += (m & 1) as usize
                + ((m & 2) >> 1) as usize
                + ((m & 4) >> 2) as usize
                + ((m & 8) >> 3) as usize * 2;
        }
    }
    let mut channel_track = [usize::MAX; 64];
    let mut tracks = Vec::new();
    for ch in 0..64 {
        if used[ch] && tracks.len() < MAX_NOTE_COLUMNS * 4 {
            channel_track[ch] = tracks.len();
            let mut t = Track::new(format!("Channel {:02}", ch + 1), TrackKind::Sequencer);
            let pan = r.u8(0x40 + ch).unwrap_or(32);
            if pan <= 64 {
                t.panning = ((pan as f32 - 32.0) / 32.0).clamp(-1.0, 1.0);
            }
            tracks.push(t);
        }
    }
    if tracks.is_empty() {
        tracks.push(Track::new("Channel 01", TrackKind::Sequencer));
        channel_track[0] = 0;
    }

    let mut song = Song {
        name: r.text(4, 26),
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
    let tick_secs = 2.5 / tempo0 as f32;

    // Samples first: instruments refer to them.
    let mut samples: Vec<Option<Sample>> = Vec::with_capacity(smp_num);
    let mut sample_frames = vec![0usize; smp_num];
    for (i, &at) in smp_ptrs.iter().enumerate() {
        let sample = read_sample_header(&r, at).and_then(|h| decode_sample(bytes, &h, &mut warnings));
        if let Some(s) = &sample {
            sample_frames[i] = s.frames();
        } else if at != 0 {
            warnings.push(format!("sample {:02} has no audio", i + 1));
        }
        samples.push(sample);
    }

    // trakr instruments.
    if use_instruments {
        for (i, &at) in ins_ptrs.iter().enumerate() {
            let name = r.text(at + 0x20, 26);
            let mut inst =
                Instrument::new(if name.is_empty() { format!("Instrument {:02}", i + 1) } else { name });
            if at == 0 || r.b.get(at..at + 4) != Some(b"IMPI") {
                song.instruments.push(inst);
                continue;
            }
            inst.nna = match r.u8(at + 0x11) {
                Some(1) => Nna::Continue,
                Some(2) => Nna::NoteOff,
                _ => Nna::Cut,
            };
            inst.volume = (r.u8(at + 0x18).unwrap_or(128).min(128) as f32 / 128.0).max(0.01);
            let fade = r.u16(at + 0x14).unwrap_or(0);
            if fade > 0 {
                // fade-out is 1/1024 of full volume per tick after note-off, twice as fast per 32 units
                inst.fade_out = (1024.0 / (fade as f32 * 32.0) * tick_secs * 32.0).clamp(0.0, 60.0);
            }
            // note -> sample table: group consecutive notes using the same sample into key ranges
            let mut ranges: Vec<(u8, u8, usize)> = Vec::new();
            for note in 0..120usize {
                let Some(s) = r.u8(at + 0x40 + note * 2 + 1) else { break };
                if s == 0 || s as usize > smp_num {
                    continue;
                }
                let si = s as usize - 1;
                match ranges.last_mut() {
                    Some(last) if last.2 == si && last.1 as usize + 1 == note => last.1 = note as u8,
                    _ => ranges.push((note as u8, note as u8, si)),
                }
            }
            for (lo, hi, si) in ranges {
                if let Some(Some(s)) = samples.get(si) {
                    let mut s = s.clone();
                    s.key_range = (lo, hi);
                    inst.samples.push(s);
                }
            }
            if let Some(e) = read_envelope(&r, at + 0x130, tick_secs, false) {
                inst.volume_env = Arc::new(e);
            }
            if let Some(e) = read_envelope(&r, at + 0x130 + 82, tick_secs, true) {
                inst.pan_env = Arc::new(e);
            }
            if r.u8(at + 0x130 + 164).is_some_and(|f| f & 1 != 0) {
                warnings.push(format!("instrument {:02}: pitch/filter envelopes are not imported", i + 1));
            }
            song.instruments.push(inst);
        }
    } else {
        for (i, s) in samples.iter().enumerate() {
            let mut inst = Instrument::new(
                s.as_ref()
                    .map(|s| s.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Sample {:02}", i + 1)),
            );
            if let Some(s) = s {
                inst.samples.push(s.clone());
            }
            song.instruments.push(inst);
        }
    }

    // Patterns.
    let mut speed = speed0;
    let mut tempo = tempo0;
    let mut skipped = std::collections::BTreeMap::<String, usize>::new();
    for &pp in &pat_ptrs {
        let rows = r.u16(pp + 2).filter(|_| pp != 0).unwrap_or(64).clamp(1, 512);
        let mut pat = Pattern::new(ntracks, rows);
        if pp != 0 {
            if let Some(len) = r.u16(pp) {
                let end = (pp + 8 + len as usize).min(bytes.len());
                let mut at = pp + 8;
                let mut row = 0u16;
                let (mut mask, mut last_note, mut last_ins, mut last_vol, mut last_cmd) =
                    ([0u8; 64], [0u8; 64], [0u8; 64], [255u8; 64], [(0u8, 0u8); 64]);
                while at < end && row < rows {
                    let cv = bytes[at];
                    at += 1;
                    if cv == 0 {
                        row += 1;
                        continue;
                    }
                    let ch = ((cv - 1) & 63) as usize;
                    if cv & 128 != 0 {
                        if at >= end {
                            break;
                        }
                        mask[ch] = bytes[at];
                        at += 1;
                    }
                    let m = mask[ch];
                    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
                        let s = bytes.get(*at..*at + n)?;
                        *at += n;
                        Some(s)
                    };
                    let (mut note, mut ins, mut vol, mut cmd) = (None, None, None, None);
                    if m & 1 != 0 {
                        let Some(b) = take(&mut at, 1) else { break };
                        last_note[ch] = b[0];
                        note = Some(b[0]);
                    } else if m & 0x10 != 0 {
                        note = Some(last_note[ch]);
                    }
                    if m & 2 != 0 {
                        let Some(b) = take(&mut at, 1) else { break };
                        last_ins[ch] = b[0];
                        ins = Some(b[0]);
                    } else if m & 0x20 != 0 {
                        ins = Some(last_ins[ch]);
                    }
                    if m & 4 != 0 {
                        let Some(b) = take(&mut at, 1) else { break };
                        last_vol[ch] = b[0];
                        vol = Some(b[0]);
                    } else if m & 0x40 != 0 {
                        vol = Some(last_vol[ch]);
                    }
                    if m & 8 != 0 {
                        let Some(b) = take(&mut at, 2) else { break };
                        last_cmd[ch] = (b[0], b[1]);
                        cmd = Some((b[0], b[1]));
                    } else if m & 0x80 != 0 {
                        cmd = Some(last_cmd[ch]);
                    }
                    let Some(&track) = channel_track.get(ch).filter(|t| **t != usize::MAX) else { continue };
                    let line = pat.line_mut(track, row);
                    let cell = &mut line.notes[0];
                    let mut has_note = false;
                    if let Some(n) = note {
                        if n < 120 {
                            cell.note = n;
                            has_note = true;
                        } else if n == 254 || n == 255 || n == 253 {
                            cell.note = NOTE_OFF;
                        }
                    }
                    if let Some(i) = ins.filter(|i| *i > 0) {
                        if has_note || note.is_none() {
                            cell.instrument = i - 1;
                        }
                    }
                    if let Some(v) = vol {
                        match v {
                            0..=64 => cell.volume = v * 2,
                            128..=192 => cell.panning = ((v - 128) as u16 * 2).min(0x80) as u8,
                            _ => *skipped.entry("volume column slides".into()).or_default() += 1,
                        }
                    }
                    let Some((c, info)) = cmd else { continue };
                    let (hi, lo) = (info >> 4, info & 15);
                    let cur_frames = (cell.instrument != 0xFF)
                        .then(|| sample_frames.get(cell.instrument as usize).copied())
                        .flatten()
                        .unwrap_or(0)
                        .max(1);
                    let mut out = None;
                    match c {
                        0 => {}
                        1 if info > 0 => {
                            speed = info.min(48);
                            let bpm = (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8;
                            line.effects[1] = fx(b"ZT", bpm);
                            out = Some(fx(b"ZK", speed));
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
                            0x8 => cell.panning = (lo as u16 * 0x80 / 15).min(0x80) as u8,
                            _ => *skipped.entry(format!("S{hi:X}x")).or_default() += 1,
                        },
                        20 if info >= 32 => {
                            tempo = info;
                            let bpm = (6.0 * tempo as f32 / speed as f32).round().clamp(32.0, 255.0) as u8;
                            out = Some(fx(b"ZT", bpm));
                        }
                        24 => cell.panning = (info as u16 * 0x80 / 255).min(0x80) as u8,
                        2 | 4 | 5 | 6 | 20 => {}
                        c => {
                            *skipped
                                .entry(format!(
                                    "{}{:02X}",
                                    (b'A' + c.saturating_sub(1).min(25)) as char,
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
    for (name, n) in skipped {
        warnings.push(format!("effect {name} is not supported ({n} use(s) skipped)"));
    }
    Ok((song, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bit writer matching `Bits`.
    struct W {
        out: Vec<u8>,
        bit: u32,
    }

    impl W {
        fn new() -> Self {
            W { out: vec![], bit: 0 }
        }
        fn put(&mut self, v: u32, n: u32) {
            for i in 0..n {
                if self.bit == 0 {
                    self.out.push(0);
                }
                let last = self.out.len() - 1;
                self.out[last] |= (((v >> i) & 1) as u8) << self.bit;
                self.bit = (self.bit + 1) % 8;
            }
        }
    }

    /// Compress 8-bit samples the simple way: 9-bit codes only (no width changes).
    fn compress8_plain(vals: &[i8]) -> Vec<u8> {
        let mut w = W::new();
        let mut prev = 0i32;
        for &v in vals {
            let d = v as i32 - prev;
            prev = v as i32;
            w.put((d & 0xFF) as u32, 9); // a 9-bit code whose top bit is clear
        }
        let mut data = (w.out.len() as u16).to_le_bytes().to_vec();
        data.extend(w.out);
        data
    }

    #[test]
    fn decompression_follows_deltas_and_width_changes() {
        let vals: Vec<i8> = (0..300).map(|i| ((i as f32 * 0.2).sin() * 100.0) as i8).collect();
        let packed = compress8_plain(&vals);
        let out = decompress(&packed, vals.len(), false, false);
        assert_eq!(out, vals.iter().map(|v| *v as i32).collect::<Vec<_>>());
        // a width switch: at width 9 a code with the top bit set picks width 5 ((0x104 + 1) & 0xFF = 5),
        // then 5-bit deltas follow; the escape value 16 (1 << 4) with a 3-bit payload switches again
        let mut w = W::new();
        w.put(0x100 | 4, 9); // new width 5
        for d in [3i32, -2, 4, 1] {
            w.put((d & 0x1F) as u32, 5);
        }
        w.put(16, 5); // method 1 escape
        w.put(2, 3); // new width = 3 + 1 -> 3 is below 5, so width 3
        w.put(1, 3);
        w.put(0b111, 3); // -1
        let mut data = (w.out.len() as u16).to_le_bytes().to_vec();
        data.extend(w.out);
        let out = decompress(&data, 6, false, false);
        assert_eq!(out, vec![3, 1, 5, 6, 7, 6]);
        // IT 2.15 integrates once more
        let out215 = decompress(&data, 6, false, true);
        assert_eq!(out215, vec![3, 4, 9, 15, 22, 28]);
        // garbage and truncated data never panic and always fill the requested length
        assert_eq!(decompress(&[1, 0, 0xFF], 10, false, false).len(), 10);
        assert_eq!(decompress(&[], 5, true, false), vec![0; 5]);
    }

    /// Build a small module in sample mode: 2 channels, one 8-bit looping sample, a few notes and effects.
    fn module(instrument_mode: bool, compressed: bool) -> Vec<u8> {
        let mut v = vec![0u8; 0xC0];
        v[0..4].copy_from_slice(b"IMPM");
        v[4..8].copy_from_slice(b"demo");
        let (ords, ins, smp, pats) = (3usize, instrument_mode as usize, 1usize, 1usize);
        v[0x20..0x22].copy_from_slice(&(ords as u16).to_le_bytes());
        v[0x22..0x24].copy_from_slice(&(ins as u16).to_le_bytes());
        v[0x24..0x26].copy_from_slice(&(smp as u16).to_le_bytes());
        v[0x26..0x28].copy_from_slice(&(pats as u16).to_le_bytes());
        v[0x2C] = if instrument_mode { 4 } else { 0 };
        v[0x32] = 4; // speed
        v[0x33] = 150; // tempo
        for c in 0..64 {
            v[0x40 + c] = 32;
        }
        v[0x40] = 0; // channel 1 left
        v[0x41] = 64; // channel 2 right
        v.extend([0, 254, 255]); // orders
        let ptr_at = v.len();
        v.extend(vec![0u8; (ins + smp + pats) * 4]);
        // instrument
        let ins_at = v.len();
        if instrument_mode {
            let mut i = vec![0u8; 0x130 + 82 * 3];
            i[0..4].copy_from_slice(b"IMPI");
            i[0x11] = 2; // NNA: note off
            i[0x18] = 128;
            i[0x20..0x24].copy_from_slice(b"lead");
            for n in 0..120 {
                i[0x40 + n * 2] = n as u8;
                i[0x40 + n * 2 + 1] = if n < 60 { 0 } else { 1 }; // notes from C-5 up play sample 1
            }
            // volume envelope: on, sustain at node 1, three nodes
            let e = 0x130;
            i[e] = 1 | 4;
            i[e + 1] = 3;
            i[e + 4] = 1;
            i[e + 5] = 1;
            for (k, (y, t)) in [(0i8, 0u16), (64, 10), (0, 30)].into_iter().enumerate() {
                i[e + 6 + k * 3] = y as u8;
                i[e + 7 + k * 3..e + 9 + k * 3].copy_from_slice(&t.to_le_bytes());
            }
            v.extend(i);
        }
        // sample header
        let smp_at = v.len();
        let data: Vec<i8> = (0..64).map(|i| ((i as f32 * 0.4).sin() * 90.0) as i8).collect();
        let mut s = vec![0u8; 0x50];
        s[0..4].copy_from_slice(b"IMPS");
        s[0x11] = 64; // global volume
        s[0x12] = 1 | 0x10 | if compressed { 8 } else { 0 }; // has data, loop
        s[0x13] = 32; // volume
        s[0x14..0x17].copy_from_slice(b"saw");
        s[0x2E] = 1; // signed
        s[0x30..0x34].copy_from_slice(&64u32.to_le_bytes());
        s[0x34..0x38].copy_from_slice(&8u32.to_le_bytes());
        s[0x38..0x3C].copy_from_slice(&40u32.to_le_bytes());
        s[0x3C..0x40].copy_from_slice(&8363u32.to_le_bytes());
        let data_at = smp_at + 0x50;
        s[0x48..0x4C].copy_from_slice(&(data_at as u32).to_le_bytes());
        v.extend(s);
        if compressed {
            v.extend(compress8_plain(&data));
        } else {
            v.extend(data.iter().map(|b| *b as u8));
        }
        // pattern: 8 rows
        let pat_at = v.len();
        let mut p: Vec<u8> = Vec::new();
        // row 0: ch1 note C-5 (60), instrument 1, volume 48, command A03 (speed 3)
        p.extend([1 | 128, 1 | 2 | 4 | 8, 60, 1, 48, 1, 3]);
        // ch2 note E-5 with only a note and instrument (mask reused for the next line)
        p.extend([2 | 128, 1 | 2, 64, 1]);
        p.push(0);
        // row 1: ch1 repeats last mask: note off
        p.extend([1 | 128, 1, 255]);
        p.push(0);
        // row 2: ch2 uses the previous mask (note + instrument) again
        p.extend([2, 67, 1]);
        p.push(0);
        // row 3: ch1 panning column (set to 128 -> pan 0) with a G07 style command (8: vibrato H)
        p.extend([1 | 128, 4 | 8, 128 + 64, 8, 0x42]);
        p.push(0);
        p.extend([0u8; 4]);
        let mut pat = (p.len() as u16).to_le_bytes().to_vec();
        pat.extend(8u16.to_le_bytes());
        pat.extend([0; 4]);
        pat.extend(p);
        v.extend(pat);
        let mut at = ptr_at;
        if instrument_mode {
            v[at..at + 4].copy_from_slice(&(ins_at as u32).to_le_bytes());
            at += 4;
        }
        v[at..at + 4].copy_from_slice(&(smp_at as u32).to_le_bytes());
        at += 4;
        v[at..at + 4].copy_from_slice(&(pat_at as u32).to_le_bytes());
        v
    }

    #[test]
    fn sample_mode_modules_import_notes_effects_and_audio() {
        for compressed in [false, true] {
            let (song, warnings) = import(&module(false, compressed)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(song.name, "demo");
            assert_eq!(song.tracks.len(), 3, "two used channels plus the master");
            assert!(song.tracks[0].panning < 0.0 && song.tracks[1].panning > 0.0);
            assert_eq!(song.sequence, vec![0], "separators and the end marker are skipped");
            assert_eq!(song.patterns[0].number_of_lines, 8);
            assert_eq!(song.instruments.len(), 1);
            let s = &song.instruments[0].samples[0];
            assert_eq!((s.frames(), s.sample_rate, s.base_note), (64, 8363, 60));
            assert_eq!((s.loop_mode, s.loop_start, s.loop_end), (LoopMode::Forward, 8, 40));
            assert!((s.volume - 0.5).abs() < 1e-6);
            assert!(s.data.iter().any(|v| v.abs() > 0.5), "audio decoded (compressed: {compressed})");
            let p = &song.patterns[0];
            let c = p.line(0, 0).unwrap();
            assert_eq!((c.notes[0].note, c.notes[0].instrument, c.notes[0].volume), (60, 0, 96));
            assert_eq!((&c.effects[0].id, c.effects[0].amount), (b"ZK", 3), "A03 sets the speed");
            assert_eq!(p.line(1, 0).unwrap().notes[0].note, 64);
            assert_eq!(p.line(0, 1).unwrap().notes[0].note, NOTE_OFF);
            assert_eq!(p.line(1, 2).unwrap().notes[0].note, 67, "the repeated mask applies to the new note");
            let row3 = p.line(0, 3).unwrap();
            assert_eq!(row3.notes[0].panning, 0x80, "panning column 64 is hard right");
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    #[test]
    fn instrument_mode_maps_notes_to_samples_and_reads_envelopes() {
        let (song, _w) = import(&module(true, false)).unwrap();
        let inst = &song.instruments[0];
        assert_eq!(inst.name, "lead");
        assert_eq!(inst.nna, Nna::NoteOff);
        assert_eq!(inst.samples.len(), 1, "notes 60 and up map to the one sample");
        assert_eq!(inst.samples[0].key_range, (60, 119));
        let env = &inst.volume_env;
        assert!(env.enabled && env.points.len() == 3);
        assert_eq!(env.sustain, Some(1));
        assert!((env.points[1].1 - 1.0).abs() < 1e-6);
        assert!(
            (env.points[1].0 - 10.0 * 2.5 / 150.0).abs() < 1e-5,
            "ticks become seconds at the start tempo"
        );
    }

    #[test]
    fn damaged_files_error_or_import_without_panicking() {
        assert!(import(b"IMPM").is_err());
        assert!(import(&vec![0u8; 400]).is_err());
        let good = module(true, true);
        // truncate at every length and flip bytes: nothing may panic
        for cut in (0..good.len()).step_by(7) {
            let _ = import(&good[..cut]);
        }
        let mut x = 0x2545_F491u32;
        for _ in 0..300 {
            let mut bad = good.clone();
            for _ in 0..4 {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                let i = (x >> 8) as usize % bad.len();
                bad[i] = (x >> 3) as u8;
            }
            let _ = import(&bad);
        }
    }
}
