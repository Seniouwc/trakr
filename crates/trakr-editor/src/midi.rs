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
                let note = note.saturating_sub(12).min(119);
                let (track, instrument) = (self.cursor.track, self.instrument);
                self.send(Command::NoteOn { track, instrument, note, volume: velocity as f32 / 127.0 });
                let col = self.cursor.col;
                if self.edit_mode && col < self.cols(track).0 {
                    let vol = ((velocity as u16 * 0x80) / 127) as u8;
                    self.edit(|l| {
                        let n = &mut l.notes[col];
                        n.note = note;
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
            MidiEvent::Cc { .. } | MidiEvent::PitchBend { .. } => {}
            MidiEvent::Start => self.do_action(Action::PlaySong),
            MidiEvent::Continue => self.do_action(Action::PlayFromCursor),
            MidiEvent::Stop => self.do_action(Action::Stop),
        }
    }
}
