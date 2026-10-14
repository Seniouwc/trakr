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

/// A message trakr sends out. Channels are 0..=15.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiOutMsg {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    Program {
        channel: u8,
        program: u8,
    },
    /// All notes off on a channel (CC 123).
    AllNotesOff {
        channel: u8,
    },
    /// Timing clock, 24 per quarter note.
    Clock,
    Start,
    Stop,
}

impl MidiOutMsg {
    /// Raw bytes and how many of them are used.
    pub fn bytes(self) -> ([u8; 3], usize) {
        match self {
            MidiOutMsg::NoteOn { channel, note, velocity } => {
                ([0x90 | (channel & 0x0F), note & 0x7F, velocity & 0x7F], 3)
            }
            MidiOutMsg::NoteOff { channel, note } => ([0x80 | (channel & 0x0F), note & 0x7F, 0], 3),
            MidiOutMsg::Program { channel, program } => ([0xC0 | (channel & 0x0F), program & 0x7F, 0], 2),
            MidiOutMsg::AllNotesOff { channel } => ([0xB0 | (channel & 0x0F), 123, 0], 3),
            MidiOutMsg::Clock => ([0xF8, 0, 0], 1),
            MidiOutMsg::Start => ([0xFA, 0, 0], 1),
            MidiOutMsg::Stop => ([0xFC, 0, 0], 1),
        }
    }
}

/// An open MIDI output.
pub trait MidiSink: Send {
    fn send(&mut self, bytes: &[u8]);
    fn port_name(&self) -> &str;
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

    /// Output ports. Backends without output support list none.
    fn out_ports(&mut self) -> Vec<String> {
        Vec::new()
    }

    fn open_out(&mut self, _port: usize) -> anyhow::Result<Box<dyn MidiSink>> {
        anyhow::bail!("MIDI output is not supported by this backend")
    }
}

pub mod pump {
    //! Delivers timestamped messages from the audio engine to a MIDI port on a helper thread.
    //!
    //! The engine stamps each message with the absolute frame it belongs to. The pump converts that to
    //! wall-clock time relative to when it picked the message up, plus a fixed latency that roughly
    //! matches the audio output delay, so MIDI hardware stays in step with what is heard.

    use super::{MidiOutMsg, MidiSink};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    /// Where the pump reads messages from (implemented by the engine's output queue).
    pub trait Source: Send {
        /// Next message and its absolute frame, if any.
        fn poll(&mut self) -> Option<(u64, MidiOutMsg)>;
        /// Frames the engine has rendered so far (monotonic).
        fn frames_rendered(&self) -> u64;
        /// Lets the owner recover the concrete type after `Pump::stop`.
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
    }

    pub struct Pump {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<Box<dyn Source>>>,
        port: String,
    }

