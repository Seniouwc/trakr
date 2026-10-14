//! MIDI input for trakr: event types, a byte parser, and (feature `backend`) a midir implementation.

/// A decoded MIDI message. Channels are 0..=15.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MidiEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    Cc {
        channel: u8,
        controller: u8,
        value: u8,
    },
    /// -1.0 ..= 1.0
    PitchBend {
        channel: u8,
        value: f32,
    },
    /// Realtime start / continue / stop from a sequencer or controller.
    Start,
    Continue,
    Stop,
}

/// Parse one raw MIDI message. Returns `None` for anything trakr does not use (clock, sysex, ...).
/// A note-on with velocity 0 is a note-off, as the MIDI spec says.
pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    let data = |i: usize| bytes.get(i).copied().map(|b| b & 0x7F);
    if status >= 0xF0 {
        return match status {
            0xFA => Some(MidiEvent::Start),
            0xFB => Some(MidiEvent::Continue),
            0xFC => Some(MidiEvent::Stop),
            _ => None,
        };
    }
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 => {
            let (note, velocity) = (data(1)?, data(2)?);
            Some(if velocity == 0 {
                MidiEvent::NoteOff { channel, note }
            } else {
                MidiEvent::NoteOn { channel, note, velocity }
            })
        }
        0x80 => Some(MidiEvent::NoteOff { channel, note: data(1)? }),
        0xB0 => Some(MidiEvent::Cc { channel, controller: data(1)?, value: data(2)? }),
        0xE0 => {
            let v = (data(1)? as i32) | ((data(2)? as i32) << 7);
            Some(MidiEvent::PitchBend { channel, value: (v - 8192) as f32 / 8192.0 })
        }
        _ => None,
    }
}

/// An open MIDI input; poll it from the UI thread.
pub trait MidiInput {
    /// Append every event received since the last call.
    fn poll(&mut self, out: &mut Vec<MidiEvent>);
    fn port_name(&self) -> &str;
}

/// Lists and opens MIDI input ports.
pub trait MidiBackend {
    fn ports(&mut self) -> Vec<String>;
    fn open(&mut self, port: usize) -> anyhow::Result<Box<dyn MidiInput>>;
}

#[cfg(feature = "backend")]
pub use backend::MidirBackend;

#[cfg(feature = "backend")]
mod backend {
    use super::*;
    use anyhow::{anyhow, Context};
    use std::sync::mpsc::{channel, Receiver};

    pub struct MidirBackend;

    struct Connection {
        _conn: midir::MidiInputConnection<()>,
        rx: Receiver<MidiEvent>,
        name: String,
    }

    impl MidiInput for Connection {
        fn poll(&mut self, out: &mut Vec<MidiEvent>) {
            out.extend(self.rx.try_iter());
        }
        fn port_name(&self) -> &str {
            &self.name
        }
    }

    fn client() -> anyhow::Result<midir::MidiInput> {
        midir::MidiInput::new("trakr").context("opening the MIDI system")
    }

    impl MidiBackend for MidirBackend {
        fn ports(&mut self) -> Vec<String> {
            let Ok(c) = client() else { return Vec::new() };
            c.ports().iter().filter_map(|p| c.port_name(p).ok()).collect()
        }

        fn open(&mut self, port: usize) -> anyhow::Result<Box<dyn MidiInput>> {
            let mut c = client()?;
            c.ignore(midir::Ignore::SysexAndTime); // keep clock ticks and sysex out of the queue
            let p = c.ports().get(port).cloned().ok_or_else(|| anyhow!("MIDI port {port} is gone"))?;
            let name = c.port_name(&p).unwrap_or_else(|_| "MIDI input".into());
            let (tx, rx) = channel();
            let conn = c
                .connect(
                    &p,
                    "trakr-in",
                    move |_, bytes, _| {
                        if let Some(e) = parse(bytes) {
                            let _ = tx.send(e);
                        }
                    },
                    (),
                )
                .map_err(|e| anyhow!("cannot connect to {name}: {e}"))?;
            Ok(Box::new(Connection { _conn: conn, rx, name }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notes_cc_bend_and_transport() {
        assert_eq!(parse(&[0x93, 60, 100]), Some(MidiEvent::NoteOn { channel: 3, note: 60, velocity: 100 }));
        assert_eq!(
            parse(&[0x90, 60, 0]),
            Some(MidiEvent::NoteOff { channel: 0, note: 60 }),
            "velocity 0 is note-off"
        );
        assert_eq!(parse(&[0x81, 72, 64]), Some(MidiEvent::NoteOff { channel: 1, note: 72 }));
        assert_eq!(parse(&[0xB0, 7, 127]), Some(MidiEvent::Cc { channel: 0, controller: 7, value: 127 }));
        assert_eq!(parse(&[0xE0, 0, 0x40]), Some(MidiEvent::PitchBend { channel: 0, value: 0.0 }));
        assert!(
            matches!(parse(&[0xE0, 0x7F, 0x7F]), Some(MidiEvent::PitchBend { value, .. }) if value > 0.99)
        );
        assert_eq!(parse(&[0xFA]), Some(MidiEvent::Start));
        assert_eq!(parse(&[0xFB]), Some(MidiEvent::Continue));
        assert_eq!(parse(&[0xFC]), Some(MidiEvent::Stop));
        assert_eq!(parse(&[0xF8]), None, "clock is ignored");
        assert_eq!(parse(&[0x90, 60]), None, "truncated message");
        assert_eq!(parse(&[]), None);
    }
}
