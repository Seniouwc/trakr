use crate::envelope::Envelope;
use crate::instrument::{Instrument, LoopMode, Sample};
use crate::note::{parse_note, EMPTY, NOTE_OFF};
use crate::pattern::{Pattern, DEFAULT_LINES};
use crate::track::{Track, TrackKind};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Per-slot extras that sit beside the play order (one entry per sequence slot).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlotMeta {
    /// Bit `t` set: track `t` is silent while this slot plays.
    #[serde(default)]
    pub mutes: u64,
    /// A section starts at this slot, with this name.
    #[serde(default)]
    pub section: Option<String>,
    /// How many times the section that starts here plays before the song moves on (0 or 1 = once).
    #[serde(default)]
    pub repeat: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    pub author: String,
    pub comments: String,
    pub bpm: f32,
    pub lpb: u8,
    pub tpl: u8,
    /// Song track order. Sequencer tracks first, master after them, sends last.
    pub tracks: Vec<Track>,
    /// Shared so that snapshots sent to the engine are cheap and an edit clones only the touched pattern.
    pub patterns: Vec<Arc<Pattern>>,
    /// Pattern indices in play order.
    pub sequence: Vec<usize>,
    pub instruments: Vec<Instrument>,
    /// Inclusive range of sequence slots to loop during playback.
    #[serde(default)]
    pub loop_range: Option<(usize, usize)>,
    /// Parallel to `sequence` (shorter is fine: missing entries are defaults).
    #[serde(default)]
    pub slot_meta: Vec<SlotMeta>,
    /// Shuffle: how far each line of a repeating group starts late (+) or early (-), as a fraction of a
    /// line. Empty = straight timing.
    #[serde(default)]
    pub groove: Vec<f32>,
}

impl Default for Song {
    fn default() -> Self {
        let mut tracks: Vec<Track> =
            (1..=8).map(|i| Track::new(format!("Track {i:02}"), TrackKind::Sequencer)).collect();
        let mut master = Track::new("Master", TrackKind::Master);
        master.visible_note_columns = 0;
        tracks.push(master);
        let n = tracks.len();
        Self {
            name: "Untitled".into(),
            author: String::new(),
            comments: String::new(),
            bpm: 120.0,
            lpb: 4,
            tpl: 12,
            tracks,
            patterns: vec![Arc::new(Pattern::new(n, DEFAULT_LINES))],
            sequence: vec![0],
            instruments: Vec::new(),
            loop_range: None,
            slot_meta: Vec::new(),
            groove: Vec::new(),
        }
    }
}

/// Longest groove cycle.
pub const MAX_GROOVE: usize = 8;

impl Song {
    /// Whether `track` is muted in sequence slot `slot`.
    pub fn slot_muted(&self, slot: usize, track: usize) -> bool {
        track < 64 && self.slot_meta.get(slot).is_some_and(|m| m.mutes >> track & 1 != 0)
    }

    /// Make `slot_meta` exactly as long as the sequence.
    pub fn normalize_slot_meta(&mut self) {
        self.slot_meta.resize(self.sequence.len(), SlotMeta::default());
    }

    pub fn insert_slot(&mut self, at: usize, pattern: usize, meta: SlotMeta) {
        self.normalize_slot_meta();
        let at = at.min(self.sequence.len());
        self.sequence.insert(at, pattern);
        self.slot_meta.insert(at, meta);
    }

    pub fn remove_slot(&mut self, at: usize) {
        self.normalize_slot_meta();
        if at < self.sequence.len() {
            self.sequence.remove(at);
            self.slot_meta.remove(at);
        }
    }

    pub fn swap_slots(&mut self, a: usize, b: usize) {
        self.normalize_slot_meta();
        self.sequence.swap(a, b);
        self.slot_meta.swap(a, b);
    }

    /// Sections as (first slot, last slot, name). Slots before the first marker belong to no section.
    pub fn sections(&self) -> Vec<(usize, usize, String)> {
        let marks: Vec<(usize, &str)> = self
            .slot_meta
            .iter()
            .enumerate()
            .take(self.sequence.len())
            .filter_map(|(i, m)| m.section.as_deref().map(|n| (i, n)))
            .collect();
        marks
            .iter()
            .enumerate()
            .map(|(k, (start, name))| {
                let end = marks.get(k + 1).map_or(self.sequence.len(), |n| n.0).saturating_sub(1);
                (*start, end.max(*start), name.to_string())
            })
            .collect()
    }

    /// Times the section starting at `slot` plays (at least once).
    pub fn section_repeats(&self, slot: usize) -> u32 {
        self.slot_meta.get(slot).map_or(1, |m| m.repeat.max(1) as u32)
    }

    /// The section a slot belongs to, if any: (first slot, last slot, name).
    pub fn section_of(&self, slot: usize) -> Option<(usize, usize, String)> {
        self.sections().into_iter().find(|(a, b, _)| (*a..=*b).contains(&slot))
    }