    pub fn spawn(
        mut source: Box<dyn Source>,
        mut sink: Box<dyn MidiSink>,
        sample_rate: u32,
        latency: Duration,
    ) -> Pump {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let port = sink.port_name().to_string();
        let thread = std::thread::Builder::new()
            .name("trakr-midi-out".into())
            .spawn(move || {
                let mut pending: VecDeque<(Instant, MidiOutMsg)> = VecDeque::new();
                let mut sounding = [[false; 128]; 16];
                let rate = sample_rate.max(1) as f64;
                let mut batch: Vec<(u64, MidiOutMsg)> = Vec::new();
                while !flag.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    batch.clear();
                    while let Some(m) = source.poll() {
                        batch.push(m);
                    }
                    if !batch.is_empty() {
                        let rendered = source.frames_rendered();
                        for &(frame, msg) in &batch {
                            let lag = Duration::from_secs_f64(rendered.saturating_sub(frame) as f64 / rate);
                            let due = (now + latency).checked_sub(lag).unwrap_or(now).max(now);
                            // Keep the queue ordered by due time.
                            let at = pending.iter().rposition(|(t, _)| *t <= due).map_or(0, |i| i + 1);
                            pending.insert(at, (due, msg));
                        }
                    }
                    while pending.front().is_some_and(|(t, _)| *t <= Instant::now()) {
                        let (_, msg) = pending.pop_front().unwrap();
                        match msg {
                            MidiOutMsg::NoteOn { channel, note, .. } => {
                                sounding[(channel & 15) as usize][(note & 127) as usize] = true
                            }
                            MidiOutMsg::NoteOff { channel, note } => {
                                sounding[(channel & 15) as usize][(note & 127) as usize] = false
                            }
                            MidiOutMsg::AllNotesOff { channel } => {
                                sounding[(channel & 15) as usize] = [false; 128]
                            }
                            _ => {}
                        }
                        let (b, n) = msg.bytes();
                        sink.send(&b[..n]);
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                // Never leave notes hanging on the receiving instrument.
                for (ch, notes) in sounding.iter().enumerate() {
                    for (note, on) in notes.iter().enumerate() {
                        if *on {
                            let (b, n) = MidiOutMsg::NoteOff { channel: ch as u8, note: note as u8 }.bytes();
                            sink.send(&b[..n]);
                        }
                    }
                }
                source
            })
            .expect("spawning the MIDI output thread");
        Pump { stop, thread: Some(thread), port }
    }

    impl Pump {
        pub fn port_name(&self) -> &str {
            &self.port
        }

        /// Stop the thread (releasing sounding notes) and take the source back for reuse.
        pub fn stop(mut self) -> Option<Box<dyn Source>> {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().and_then(|t| t.join().ok())
        }
    }

    impl Drop for Pump {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }
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

    struct OutConnection {
        conn: midir::MidiOutputConnection,
        name: String,
    }

    impl MidiSink for OutConnection {
        fn send(&mut self, bytes: &[u8]) {
            let _ = self.conn.send(bytes);
        }
        fn port_name(&self) -> &str {
            &self.name
        }
    }

    fn out_client() -> anyhow::Result<midir::MidiOutput> {
        midir::MidiOutput::new("trakr").context("opening the MIDI system")
    }

    fn client() -> anyhow::Result<midir::MidiInput> {
        midir::MidiInput::new("trakr").context("opening the MIDI system")
    }

    impl MidiBackend for MidirBackend {
        fn out_ports(&mut self) -> Vec<String> {
            let Ok(c) = out_client() else { return Vec::new() };
            c.ports().iter().filter_map(|p| c.port_name(p).ok()).collect()
        }

        fn open_out(&mut self, port: usize) -> anyhow::Result<Box<dyn MidiSink>> {
            let c = out_client()?;
            let p = c.ports().get(port).cloned().ok_or_else(|| anyhow!("MIDI port {port} is gone"))?;
            let name = c.port_name(&p).unwrap_or_else(|_| "MIDI output".into());
            let conn = c.connect(&p, "trakr-out").map_err(|e| anyhow!("cannot connect to {name}: {e}"))?;
            Ok(Box::new(OutConnection { conn, name }))
        }

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
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct Script(std::collections::VecDeque<(u64, MidiOutMsg)>, u64);
    impl pump::Source for Script {
        fn poll(&mut self) -> Option<(u64, MidiOutMsg)> {
            self.0.pop_front()
        }
        fn frames_rendered(&self) -> u64 {
            self.1
        }
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    struct Log(Arc<Mutex<Vec<(Instant, Vec<u8>)>>>);
    impl MidiSink for Log {
        fn send(&mut self, bytes: &[u8]) {
            self.0.lock().unwrap().push((Instant::now(), bytes.to_vec()));
        }
        fn port_name(&self) -> &str {
            "log"
        }
    }

    #[test]
    fn message_bytes() {
        assert_eq!(MidiOutMsg::NoteOn { channel: 2, note: 60, velocity: 100 }.bytes(), ([0x92, 60, 100], 3));
        assert_eq!(MidiOutMsg::Program { channel: 0, program: 5 }.bytes().1, 2);
        assert_eq!(MidiOutMsg::Clock.bytes(), ([0xF8, 0, 0], 1));
        assert_eq!(MidiOutMsg::AllNotesOff { channel: 1 }.bytes(), ([0xB1, 123, 0], 3));
    }

    #[test]
    fn pump_keeps_the_spacing_between_messages_and_releases_hanging_notes() {
        let log = Arc::new(Mutex::new(Vec::new()));
        // Two messages 4410 frames (100 ms at 44.1 kHz) apart, both from a buffer that ended at frame 4410.
        let events =
            vec![(0, MidiOutMsg::NoteOn { channel: 0, note: 60, velocity: 90 }), (4410, MidiOutMsg::Clock)];
        let src = Box::new(Script(events.into(), 4410));
        let p = pump::spawn(src, Box::new(Log(log.clone())), 44_100, Duration::from_millis(150));
        std::thread::sleep(Duration::from_millis(400));
        p.stop().expect("source comes back");
        let got = log.lock().unwrap().clone();
        assert_eq!(got.len(), 3, "{got:?}"); // note on, clock, release of the hanging note on stop
        assert_eq!(got[0].1, vec![0x90, 60, 90]);
        assert_eq!(got[1].1, vec![0xF8]);
        assert_eq!(got[2].1, vec![0x80, 60, 0]);
        let gap = got[1].0.duration_since(got[0].0).as_millis();
        assert!((60..160).contains(&gap), "gap {gap} ms");
    }

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
