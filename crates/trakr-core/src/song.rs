use crate::envelope::Envelope;
use crate::instrument::{Instrument, LoopMode, Sample};
use crate::note::{parse_note, EMPTY, NOTE_OFF};
use crate::pattern::{Pattern, DEFAULT_LINES};
use crate::track::{Track, TrackKind};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
        }
    }
}

impl Song {
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