    /// Length of line `line` relative to a straight line (1.0), following the groove.
    pub fn groove_scale(&self, line: u16) -> f64 {
        let n = self.groove.len().min(MAX_GROOVE);
        if n == 0 {
            return 1.0;
        }
        let g = |l: usize| {
            let v = self.groove[l % n] as f64;
            if v.is_finite() {
                v.clamp(-0.45, 0.45)
            } else {
                0.0
            }
        };
        1.0 + g(line as usize + 1) - g(line as usize)
    }

    /// Frames per pattern line at the given rate and tempo.
    pub fn frames_per_line(bpm: f32, lpb: u8, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / (bpm as f64 * lpb.max(1) as f64)
    }

    pub fn pattern_at(&self, seq_pos: usize) -> Option<&Pattern> {
        self.patterns.get(*self.sequence.get(seq_pos)?).map(|p| &**p)
    }

    /// A small audible song: one decaying sine instrument and an arpeggio.
    pub fn demo() -> Self {
        let mut song = Song { name: "Demo".into(), ..Song::default() };
        let rate = 44_100u32;
        let data: Vec<f32> = (0..rate / 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (t * 261.63 * std::f32::consts::TAU).sin() * (-t * 6.0).exp() * 0.6
            })
            .collect();
        let mut inst = Instrument::new("Sine C-4");
        inst.samples.push(Sample::new("sine", rate, 1, data));
        song.instruments.push(inst);

        let pat = Arc::make_mut(&mut song.patterns[0]);
        for (i, name) in ["C-4", "E-4", "G-4", "C-5"].iter().cycle().take(16).enumerate() {
            let col = &mut pat.line_mut(0, (i * 4) as u16).notes[0];
            col.note = parse_note(name).unwrap();
            col.instrument = 0;
            col.volume = EMPTY;
        }
        song
    }

    /// A short, complete song for first-time users: drums, bass and a lead, all synthesized, with a
    /// reverb send and a limiter on the master. Shows off patterns, instruments, the mixer and devices.
    pub fn showcase() -> Self {
        use std::f32::consts::TAU;
        const RATE: u32 = 44_100;
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 8_388_608.0 - 1.0
        };
        let secs = |s: f32| (RATE as f32 * s) as usize;
        let mk = |name: &str, base: u8, data: Vec<f32>| {
            let mut i = Instrument::new(name);
            let mut s = Sample::new(name, RATE, 1, data);
            s.base_note = base;
            i.samples.push(s);
            i
        };

        // Kick: a sine that falls from ~150 Hz to 45 Hz.
        let mut phase = 0.0f32;
        let kick: Vec<f32> = (0..secs(0.4))
            .map(|i| {
                let t = i as f32 / RATE as f32;
                phase += TAU * (45.0 + 110.0 * (-t * 18.0).exp()) / RATE as f32;
                (phase.sin() * (-t * 8.0).exp() * 0.95).clamp(-1.0, 1.0)
            })
            .collect();
        // Snare: noise burst over a short tone.
        let snare: Vec<f32> = (0..secs(0.3))
            .map(|i| {
                let t = i as f32 / RATE as f32;
                (noise() * (-t * 14.0).exp() * 0.6 + (t * 190.0 * TAU).sin() * (-t * 24.0).exp() * 0.5)
                    .clamp(-1.0, 1.0)
            })
            .collect();
        // Hat: differentiated (brightened) noise, very short.
        let mut prev = 0.0f32;
        let hat: Vec<f32> = (0..secs(0.09))
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let n = noise();
                let v = (n - prev) * 0.5;
                prev = n;
                v * (-t * 60.0).exp()
            })
            .collect();
        // Bass: band-limited saw at C-2 with a decaying body.
        let bass: Vec<f32> = (0..secs(0.6))
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let saw: f32 = (1..=10).map(|k| (t * 65.41 * k as f32 * TAU).sin() / k as f32).sum();
                saw * 0.45 * (-t * 3.0).exp()
            })
            .collect();
        // Lead: square wave with an integer number of cycles so it can loop (168 samples per cycle).
        let cycle = 168usize;
        let lead: Vec<f32> = (0..cycle * 8)
            .map(|i| {
                let t = (i % cycle) as f32 / cycle as f32;
                (1..=9).step_by(2).map(|k| (t * k as f32 * TAU).sin() / k as f32).sum::<f32>() * 0.4
            })
            .collect();

        let mut song = Song { name: "trakr showcase".into(), bpm: 124.0, ..Song::default() };
        song.instruments =
            vec![mk("Kick", 48, kick), mk("Snare", 48, snare), mk("Hat", 48, hat), mk("Bass", 24, bass)];
        let mut lead_inst = mk("Lead", 48, lead);
        {
            let s = &mut lead_inst.samples[0];
            s.loop_mode = LoopMode::Forward;
            s.loop_end = s.frames();
            s.fine_tune = -7; // 44100/168 Hz is 6 cents above C-4
        }
        lead_inst.volume_env = std::sync::Arc::new(Envelope::adsr(0.005, 0.1, 0.6, 0.15));
        lead_inst.volume = 0.7;
        song.instruments.push(lead_inst);
        for (t, name) in ["Kick", "Snare", "Hats", "Bass", "Lead"].iter().enumerate() {
            song.tracks[t].name = (*name).into();
        }
        song.tracks[4].panning = -0.15;

        // A reverb return and a safety limiter on the master.
        let master = song.tracks.len() - 1;
        let send = song.tracks.len();
        let mut ret = Track::new("Reverb", TrackKind::Send);
        ret.visible_note_columns = 0;
        let mut rv = crate::track::DeviceSpec::new("native.reverb").expect("catalogue device");
        rv.params = vec![0.75, 0.35, 1.0, 1.0];
        ret.devices.push(rv);
        song.tracks.push(ret);
        song.tracks[4].sends.push(crate::track::SendLevel { target: send, level: 0.4, pre_fader: false });
        song.tracks[master]
            .devices
            .push(crate::track::DeviceSpec::new("native.limiter").expect("catalogue device"));

        let ntracks = song.tracks.len();
        let mut groove = Pattern::new(ntracks, 64);
        let set = |p: &mut Pattern, track: usize, line: u16, note: u8, inst: u8, vol: u8| {
            let c = &mut p.line_mut(track, line).notes[0];
            c.note = note;
            c.instrument = inst;
            c.volume = vol;
        };
        for l in 0..64u16 {
            if l % 4 == 0 {
                set(&mut groove, 0, l, 48, 0, EMPTY);
            }
            if l % 8 == 4 {
                set(&mut groove, 1, l, 48, 1, 0x70);
            }
            if l % 2 == 0 {
                set(&mut groove, 2, l, 48, 2, if l % 4 == 2 { 0x60 } else { 0x38 });
            }
        }
        // Bass: four chords (C, Ab, Eb, Bb) of 16 lines, same rhythm each.
        for (bar, root) in [24u8, 20, 27, 22].into_iter().enumerate() {
            for &l in &[0u16, 3, 6, 8, 11, 14] {
                set(&mut groove, 3, bar as u16 * 16 + l, root, 3, 0x70);
            }
        }
        let mut with_lead = groove.clone();
        let melody: [(u16, u8); 21] = [
            (0, 55),
            (4, 51),
            (6, 53),
            (8, 55),
            (12, 58),
            (14, 55),
            (16, 56),
            (20, 53),
            (22, 51),
            (24, 48),
            (28, 51),
            (32, 55),
            (36, 51),
            (38, 53),
            (40, 55),
            (44, 58),
            (46, 60),
            (48, 58),
            (52, 55),
            (56, 51),
            (60, 48),
        ];
        for (i, &(line, note)) in melody.iter().enumerate() {
            set(&mut with_lead, 4, line, note, 4, 0x70);
            let next = melody.get(i + 1).map_or(64, |m| m.0);
            let off = (line + 3).min(next.saturating_sub(1));
            if off > line {
                with_lead.line_mut(4, off).notes[0].note = NOTE_OFF;
            }
        }
        song.patterns = vec![std::sync::Arc::new(groove), std::sync::Arc::new(with_lead)];
        song.sequence = vec![0, 1, 1, 0];
        song
    }
}

