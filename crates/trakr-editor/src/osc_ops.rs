//! Remote control over OSC (UDP). Addresses, with track / device / parameter numbers starting at 1:
//!
//! - `/trakr/play`, `/trakr/stop`, `/trakr/play_pattern`, `/trakr/play_from_cursor`
//! - `/trakr/bpm <f>`
//! - `/trakr/action <name>`: any command by its name, e.g. `PlaySong`, `ToggleEdit`
//! - `/trakr/track/<n>/volume <0..1>`, `/trakr/track/<n>/pan <0..1>`, `/trakr/track/<n>/mute <0|1>`, `/trakr/track/<n>/solo <0|1>`
//! - `/trakr/device/<track>/<device>/<param> <0..1>`: a device parameter by its normalised position
//! - `/trakr/note/on <note> [velocity 0-127] [instrument]` and `/trakr/note/off` on the cursor track
//!
//! Feedback (`osc_feedback_start`) sends to a control surface, only when a value changes:
//! `/trakr/playing <0|1>`, `/trakr/position <slot> <line>`, `/trakr/bpm <f>`,
//! `/trakr/track/<n>/volume|pan <0..1>`, `/trakr/track/<n>/mute|solo <0|1>` and
//! `/trakr/track/<n>/meter <0..1>`.

use super::*;
use std::net::SocketAddr;
use trakr_osc::{Arg, Message};

impl Editor {
    /// Start listening on UDP `port` (this machine only unless `any_interface`). Restarts if already running.
    pub fn osc_start(&mut self, port: u16, any_interface: bool) {
        let ip: std::net::IpAddr = if any_interface { [0, 0, 0, 0].into() } else { [127, 0, 0, 1].into() };
        match trakr_osc::OscServer::bind(SocketAddr::new(ip, port)) {
            Ok(s) => {
                self.status =
                    format!("OSC listening on {}", s.local_addr().map_or("?".into(), |a| a.to_string()));
                self.osc = Some(s);
            }
            Err(e) => {
                self.osc = None;
                self.status = format!("OSC: cannot listen on port {port}: {e}");
            }
        }
    }

