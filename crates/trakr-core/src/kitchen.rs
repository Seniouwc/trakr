//! A small song that touches every kind of data the format can hold, so the saved JSON shows the exact
//! spelling of every field and enum variant (`trakr --example-full`). It is also a test fixture: the engine
//! renders it and the format roundtrips it.

use crate::automation::{AutoCurve, AutoTarget, Automation};
use crate::envelope::Envelope;
use crate::instrument::*;
use crate::note::{EffectCmd, NOTE_OFF};
use crate::phrase::{Phrase, PhraseMode};
use crate::song::{SlotMeta, Song};
use crate::track::{DeviceSpec, MuteState, SendLevel, Track, TrackKind};
use std::sync::Arc;

const RATE: u32 = 44_100;

fn tone(freq: f32, secs: f32, saw: bool) -> Vec<f32> {
    let n = (RATE as f32 * secs) as usize;
    (0..n)
        .map(|i| {
            let ph = (i as f32 * freq / RATE as f32).fract();
            if saw {
                (ph * 2.0 - 1.0) * 0.4
            } else {
                (ph * std::f32::consts::TAU).sin() * 0.5
            }
        })
        .collect()
}

fn fx(id: [u8; 2], amount: u8) -> EffectCmd {
    EffectCmd { id, amount }
}