#[cfg(test)]
mod structure_tests {
    use super::*;

    #[test]
    fn slot_meta_follows_the_sequence() {
        let mut s = Song::default();
        s.insert_slot(1, 0, SlotMeta { mutes: 0b10, section: Some("Verse".into()), repeat: 0 });
        s.insert_slot(2, 0, SlotMeta::default());
        assert_eq!(s.sequence.len(), 3);
        assert!(s.slot_muted(1, 1) && !s.slot_muted(1, 0) && !s.slot_muted(0, 1) && !s.slot_muted(9, 1));
        s.swap_slots(1, 2);
        assert!(s.slot_muted(2, 1));
        s.remove_slot(2);
        assert_eq!(s.slot_meta.len(), 2);
        assert!(!s.slot_muted(2, 1) && !s.slot_muted(0, 70));
    }

    #[test]
    fn sections_cover_the_slots_up_to_the_next_marker() {
        let mut s = Song::default();
        for i in 0..5 {
            s.insert_slot(i + 1, 0, SlotMeta::default());
        }
        s.slot_meta[1].section = Some("A".into());
        s.slot_meta[4].section = Some("B".into());
        assert_eq!(s.sections(), vec![(1, 3, "A".to_string()), (4, 5, "B".to_string())]);
        assert_eq!(s.section_of(2).unwrap().2, "A");
        assert_eq!(s.section_of(5).unwrap().0, 4);
        assert!(s.section_of(0).is_none());
    }

    #[test]
    fn groove_lengthens_and_shortens_lines_but_keeps_the_total() {
        let mut s = Song::default();
        assert_eq!(s.groove_scale(3), 1.0);
        s.groove = vec![0.0, 0.25, 0.0, 0.25];
        let total: f64 = (0..4).map(|l| s.groove_scale(l)).sum();
        assert!((total - 4.0).abs() < 1e-9);
        assert!(
            s.groove_scale(0) > 1.0 && s.groove_scale(1) < 1.0,
            "line 1 starts late, so line 0 is longer"
        );
        s.groove = vec![f32::NAN; 3];
        assert_eq!(s.groove_scale(1), 1.0, "damaged values are ignored");
    }
}