    /// Send state changes to `target` (`host:port`) so a control surface can follow the app.
    pub fn osc_feedback_start(&mut self, target: &str) -> bool {
        use std::net::ToSocketAddrs;
        let Some(addr) = target.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
            self.status = format!("OSC feedback: cannot resolve {target:?} (use host:port)");
            return false;
        };
        match trakr_osc::OscClient::new(addr) {
            Ok(c) => {
                self.osc_out = Some((c, std::collections::HashMap::new()));
                self.status = format!("OSC feedback to {addr}");
                true
            }
            Err(e) => {
                self.status = format!("OSC feedback: {e}");
                false
            }
        }
    }

    pub fn osc_feedback_stop(&mut self) {
        if self.osc_out.take().is_some() {
            self.status = "OSC feedback stopped".into();
        }
    }

    pub fn osc_feedback_target(&self) -> Option<SocketAddr> {
        self.osc_out.as_ref().map(|(c, _)| c.target())
    }

    /// Send whatever changed since the last call. Called from `tick`.
    pub(crate) fn osc_feedback_poll(&mut self) {
        let Some((client, last)) = &mut self.osc_out else { return };
        let mut send = |addr: String, args: Vec<Arg>, key: Vec<i64>| {
            if last.get(&addr) != Some(&key) {
                client.send(&addr, &args);
                last.insert(addr, key);
            }
        };
        let (playing, seq, line) = self.engine.as_ref().map_or((false, 0, 0), |e| e.pos.get());
        send("/trakr/playing".into(), vec![Arg::Int(playing as i64)], vec![playing as i64]);
        send(
            "/trakr/position".into(),
            vec![Arg::Int(seq as i64), Arg::Int(line as i64)],
            vec![seq as i64, line as i64],
        );
        let s = &self.song;
        send("/trakr/bpm".into(), vec![Arg::Float(s.bpm as f64)], vec![(s.bpm * 100.0) as i64]);
        for (i, t) in s.tracks.iter().enumerate().take(trakr_engine::MAX_METER_TRACKS) {
            let n = i + 1;
            let pos = (t.panning + 1.0) * 0.5;
            send(
                format!("/trakr/track/{n}/volume"),
                vec![Arg::Float(t.volume as f64)],
                vec![(t.volume * 1000.0) as i64],
            );
            send(format!("/trakr/track/{n}/pan"), vec![Arg::Float(pos as f64)], vec![(pos * 1000.0) as i64]);
            let muted = t.mute != trakr_core::MuteState::Active;
            send(format!("/trakr/track/{n}/mute"), vec![Arg::Int(muted as i64)], vec![muted as i64]);
            send(format!("/trakr/track/{n}/solo"), vec![Arg::Int(t.solo as i64)], vec![t.solo as i64]);
            if let Some(e) = &self.engine {
                let m = e.pos.track_peak(i).clamp(0.0, 1.0);
                // meters move constantly: send in steps of 2%
                send(format!("/trakr/track/{n}/meter"), vec![Arg::Float(m as f64)], vec![(m * 50.0) as i64]);
            }
        }
    }

    pub fn osc_stop(&mut self) {
        if self.osc.take().is_some() {
            self.status = "OSC stopped".into();
        }
    }

    /// Where the OSC server listens, if it runs.
    pub fn osc_addr(&self) -> Option<SocketAddr> {
        self.osc.as_ref().and_then(|s| s.local_addr().ok())
    }

    pub(crate) fn osc_poll(&mut self) {
        let Some(s) = &mut self.osc else { return };
        let mut msgs = std::mem::take(&mut self.osc_buf);
        msgs.clear();
        s.poll(&mut msgs);
        for m in msgs.drain(..) {
            self.handle_osc(&m);
        }
        self.osc_buf = msgs;
    }

    /// Act on one OSC message.
    pub fn handle_osc(&mut self, m: &Message) {
        let parts: Vec<&str> = m.address.trim_matches('/').split('/').collect();
        let f = |i: usize| m.args.get(i).and_then(Arg::as_f32).filter(|v| v.is_finite());
        let n = |i: usize| m.args.get(i).and_then(Arg::as_i64);
        let idx = |s: &str| s.parse::<usize>().ok().filter(|v| *v >= 1).map(|v| v - 1);
        let ok = match parts.as_slice() {
            ["trakr", "play"] => {
                self.do_action(Action::PlaySong);
                true
            }
            ["trakr", "stop"] => {
                self.do_action(Action::Stop);
                true
            }
            ["trakr", "play_pattern"] => {
                self.do_action(Action::PlayPattern);
                true
            }
            ["trakr", "play_from_cursor"] => {
                self.do_action(Action::PlayFromCursor);
                true
            }
            ["trakr", "bpm"] => match f(0) {
                Some(b) => {
                    let (l, t) = (self.song.lpb, self.song.tpl);
                    self.set_tempo(b.clamp(32.0, 999.0), l, t);
                    true
                }
                None => false,
            },
            ["trakr", "action"] => {
                let name = m.args.first().and_then(Arg::as_str).unwrap_or("");
                match Action::ALL.iter().copied().find(|a| format!("{a:?}") == name) {
                    Some(Action::Quit) | Some(Action::New) | Some(Action::Save) => false, // not remotely
                    Some(a) => {
                        self.do_action(a);
                        true
                    }
                    None => false,
                }
            }
            ["trakr", "track", t, what] => match (idx(t).filter(|t| *t < self.song.tracks.len()), what) {
                (Some(t), &"volume") => f(0).map(|v| self.set_track_volume(t, fader_gain(v))).is_some(),
                (Some(t), &"pan") => {
                    f(0).map(|v| self.set_track_panning(t, v.clamp(0.0, 1.0) * 2.0 - 1.0)).is_some()
                }
                (Some(t), &"mute") => {
                    let want = n(0).is_some_and(|v| v != 0);
                    if t < self.song.tracks.len()
                        && (self.song.tracks[t].mute != trakr_core::MuteState::Active) != want
                    {
                        self.track_toggle_mute_at(t);
                    }
                    t < self.song.tracks.len()
                }
                (Some(t), &"solo") => {
                    let want = n(0).is_some_and(|v| v != 0);
                    if t < self.song.tracks.len() && self.song.tracks[t].solo != want {
                        self.track_toggle_solo(t);
                    }
                    t < self.song.tracks.len()
                }
                _ => false,
            },
            ["trakr", "device", t, d, p] => match (idx(t), idx(d), idx(p), f(0)) {
                (Some(t), Some(d), Some(p), Some(v)) => {
                    let value = self
                        .song
                        .tracks
                        .get(t)
                        .and_then(|tr| tr.devices.get(d))
                        .and_then(|dev| dev.denormalize_param(p, v.clamp(0.0, 1.0)));
                    value.map(|v| self.device_set_param(t, d, p, v)).is_some()
                }
                _ => false,
            },
            ["trakr", "note", "on"] => match n(0) {
                Some(note) => {
                    let vel = n(1).unwrap_or(100).clamp(1, 127) as f32 / 127.0;
                    let inst = n(2).map_or(self.instrument, |i| i.clamp(0, 254) as u8);
                    let track = self.cursor.track;
                    self.send(Command::NoteOn {
                        track,
                        instrument: inst,
                        note: note.clamp(0, 119) as u8,
                        volume: vel,
                        micro: 0,
                    });
                    true
                }
                None => false,
            },
            ["trakr", "note", "off"] => {
                let track = self.cursor.track;
                self.send(Command::NoteOff { track, note: 0 });
                true
            }
            _ => false,
        };
        if !ok {
            self.status = format!("OSC: ignored {}", m.address);
        }
    }
}

/// Fader position 0..=1 as a linear gain, the same curve automation lanes use.
fn fader_gain(n: f32) -> f32 {
    if n <= 0.0 {
        0.0
    } else {
        10f32.powf((-60.0 + 66.0 * n.min(1.0)) / 20.0)
    }
}
