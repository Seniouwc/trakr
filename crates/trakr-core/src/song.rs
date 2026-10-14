use crate::instrument::{Instrument, Sample};
use crate::note::{parse_note, EMPTY};
use crate::pattern::{Pattern, DEFAULT_LINES};
use crate::track::{Track, TrackKind};
use serde::{Deserialize, Serialize};

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
    pub patterns: Vec<Pattern>,
    /// Pattern indices in play order.
    pub sequence: Vec<usize>,
    pub instruments: Vec<Instrument>,
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
            patterns: vec![Pattern::new(n, DEFAULT_LINES)],
            sequence: vec![0],
            instruments: Vec::new(),
        }
    }
}

impl Song {
    /// Frames per pattern line at the given rate and tempo.
    pub fn frames_per_line(bpm: f32, lpb: u8, sample_rate: u32) -> f64 {
        sample_rate as f64 * 60.0 / (bpm as f64 * lpb.max(1) as f64)
    }

    pub fn pattern_at(&self, seq_pos: usize) -> Option<&Pattern> {
        self.patterns.get(*self.sequence.get(seq_pos)?)
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

        let pat = &mut song.patterns[0];
        for (i, name) in ["C-4", "E-4", "G-4", "C-5"].iter().cycle().take(16).enumerate() {
            let col = &mut pat.line_mut(0, (i * 4) as u16).notes[0];
            col.note = parse_note(name).unwrap();
            col.instrument = 0;
            col.volume = EMPTY;
        }
        song
    }
}