impl Song {
    /// A song that uses every feature the file format stores. Meant for reading and for tests, not for music.
    pub fn kitchen_sink() -> Self {
        let mut song =
            Song { name: "Kitchen sink".into(), author: "trakr".into(), bpm: 125.0, ..Song::default() };
        song.comments = "Every field and enum variant appears somewhere in this file.".into();
        song.edo = 19; // note cells are step numbers; `micro` carries the offset between semitones
        song.groove = vec![0.1, -0.1];

        // ---- tracks: 3 sequencer tracks, a group they feed, a send, and the master -----------------------
        song.tracks.clear();
        for (i, name) in ["Lead", "Bass", "Drums"].into_iter().enumerate() {
            let mut t = Track::new(name, TrackKind::Sequencer);
            t.panning = [-0.3, 0.0, 0.3][i];
            t.color = Some([200, 80 + 50 * i as u8, 90]);
            t.group = Some(3);
            t.sends = vec![SendLevel { target: 4, level: 0.3, pre_fader: i == 0 }];
            song.tracks.push(t);
        }
        song.tracks[1].visible_note_columns = 2;
        song.tracks[1].hide_delay = true;
        song.tracks[2].mute = MuteState::Active;
        song.tracks[2].hide_volume = true;
        song.tracks[2].hide_panning = true;
        song.tracks[2].collapsed = false;
        let mut group = Track::new("Band", TrackKind::Group);
        group.visible_note_columns = 0;
        song.tracks.push(group);
        let mut send = Track::new("Reverb bus", TrackKind::Send);
        send.visible_note_columns = 0;
        send.devices.push(DeviceSpec::new("native.reverb").unwrap());
        song.tracks.push(send);
        let mut master = Track::new("Master", TrackKind::Master);
        master.visible_note_columns = 0;
        master.devices.push(DeviceSpec::new("native.limiter").unwrap());
        song.tracks.push(master);
        // a modulator drives the filter cutoff on the lead track (device 1, parameter 1)
        song.tracks[0].devices.push(DeviceSpec::new("native.lfo").unwrap());
        song.tracks[0].devices.push(DeviceSpec::new("native.filter").unwrap());
        let mut formula = DeviceSpec::new("native.formula").unwrap();
        formula.text = "sin(t * 2)".into();
        song.tracks[1].devices.push(formula);
        song.tracks[1].devices.push(DeviceSpec::new("native.sidechain").unwrap());
        song.tracks[2].devices.push(DeviceSpec::new("native.compressor").unwrap());

        // ---- instruments ---------------------------------------------------------------------------------
        let mut lead = Instrument::new("Lead (all the switches)");
        let mut a = Sample::new("low", RATE, 1, tone(220.0, 0.25, true));
        a.base_note = 45;
        a.fine_tune = 10;
        a.volume = 0.9;
        a.panning = -0.1;
        a.loop_mode = LoopMode::PingPong;
        a.loop_start = 1000;
        a.loop_end = a.data.len();
        a.loop_release = true;
        a.key_range = (0, 59);
        a.vel_range = (0, 127);
        a.slices = vec![2000, 5000];
        let mut b = Sample::new("high", RATE, 1, tone(440.0, 0.25, false));
        b.base_note = 57;
        b.loop_mode = LoopMode::Forward;
        b.loop_start = 0;
        b.loop_end = b.data.len();
        b.key_range = (60, 119);
        b.beat_sync_lines = 0;
        lead.samples = vec![a, b];
        lead.volume = 0.8;
        lead.nna = Nna::NoteOff;
        lead.interpolation = Interpolation::Sinc;
        lead.filter = InstrumentFilter { mode: FilterMode::LowPass, cutoff: 4000.0, resonance: 1.2 };
        *Arc::make_mut(&mut lead.volume_env) = Envelope::adsr(0.01, 0.1, 0.7, 0.2);
        *Arc::make_mut(&mut lead.pan_env) = Envelope {
            enabled: true,
            points: vec![(0.0, 0.5), (0.5, 0.8), (1.0, 0.5)],
            sustain: None,
            loop_points: Some((0, 2)),
            curve: AutoCurve::Smooth,
        };
        *Arc::make_mut(&mut lead.pitch_env) = Envelope {
            enabled: true,
            points: vec![(0.0, 0.55), (0.1, 0.5)],
            sustain: None,
            loop_points: None,
            curve: AutoCurve::EaseOut,
        };
        *Arc::make_mut(&mut lead.cutoff_env) = Envelope {
            enabled: true,
            points: vec![(0.0, 0.4), (0.3, 0.6)],
            sustain: Some(1),
            loop_points: None,
            curve: AutoCurve::EaseIn,
        };
        let step = |target, shape, rate, depth| InstLfo {
            target,
            shape,
            rate,
            depth,
            fade_in: 0.2,
            step_values: [0.0; MAX_STEPS],
            steps: 8,
        };
        lead.lfos = vec![
            step(LfoTarget::Pitch, LfoShape::Sine, 5.0, 0.1),
            step(LfoTarget::Cutoff, LfoShape::Triangle, 0.5, 0.3),
            step(LfoTarget::Pan, LfoShape::SmoothRandom, 2.0, 0.2),
            InstLfo { shape: LfoShape::Stepper, ..step(LfoTarget::Volume, LfoShape::Stepper, 4.0, 0.2) },
        ];
        lead.lfos[3].step_values =
            [1.0, 0.0, 0.5, -0.5, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        lead.key_track = 0.5;
        lead.vel_cutoff = 1.0;
        lead.mods = vec![
            InstMod { source: ModSource::Key, target: LfoTarget::Resonance, depth: 0.2 },
            InstMod { source: ModSource::Velocity, target: LfoTarget::Volume, depth: 0.3 },
            InstMod { source: ModSource::Macro(0), target: LfoTarget::Drive, depth: 0.5 },
        ];
        lead.macros = [0.5, 0.2, 0.8, 0.5];
        lead.fade_out = 0.05;
        lead.drive = 0.1;
        lead.fx = vec![DeviceSpec::new("native.chorus").unwrap()];
        lead.midi_out = Some(MidiOut { channel: 1, program: Some(5) });
        lead.mute_group = 0;
        let mut arp = Phrase::new("Up arp", 8);
        arp.lpb = 8;
        arp.key_range = (60, 71);
        arp.base_note = 60;
        arp.shuffle = 0.2;
        for (i, off) in [0u8, 4, 7, 12, 7, 4, 0, 4].into_iter().enumerate() {
            arp.lines.entry(i as u16).or_default().notes[0].note = 60 + off;
            arp.lines.entry(i as u16).or_default().notes[0].instrument = 0;
        }
        lead.phrases = vec![Arc::new(arp)];
        lead.phrase_mode = PhraseMode::Keymap;

        let mut kick = Instrument::new("Kick");
        kick.samples.push(Sample::new("kick", RATE, 1, {
            (0..8000)
                .map(|i| {
                    ((i as f32 * 0.01 * (1.0 - i as f32 / 9000.0)).sin() * (1.0 - i as f32 / 8000.0)) * 0.9
                })
                .collect()
        }));
        kick.samples[0].one_shot = true;
        kick.mute_group = 1;
        kick.nna = Nna::Continue;
        kick.interpolation = Interpolation::Nearest;
        let mut hat = Instrument::new("Hat");
        hat.samples.push(Sample::new(
            "hat",
            RATE,
            2,
            (0..4000)
                .map(|i| ((i * 7919 % 200) as f32 / 100.0 - 1.0) * 0.2 * (1.0 - i as f32 / 4000.0))
                .collect(),
        ));
        hat.mute_group = 1;
        hat.interpolation = Interpolation::Linear;
        hat.filter = InstrumentFilter { mode: FilterMode::HighPass, cutoff: 6000.0, resonance: 0.707 };
        let mut pad = Instrument::new("Pad (band-pass, notch, program phrases)");
        pad.samples.push(Sample::new("pad", RATE, 1, tone(110.0, 0.5, true)));
        pad.samples[0].loop_mode = LoopMode::Reverse;
        pad.samples[0].loop_end = pad.samples[0].data.len();
        pad.filter = InstrumentFilter { mode: FilterMode::BandPass, cutoff: 900.0, resonance: 2.0 };
        pad.phrase_mode = PhraseMode::Program;
        let mut notch = Instrument::new("Notch");
        notch.samples.push(Sample::new("n", RATE, 1, tone(330.0, 0.2, true)));
        notch.filter = InstrumentFilter { mode: FilterMode::Notch, cutoff: 1000.0, resonance: 3.0 };
        song.instruments = vec![lead, kick, hat, pad, notch];

        // ---- patterns, automation, sequence ------------------------------------------------------------
        let mut p0 = crate::pattern::Pattern::new(song.tracks.len(), 32);
        p0.name = "Verse".into();
        for (line, step) in [(0u16, 57), (4, 60), (8, 63), (12, 66)] {
            let (note, micro) = crate::note::step_to_note(step, 19).unwrap();
            let n = &mut p0.line_mut(0, line).notes[0];
            n.note = note;
            n.micro = micro;
            n.instrument = 0;
            n.volume = 0x60;
            n.panning = 0x50;
            n.delay = if line == 12 { 0x40 } else { 0 };
        }
        p0.line_mut(0, 14).notes[0].note = NOTE_OFF;
        p0.line_mut(0, 6).notes[0].note = 61;
        p0.line_mut(0, 6).notes[0].instrument = 0;
        p0.line_mut(0, 6).notes[0].volume = 0x85; // column command G (glide), digit 4
        p0.line_mut(0, 6).notes[0].effect = fx(*b"0V", 0x34);
        p0.line_mut(0, 16).effects[0] = fx(*b"ZT", 0x90);
        p0.line_mut(0, 24).effects[0] = fx(*b"0L", 0x40);
        for line in (0..32).step_by(8) {
            let n = &mut p0.line_mut(1, line).notes[0];
            n.note = 24 + (line as u8 / 8) * 2;
            n.instrument = 3;
            p0.line_mut(1, line).notes[1].note = 31;
            p0.line_mut(1, line).notes[1].instrument = 4;
        }
        for line in 0..32u16 {
            if line % 4 == 0 {
                p0.line_mut(2, line).notes[0].note = 36;
                p0.line_mut(2, line).notes[0].instrument = 1;
            }
            if line % 2 == 1 {
                let n = &mut p0.line_mut(2, line).notes[0];
                n.note = 60;
                n.instrument = 2;
                n.effect = fx(*b"0Y", 0xA0);
            }
        }
        p0.line_mut(0, 20).notes[0].note = 72; // triggers the Keymap phrase of the lead
        p0.line_mut(0, 20).notes[0].instrument = 0;
        let mut auto = |track: usize, target: AutoTarget, curve: AutoCurve, pts: &[(f32, f32)]| {
            let mut a = Automation::new(track, target);
            a.curve = curve;
            a.points = pts.to_vec();
            p0.automation.push(a);
        };
        auto(0, AutoTarget::TrackVolume, AutoCurve::Linear, &[(0.0, 0.8), (16.0, 1.0)]);
        auto(0, AutoTarget::TrackPan, AutoCurve::Step, &[(0.0, 0.3), (8.0, 0.7)]);
        auto(0, AutoTarget::Device { device: 1, param: 1 }, AutoCurve::Smooth, &[(0.0, 0.3), (31.0, 0.8)]);
        auto(0, AutoTarget::Bpm, AutoCurve::EaseIn, &[(0.0, 0.4), (31.0, 0.45)]);
        auto(0, AutoTarget::Lpb, AutoCurve::EaseOut, &[(0.0, 0.2)]);
        let mut p1 = p0.clone();
        p1.name = "Variation".into();
        p1.automation.clear();
        song.patterns = vec![Arc::new(p0), Arc::new(p1)];
        song.sequence = vec![0, 1, 0];
        song.slot_meta = vec![
            SlotMeta { mutes: 0, section: Some("Intro".into()), repeat: 2 },
            SlotMeta { mutes: 0b100, section: None, repeat: 0 },
            SlotMeta { mutes: 0, section: Some("Outro".into()), repeat: 0 },
        ];
        song
    }
}
