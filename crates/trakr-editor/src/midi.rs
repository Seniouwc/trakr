//! MIDI keyboard and controller input: play and record notes, mixer knobs, transport.

use super::*;
use trakr_midi::{MidiBackend, MidiEvent};

impl Editor {
    /// Give the editor a MIDI backend (the binary supplies one backed by midir).
    pub fn set_midi_backend(&mut self, backend: Box<dyn MidiBackend>) {
        self.midi_backend = Some(backend);
    }

    pub fn has_midi(&self) -> bool {
        self.midi_backend.is_some()
    }

    /// Names of the available MIDI input ports.
    pub fn midi_ports(&mut self) -> Vec<String> {
        self.midi_backend.as_mut().map(|b| b.ports()).unwrap_or_default()
    }

    pub fn midi_port_name(&self) -> Option<&str> {
        self.midi_in.as_ref().map(|m| m.port_name())
    }

    pub fn midi_connect(&mut self, port: usize) {
        let Some(backend) = &mut self.midi_backend else {
            self.status = "no MIDI system available".into();
            return;
        };
        match backend.open(port) {
            Ok(input) => {
                self.status = format!("MIDI input: {}", input.port_name());
                self.midi_in = Some(input);
            }
            Err(e) => self.status = format!("MIDI: {e:#}"),
        }
    }

    pub fn midi_disconnect(&mut self) {
        if self.midi_in.take().is_some() {
            self.status = "MIDI input closed".into();
        }
    }

    /// Names of the available MIDI output ports.
    pub fn midi_out_ports(&mut self) -> Vec<String> {
        self.midi_backend.as_mut().map(|b| b.out_ports()).unwrap_or_default()
    }

    pub fn midi_out_port_name(&self) -> Option<&str> {
        self.midi_pump.as_ref().map(|p| p.port_name())
    }

    /// Send instruments' MIDI notes (and the clock, if enabled) to an output port.
    pub fn midi_out_connect(&mut self, port: usize) {
        self.midi_out_disconnect();
        let Some(backend) = &mut self.midi_backend else {
            self.status = "no MIDI system available".into();
            return;
        };
        let Some(engine) = &mut self.engine else {
            self.status = "MIDI output needs the audio engine".into();
            return;
        };
        let Some(source) = engine.midi_out.take() else {
            self.status = "MIDI output is busy".into();
            return;
        };
        match backend.open_out(port) {
            Ok(sink) => {
                let latency = std::time::Duration::from_millis(self.prefs.midi_latency_ms.min(1000) as u64);
                let pump = trakr_midi::pump::spawn(Box::new(source), sink, engine.sample_rate, latency);
                self.status = format!("MIDI output: {}", pump.port_name());
                self.midi_pump = Some(pump);
                let clock = self.prefs.midi_clock_out;
                self.send(Command::SetMidiOut { enabled: true, clock });
            }
            Err(e) => {
                engine.midi_out = Some(source);
                self.status = format!("MIDI: {e:#}");
            }
        }
    }

    pub fn midi_out_disconnect(&mut self) {
        let Some(pump) = self.midi_pump.take() else { return };
        self.send(Command::SetMidiOut { enabled: false, clock: false });
        // Give the audio thread a moment to queue the note-offs before the pump goes away.
        std::thread::sleep(std::time::Duration::from_millis(30));
        if let (Some(src), Some(engine)) = (pump.stop(), &mut self.engine) {
            // The queue only ever holds our own source type; put it back for the next connection.
            engine.midi_out = trakr_engine::MidiOutSource::from_boxed(src);
        }
        self.status = "MIDI output closed".into();
    }

    pub fn set_midi_clock_out(&mut self, on: bool) {
        self.prefs.midi_clock_out = on;
        if self.midi_pump.is_some() {
            self.send(Command::SetMidiOut { enabled: true, clock: on });
        }
    }

    /// Pull pending events from the open port and act on them; called from `tick`.
    pub(crate) fn midi_poll(&mut self) {
        let Some(input) = &mut self.midi_in else { return };
        let mut events = std::mem::take(&mut self.midi_buf);
        events.clear();
        input.poll(&mut events);
        for e in events.drain(..) {
            self.handle_midi(e);
        }
        self.midi_buf = events;
    }

    /// Act on one MIDI event.
    ///
    /// - Note on: plays the selected instrument on the cursor track; in edit mode on a note column it
    ///   also writes the note, instrument and velocity (as the volume column) and steps down.
    ///   MIDI note 60 is C-4.
    /// - Note off: releases the cursor track's voices.
    /// - CC 7 and CC 10: cursor track volume and pan. CC 120 and 123: silence the track.
    /// - Start, continue, stop: transport.
    pub fn handle_midi(&mut self, e: MidiEvent) {
        match e {
            MidiEvent::NoteOn { note, velocity, .. } => {
                // other tunings: a MIDI key is one step of the tuning, counted from C-0 at MIDI 12
                let (note, micro) = if self.song.edo == 12 {
                    (note.saturating_sub(12).min(119), 0)
                } else {
                    trakr_core::step_to_note(note as i32 - 12, self.song.edo).unwrap_or((119, 0))
                };
                let (track, instrument) = (self.cursor.track, self.instrument);
                self.send(Command::NoteOn {
                    track,
                    instrument,
                    note,
                    volume: velocity as f32 / 127.0,
                    micro,
                });
                let col = self.cursor.col;
                if self.edit_mode && col < self.cols(track).0 {
                    let vol = ((velocity as u16 * 0x80) / 127) as u8;
                    self.edit(|l| {
                        let n = &mut l.notes[col];
                        n.note = note;
                        n.micro = micro;
                        n.instrument = instrument;
                        n.volume = vol;
                    });
                    self.step_down();
                }
            }
            MidiEvent::NoteOff { .. } => {
                let track = self.cursor.track;
                self.send(Command::NoteOff { track, note: 0 });
            }
            MidiEvent::Clock { micros } => {
                if self.prefs.midi_sync_in {
                    if let Some(bpm) = self.midi_clock.tick(micros) {
                        let bpm = bpm.round().clamp(32.0, 999.0);
                        if (bpm - self.song.bpm).abs() >= 1.0 {
                            self.song_mut().bpm = bpm;
                            self.sync();
                        }
                    }
                }
            }
            MidiEvent::Cc { channel, controller, value } if self.midi_cc(channel, controller, value) => {}
            MidiEvent::Cc { controller: 7, value, .. } => {
                let t = self.cursor.track;
                self.set_track_volume(t, value as f32 / 100.0);
            }
            MidiEvent::Cc { controller: 10, value, .. } => {
                let t = self.cursor.track;
                self.set_track_panning(t, (value as f32 - 64.0) / 63.0);
            }
            MidiEvent::Cc { controller: 120 | 123, .. } => {
                let track = self.cursor.track;
                self.send(Command::NoteOff { track, note: 0 });
            }
            MidiEvent::PitchBend { value, .. } => {
                // The wheel spans +-2 semitones, as on most keyboards.
                self.send(Command::PitchBend(value.clamp(-1.0, 1.0) * 2.0));
            }
            MidiEvent::Cc { .. } => {}
            MidiEvent::Start => self.do_action(Action::PlaySong),
            MidiEvent::Continue => self.do_action(Action::PlayFromCursor),
            MidiEvent::Stop => self.do_action(Action::Stop),
        }
    }
}
