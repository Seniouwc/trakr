//! Lua scripting for trakr.
//!
//! A script runs against a copy of the song. If it finishes without error the caller swaps the result
//! in as a single undo step; on error nothing changes. Scripts are sandboxed: no files, no OS access,
//! no loading of other code, and limits on instructions, time and memory.
//!
//! Everything is exposed through one global table, `trakr`. Pattern, track, line and column numbers
//! are 0-based, like the numbers shown in the editor (lines in hex there, decimal here). See
//! [`API`] for the reference shown in the app.

use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use trakr_core::note::{note_name, parse_note};
use trakr_core::{
    EffectCmd, Pattern, Song, EMPTY, MAX_EFFECT_COLUMNS, MAX_LINES, MAX_NOTE_COLUMNS, NOTE_EMPTY,
};

/// Scripts shipped with trakr: (menu name, source).
pub const EXAMPLES: &[(&str, &str)] = &[
    ("Euclidean rhythm", include_str!("../scripts/euclid.lua")),
    ("Humanize volumes", include_str!("../scripts/humanize.lua")),
    ("Chord stack", include_str!("../scripts/chord.lua")),
    ("Song report", include_str!("../scripts/report.lua")),
];

/// The scripting reference shown in the app.
pub const API: &str = include_str!("../scripts/API.txt");

const MAX_PATTERNS: usize = 1000;
const MAX_SEQUENCE: usize = 1000;
const MEMORY_LIMIT: usize = 128 * 1024 * 1024;
const MAX_INSTRUCTIONS: u64 = 200_000_000;
const MAX_TIME: Duration = Duration::from_secs(10);
const MAX_OUTPUT_LINES: usize = 2000;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("{0}")]
    Lua(String),
}

impl From<mlua::Error> for ScriptError {
    fn from(e: mlua::Error) -> Self {
        // Callback errors wrap the original message in a traceback; the first lines are the useful part.
        let text = e.to_string();
        let mut lines = text.lines();
        let head = lines.next().unwrap_or("script error");
        ScriptError::Lua(head.to_string())
    }
}

/// Read-only information about the editor, available to scripts as `trakr.ctx`.
#[derive(Clone, Debug, Default)]
pub struct ScriptCtx {
    /// Pattern under the cursor.
    pub pattern: usize,
    pub seq_pos: usize,
    pub track: usize,
    pub line: u16,
    /// Note or effect column within the track.
    pub col: usize,
    pub instrument: u8,
    pub octave: u8,
    /// Selected tracks and lines (inclusive), if there is a selection.
    pub selection: Option<Selection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub track0: usize,
    pub track1: usize,
    pub line0: u16,
    pub line1: u16,
}

/// Things a script asked the app to do after it finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Play,
    Stop,
}

#[derive(Debug)]
pub struct ScriptOutput {
    /// The edited song (only meaningful if `changed`).
    pub song: Song,
    pub changed: bool,
    /// Text the script printed.
    pub output: Vec<String>,
    /// Last `trakr.status(...)` message.
    pub status: Option<String>,
    pub requests: Vec<Request>,
}

struct State {
    song: Song,
    touched: BTreeSet<usize>,
    changed: bool,
    out: Vec<String>,
    status: Option<String>,
    requests: Vec<Request>,
}

type Shared = Rc<RefCell<State>>;

fn err(msg: impl Into<String>) -> mlua::Error {
    mlua::Error::runtime(msg.into())
}

fn pattern_mut(st: &mut State, p: usize) -> mlua::Result<&mut Pattern> {
    if p >= st.song.patterns.len() {
        return Err(err(format!("pattern {p} does not exist (the song has {})", st.song.patterns.len())));
    }
    st.touched.insert(p);
    st.changed = true;
    Ok(Arc::make_mut(&mut st.song.patterns[p]))
}

fn check_cell(st: &State, p: usize, t: usize, l: usize) -> mlua::Result<()> {
    let pat = st.song.patterns.get(p).ok_or_else(|| err(format!("pattern {p} does not exist")))?;
    if t >= st.song.tracks.len() {
        return Err(err(format!("track {t} does not exist (the song has {})", st.song.tracks.len())));
    }
    if l >= pat.number_of_lines as usize {
        return Err(err(format!("line {l} is outside the pattern ({} lines)", pat.number_of_lines)));
    }
    Ok(())
}

fn parse_fx(s: &str) -> mlua::Result<EffectCmd> {
    let b = s.as_bytes();
    if b.len() != 4 || !b[..2].iter().all(u8::is_ascii_alphanumeric) {
        return Err(err(format!("bad effect {s:?}: expected 4 characters like \"0A40\"")));
    }
    let amount = u8::from_str_radix(&s[2..], 16).map_err(|_| err(format!("bad effect amount in {s:?}")))?;
    Ok(EffectCmd { id: [b[0].to_ascii_uppercase(), b[1].to_ascii_uppercase()], amount })
}

fn fx_text(e: &EffectCmd) -> Option<String> {
    (!e.is_empty()).then(|| format!("{}{}{:02X}", e.id[0] as char, e.id[1] as char, e.amount))
}

fn byte(v: &Value, what: &str, max: i64) -> mlua::Result<u8> {
    let n = match v {
        Value::Integer(i) => *i,
        Value::Number(f) if f.is_finite() => f.round() as i64,
        _ => return Err(err(format!("{what} must be a number"))),
    };
    if !(0..=max).contains(&n) {
        return Err(err(format!("{what} {n} is out of range 0..{max}")));
    }
    Ok(n as u8)
}

fn note_value(v: &Value) -> mlua::Result<u8> {
    match v {
        Value::String(s) => {
            let s = s.to_str()?;
            parse_note(&s)
                .ok_or_else(|| err(format!("bad note {:?}: expected e.g. \"C-4\", \"F#3\" or \"OFF\"", &*s)))
        }
        _ => byte(v, "note", 121),
    }
}

fn index(v: i64, what: &str) -> mlua::Result<usize> {
    usize::try_from(v).map_err(|_| err(format!("{what} {v} must not be negative")))
}

/// Run `source` against `song`. Returns the edited song and what the script printed or requested.
pub fn run(song: &Song, ctx: &ScriptCtx, source: &str) -> Result<ScriptOutput, ScriptError> {
    let lua = Lua::new_with(
        StdLib::COROUTINE | StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH,
        LuaOptions::new(),
    )?;
    lua.set_memory_limit(MEMORY_LIMIT)?;
    let state: Shared = Rc::new(RefCell::new(State {
        song: song.clone(),
        touched: BTreeSet::new(),
        changed: false,
        out: Vec::new(),
        status: None,
        requests: Vec::new(),
    }));

    // No way to load other code or reach the machine.
    let globals = lua.globals();
    for name in ["dofile", "loadfile", "load", "loadstring", "require", "collectgarbage"] {
        globals.set(name, Value::Nil)?;
    }

    let started = Instant::now();
    let count = Rc::new(Cell::new(0u64));
    let c = count.clone();
    lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
        c.set(c.get() + 10_000);
        if c.get() > MAX_INSTRUCTIONS || started.elapsed() > MAX_TIME {
            Err(err("script stopped: it ran for too long (infinite loop?)"))
        } else {
            Ok(VmState::Continue)
        }
    })?;

    install_api(&lua, &state, ctx)?;
    lua.load(source).set_name("script").exec()?;

    let mut st = state.borrow_mut();
    let touched = std::mem::take(&mut st.touched);
    for p in touched {
        if let Some(pat) = st.song.patterns.get_mut(p) {
            Arc::make_mut(pat).compact();
        }
    }
    let st = &mut *st;
    Ok(ScriptOutput {
        song: take::take_song(&mut st.song),
        changed: st.changed,
        output: std::mem::take(&mut st.out),
        status: st.status.take(),
        requests: std::mem::take(&mut st.requests),
    })
}

fn install_api(lua: &Lua, state: &Shared, ctx: &ScriptCtx) -> mlua::Result<()> {
    let t = lua.create_table()?;

    // ---- context ----
    let c = lua.create_table()?;
    c.set("pattern", ctx.pattern)?;
    c.set("seq_pos", ctx.seq_pos)?;
    c.set("track", ctx.track)?;
    c.set("line", ctx.line)?;
    c.set("col", ctx.col)?;
    c.set("instrument", ctx.instrument)?;
    c.set("octave", ctx.octave)?;
    if let Some(s) = ctx.selection {
        let sel = lua.create_table()?;
        sel.set("track0", s.track0)?;
        sel.set("track1", s.track1)?;
        sel.set("line0", s.line0)?;
        sel.set("line1", s.line1)?;
        c.set("selection", sel)?;
    }
    t.set("ctx", c)?;

    // ---- output ----
    {
        let st = state.clone();
        lua.globals().set(
            "print",
            lua.create_function(move |lua, args: mlua::Variadic<Value>| {
                let tostring: mlua::Function = lua.globals().get("tostring")?;
                let mut parts = Vec::new();
                for a in args {
                    parts.push(tostring.call::<String>(a)?);
                }
                let mut s = st.borrow_mut();
                if s.out.len() < MAX_OUTPUT_LINES {
                    s.out.push(parts.join("\t"));
                }
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "status",
            lua.create_function(move |_, msg: String| {
                st.borrow_mut().status = Some(msg);
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "play",
            lua.create_function(move |_, ()| {
                st.borrow_mut().requests.push(Request::Play);
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "stop",
            lua.create_function(move |_, ()| {
                st.borrow_mut().requests.push(Request::Stop);
                Ok(())
            })?,
        )?;
    }

    // ---- notes ----
    t.set("note_name", lua.create_function(|_, n: i64| Ok(note_name(n.clamp(0, 255) as u8)))?)?;
    t.set("parse_note", lua.create_function(|_, s: String| Ok(parse_note(&s)))?)?;

    // ---- song ----
    {
        let st = state.clone();
        t.set(
            "song",
            lua.create_function(move |lua, ()| {
                let s = st.borrow();
                let t = lua.create_table()?;
                t.set("name", s.song.name.as_str())?;
                t.set("author", s.song.author.as_str())?;
                t.set("bpm", s.song.bpm)?;
                t.set("lpb", s.song.lpb)?;
                t.set("tpl", s.song.tpl)?;
                t.set("tracks", s.song.tracks.len())?;
                t.set("patterns", s.song.patterns.len())?;
                t.set("instruments", s.song.instruments.len())?;
                let seq = lua.create_table()?;
                for (i, p) in s.song.sequence.iter().enumerate() {
                    seq.set(i + 1, *p)?;
                }
                t.set("sequence", seq)?;
                Ok(t)
            })?,
        )?;
        let st = state.clone();
        t.set(
            "set_song",
            lua.create_function(move |_, fields: Table| {
                let mut s = st.borrow_mut();
                if let Some(v) = fields.get::<Option<String>>("name")? {
                    s.song.name = v;
                }
                if let Some(v) = fields.get::<Option<String>>("author")? {
                    s.song.author = v;
                }
                if let Some(v) = fields.get::<Option<f32>>("bpm")? {
                    if !(32.0..=999.0).contains(&v) {
                        return Err(err(format!("bpm {v} is out of range 32..999")));
                    }
                    s.song.bpm = v;
                }
                if let Some(v) = fields.get::<Option<i64>>("lpb")? {
                    if !(1..=32).contains(&v) {
                        return Err(err(format!("lpb {v} is out of range 1..32")));
                    }
                    s.song.lpb = v as u8;
                }
                if let Some(v) = fields.get::<Option<i64>>("tpl")? {
                    if !(1..=48).contains(&v) {
                        return Err(err(format!("tpl {v} is out of range 1..48")));
                    }
                    s.song.tpl = v as u8;
                }
                s.changed = true;
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "instrument_name",
            lua.create_function(move |_, i: i64| {
                let s = st.borrow();
                Ok(usize::try_from(i).ok().and_then(|i| s.song.instruments.get(i)).map(|i| i.name.clone()))
            })?,
        )?;
        let st = state.clone();
        t.set(
            "track_name",
            lua.create_function(move |_, i: i64| {
                let s = st.borrow();
                Ok(usize::try_from(i).ok().and_then(|i| s.song.tracks.get(i)).map(|t| t.name.clone()))
            })?,
        )?;
    }

    // ---- patterns and sequence ----
    {
        let st = state.clone();
        t.set(
            "pattern_lines",
            lua.create_function(move |_, p: i64| {
                let s = st.borrow();
                Ok(usize::try_from(p).ok().and_then(|p| s.song.patterns.get(p)).map(|p| p.number_of_lines))
            })?,
        )?;
        let st = state.clone();
        t.set(
            "set_pattern_lines",
            lua.create_function(move |_, (p, n): (i64, i64)| {
                if !(1..=MAX_LINES as i64).contains(&n) {
                    return Err(err(format!("a pattern has 1..{MAX_LINES} lines, not {n}")));
                }
                let mut s = st.borrow_mut();
                let pat = pattern_mut(&mut s, index(p, "pattern")?)?;
                pat.number_of_lines = n as u16;
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "add_pattern",
            lua.create_function(move |_, lines: Option<i64>| {
                let n = lines.unwrap_or(64);
                if !(1..=MAX_LINES as i64).contains(&n) {
                    return Err(err(format!("a pattern has 1..{MAX_LINES} lines, not {n}")));
                }
                let mut s = st.borrow_mut();
                if s.song.patterns.len() >= MAX_PATTERNS {
                    return Err(err("too many patterns"));
                }
                let tracks = s.song.tracks.len();
                s.song.patterns.push(Arc::new(Pattern::new(tracks, n as u16)));
                s.changed = true;
                Ok(s.song.patterns.len() - 1)
            })?,
        )?;
        let st = state.clone();
        t.set(
            "append_to_sequence",
            lua.create_function(move |_, p: i64| {
                let mut s = st.borrow_mut();
                let p = index(p, "pattern")?;
                if p >= s.song.patterns.len() {
                    return Err(err(format!("pattern {p} does not exist")));
                }
                if s.song.sequence.len() >= MAX_SEQUENCE {
                    return Err(err("the sequence is full"));
                }
                s.song.sequence.push(p);
                s.changed = true;
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "clear_pattern",
            lua.create_function(move |_, p: i64| {
                let mut s = st.borrow_mut();
                let pat = pattern_mut(&mut s, index(p, "pattern")?)?;
                for t in &mut pat.tracks {
                    t.lines.clear();
                }
                Ok(())
            })?,
        )?;
    }

    // ---- cells ----
    {
        let st = state.clone();
        t.set(
            "get_note",
            lua.create_function(move |lua, (p, t, l, c): (i64, i64, i64, i64)| {
                let s = st.borrow();
                let (p, t, l, c) =
                    (index(p, "pattern")?, index(t, "track")?, index(l, "line")?, index(c, "column")?);
                check_cell(&s, p, t, l)?;
                if c >= MAX_NOTE_COLUMNS {
                    return Err(err(format!("note column {c} is out of range 0..{}", MAX_NOTE_COLUMNS - 1)));
                }
                let n = s.song.patterns[p].line(t, l as u16).map(|x| x.notes[c]).unwrap_or_default();
                let out = lua.create_table()?;
                out.set("note", n.note)?;
                out.set("name", note_name(n.note))?;
                if n.instrument != EMPTY {
                    out.set("instrument", n.instrument)?;
                }
                if n.volume != EMPTY {
                    out.set("volume", n.volume)?;
                }
                if n.panning != EMPTY {
                    out.set("panning", n.panning)?;
                }
                out.set("delay", n.delay)?;
                if let Some(fx) = fx_text(&n.effect) {
                    out.set("fx", fx)?;
                }
                Ok(out)
            })?,
        )?;
        let st = state.clone();
        t.set(
            "set_note",
            lua.create_function(move |_, (p, t, l, c, f): (i64, i64, i64, i64, Table)| {
                let mut s = st.borrow_mut();
                let (p, t, l, c) =
                    (index(p, "pattern")?, index(t, "track")?, index(l, "line")?, index(c, "column")?);
                check_cell(&s, p, t, l)?;
                if c >= MAX_NOTE_COLUMNS {
                    return Err(err(format!("note column {c} is out of range 0..{}", MAX_NOTE_COLUMNS - 1)));
                }
                let mut n = s.song.patterns[p].line(t, l as u16).map(|x| x.notes[c]).unwrap_or_default();
                // A missing field keeps its value; `false` clears it.
                let get = |k: &str| f.get::<Value>(k);
                match get("note")? {
                    Value::Nil => {}
                    Value::Boolean(false) => n.note = NOTE_EMPTY,
                    v => n.note = note_value(&v)?,
                }
                for (key, slot, max) in [
                    ("instrument", &mut n.instrument, 254),
                    ("volume", &mut n.volume, 254),
                    ("panning", &mut n.panning, 254),
                ] {
                    match get(key)? {
                        Value::Nil => {}
                        Value::Boolean(false) => *slot = EMPTY,
                        v => *slot = byte(&v, key, max)?,
                    }
                }
                match get("delay")? {
                    Value::Nil => {}
                    Value::Boolean(false) => n.delay = 0,
                    v => n.delay = byte(&v, "delay", 255)?,
                }
                match get("fx")? {
                    Value::Nil => {}
                    Value::Boolean(false) => n.effect = EffectCmd::default(),
                    Value::String(x) => n.effect = parse_fx(&x.to_str()?)?,
                    _ => return Err(err("fx must be a string like \"0A40\" or false")),
                }
                let pat = pattern_mut(&mut s, p)?;
                pat.line_mut(t, l as u16).notes[c] = n;
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "clear_note",
            lua.create_function(move |_, (p, t, l, c): (i64, i64, i64, i64)| {
                let mut s = st.borrow_mut();
                let (p, t, l, c) =
                    (index(p, "pattern")?, index(t, "track")?, index(l, "line")?, index(c, "column")?);
                check_cell(&s, p, t, l)?;
                if c >= MAX_NOTE_COLUMNS {
                    return Err(err(format!("note column {c} is out of range")));
                }
                let pat = pattern_mut(&mut s, p)?;
                pat.line_mut(t, l as u16).notes[c] = Default::default();
                Ok(())
            })?,
        )?;
        let st = state.clone();
        t.set(
            "get_effect",
            lua.create_function(move |_, (p, t, l, c): (i64, i64, i64, i64)| {
                let s = st.borrow();
                let (p, t, l, c) =
                    (index(p, "pattern")?, index(t, "track")?, index(l, "line")?, index(c, "column")?);
                check_cell(&s, p, t, l)?;
                if c >= MAX_EFFECT_COLUMNS {
                    return Err(err(format!(
                        "effect column {c} is out of range 0..{}",
                        MAX_EFFECT_COLUMNS - 1
                    )));
                }
                Ok(s.song.patterns[p].line(t, l as u16).and_then(|x| fx_text(&x.effects[c])))
            })?,
        )?;
        let st = state.clone();
        t.set(
            "set_effect",
            lua.create_function(move |_, (p, t, l, c, fx): (i64, i64, i64, i64, Value)| {
                let mut s = st.borrow_mut();
                let (p, t, l, c) =
                    (index(p, "pattern")?, index(t, "track")?, index(l, "line")?, index(c, "column")?);
                check_cell(&s, p, t, l)?;
                if c >= MAX_EFFECT_COLUMNS {
                    return Err(err(format!("effect column {c} is out of range")));
                }
                let cmd = match fx {
                    Value::Nil | Value::Boolean(false) => EffectCmd::default(),
                    Value::String(x) => parse_fx(&x.to_str()?)?,
                    _ => return Err(err("effect must be a string like \"0A40\" or false")),
                };
                let pat = pattern_mut(&mut s, p)?;
                pat.line_mut(t, l as u16).effects[c] = cmd;
                Ok(())
            })?,
        )?;
    }
    install_mixer_api(lua, state, &t)?;
    lua.globals().set("trakr", t)?;
    Ok(())
}

mod take {
    use trakr_core::Song;

    /// Move a song out of a shared slot, leaving a cheap placeholder.
    pub fn take_song(s: &mut Song) -> Song {
        std::mem::replace(
            s,
            Song {
                tracks: Vec::new(),
                patterns: Vec::new(),
                sequence: Vec::new(),
                instruments: Vec::new(),
                ..Song::default()
            },
        )
    }
}

fn mute_name(m: trakr_core::MuteState) -> &'static str {
    match m {
        trakr_core::MuteState::Active => "active",
        trakr_core::MuteState::Muted => "muted",
        trakr_core::MuteState::Off => "off",
    }
}

fn track_of(st: &State, t: i64) -> mlua::Result<usize> {
    let t = index(t, "track")?;
    if t >= st.song.tracks.len() {
        return Err(err(format!("track {t} does not exist (the song has {})", st.song.tracks.len())));
    }
    Ok(t)
}

/// Instruments, tracks, devices and automation.
fn install_mixer_api(lua: &Lua, state: &Shared, t: &Table) -> mlua::Result<()> {
    use trakr_core::{AutoTarget, Automation, DeviceSpec, MuteState, Nna};

    let st = state.clone();
    t.set(
        "instrument",
        lua.create_function(move |lua, i: i64| {
            let s = st.borrow();
            let Some(inst) = usize::try_from(i).ok().and_then(|i| s.song.instruments.get(i)) else {
                return Ok(None);
            };
            let out = lua.create_table()?;
            out.set("name", inst.name.as_str())?;
            out.set("volume", inst.volume)?;
            out.set("samples", inst.samples.len())?;
            out.set("phrases", inst.phrases.len())?;
            out.set("lfos", inst.lfos.len())?;
            out.set(
                "nna",
                match inst.nna {
                    Nna::Cut => "cut",
                    Nna::NoteOff => "note-off",
                    Nna::Continue => "continue",
                },
            )?;
            out.set("midi_channel", inst.midi_out.map(|m| m.channel + 1))?;
            Ok(Some(out))
        })?,
    )?;
    let st = state.clone();
    t.set(
        "set_instrument",
        lua.create_function(move |_, (i, f): (i64, Table)| {
            let mut s = st.borrow_mut();
            let i = index(i, "instrument")?;
            if i >= s.song.instruments.len() {
                return Err(err(format!("instrument {i} does not exist")));
            }
            if let Some(v) = f.get::<Option<String>>("name")? {
                s.song.instruments[i].name = v;
            }
            if let Some(v) = f.get::<Option<f32>>("volume")? {
                if !(0.0..=4.0).contains(&v) {
                    return Err(err(format!("volume {v} is out of range 0..4")));
                }
                s.song.instruments[i].volume = v;
            }
            if let Some(v) = f.get::<Option<String>>("nna")? {
                s.song.instruments[i].nna = match v.as_str() {
                    "cut" => Nna::Cut,
                    "note-off" => Nna::NoteOff,
                    "continue" => Nna::Continue,
                    _ => return Err(err("nna must be \"cut\", \"note-off\" or \"continue\"")),
                };
            }
            s.changed = true;
            Ok(())
        })?,
    )?;

    let st = state.clone();
    t.set(
        "track",
        lua.create_function(move |lua, i: i64| {
            let s = st.borrow();
            let Some(tr) = usize::try_from(i).ok().and_then(|i| s.song.tracks.get(i)) else {
                return Ok(None);
            };
            let out = lua.create_table()?;
            out.set("name", tr.name.as_str())?;
            out.set(
                "kind",
                match tr.kind {
                    trakr_core::TrackKind::Sequencer => "track",
                    trakr_core::TrackKind::Group => "group",
                    trakr_core::TrackKind::Send => "send",
                    trakr_core::TrackKind::Master => "master",
                },
            )?;
            out.set("volume", tr.volume)?;
            out.set("panning", tr.panning)?;
            out.set("mute", mute_name(tr.mute))?;
            out.set("solo", tr.solo)?;
            out.set("devices", tr.devices.len())?;
            Ok(Some(out))
        })?,
    )?;
    let st = state.clone();
    t.set(
        "set_track",
        lua.create_function(move |_, (i, f): (i64, Table)| {
            let mut s = st.borrow_mut();
            let i = track_of(&s, i)?;
            if let Some(v) = f.get::<Option<String>>("name")? {
                s.song.tracks[i].name = v;
            }
            if let Some(v) = f.get::<Option<f32>>("volume")? {
                if !(0.0..=4.0).contains(&v) {
                    return Err(err(format!("volume {v} is out of range 0..4 (1 = 0 dB)")));
                }
                s.song.tracks[i].volume = v;
            }
            if let Some(v) = f.get::<Option<f32>>("panning")? {
                if !(-1.0..=1.0).contains(&v) {
                    return Err(err(format!("panning {v} is out of range -1..1")));
                }
                s.song.tracks[i].panning = v;
            }
            if let Some(v) = f.get::<Option<String>>("mute")? {
                s.song.tracks[i].mute = match v.as_str() {
                    "active" => MuteState::Active,
                    "muted" => MuteState::Muted,
                    "off" => MuteState::Off,
                    _ => return Err(err("mute must be \"active\", \"muted\" or \"off\"")),
                };
            }
            if let Some(v) = f.get::<Option<bool>>("solo")? {
                s.song.tracks[i].solo = v;
            }
            s.changed = true;
            Ok(())
        })?,
    )?;

    // ---- devices ----
    let st = state.clone();
    t.set(
        "devices",
        lua.create_function(move |lua, tr: i64| {
            let s = st.borrow();
            let tr = track_of(&s, tr)?;
            let list = lua.create_table()?;
            for (n, d) in s.song.tracks[tr].devices.iter().enumerate() {
                let e = lua.create_table()?;
                e.set("id", d.id.as_str())?;
                e.set("name", d.display_name())?;
                e.set("bypass", d.bypass)?;
                let params = lua.create_table()?;
                for i in 0..d.param_count() {
                    let name = d.param_meta(i).map(|m| m.0).unwrap_or_default();
                    params.set(name, d.param(i))?;
                }
                e.set("params", params)?;
                list.set(n, e)?; // 0-based, like everything else
            }
            Ok(list)
        })?,
    )?;
    let st = state.clone();
    t.set(
        "add_device",
        lua.create_function(move |_, (tr, id): (i64, String)| {
            let mut s = st.borrow_mut();
            let tr = track_of(&s, tr)?;
            let Some(spec) = DeviceSpec::new(&id) else {
                return Err(err(format!(
                    "unknown device {id:?} (see the Devices menu for ids like native.filter)"
                )));
            };
            if s.song.tracks[tr].devices.len() >= 32 {
                return Err(err("a track holds at most 32 devices"));
            }
            s.song.tracks[tr].devices.push(spec);
            s.changed = true;
            Ok(s.song.tracks[tr].devices.len() - 1)
        })?,
    )?;
    let st = state.clone();
    t.set(
        "remove_device",
        lua.create_function(move |_, (tr, d): (i64, i64)| {
            let mut s = st.borrow_mut();
            let tr = track_of(&s, tr)?;
            let d = index(d, "device")?;
            if d >= s.song.tracks[tr].devices.len() {
                return Err(err(format!("device {d} does not exist")));
            }
            s.song.tracks[tr].devices.remove(d);
            s.changed = true;
            Ok(())
        })?,
    )?;
    let st = state.clone();
    t.set(
        "set_device_param",
        lua.create_function(move |_, (tr, d, p, v): (i64, i64, i64, f32)| {
            let mut s = st.borrow_mut();
            let tr = track_of(&s, tr)?;
            let (d, p) = (index(d, "device")?, index(p, "parameter")?);
            let Some(spec) = s.song.tracks[tr].devices.get(d) else {
                return Err(err(format!("device {d} does not exist")));
            };
            let Some((name, min, max, ..)) = spec.param_meta(p) else {
                return Err(err(format!("parameter {p} does not exist")));
            };
            if !v.is_finite() || v < min.min(max) || v > max.max(min) {
                return Err(err(format!("{name} must be between {min} and {max}, not {v}")));
            }
            let defaults: Vec<f32> =
                (0..spec.param_count()).filter_map(|i| spec.param_meta(i).map(|m| m.3)).collect();
            let spec = &mut s.song.tracks[tr].devices[d];
            if spec.params.len() < defaults.len() {
                let have = spec.params.len();
                spec.params.extend_from_slice(&defaults[have..]);
            }
            spec.params[p] = v;
            s.changed = true;
            Ok(())
        })?,
    )?;
    let st = state.clone();
    t.set(
        "set_device_bypass",
        lua.create_function(move |_, (tr, d, b): (i64, i64, bool)| {
            let mut s = st.borrow_mut();
            let tr = track_of(&s, tr)?;
            let d = index(d, "device")?;
            let Some(spec) = s.song.tracks[tr].devices.get_mut(d) else {
                return Err(err(format!("device {d} does not exist")));
            };
            spec.bypass = b;
            s.changed = true;
            Ok(())
        })?,
    )?;

    // ---- automation ----
    let st = state.clone();
    t.set(
        "automate",
        lua.create_function(move |_, (p, tr, target, points): (i64, i64, String, Table)| {
            let mut s = st.borrow_mut();
            let tr = track_of(&s, tr)?;
            let p = index(p, "pattern")?;
            if p >= s.song.patterns.len() {
                return Err(err(format!("pattern {p} does not exist")));
            }
            let target = match target.as_str() {
                "volume" => AutoTarget::TrackVolume,
                "pan" => AutoTarget::TrackPan,
                other => {
                    // "device:D:P" with 0-based device and parameter numbers
                    let parts: Vec<&str> = other.split(':').collect();
                    match parts.as_slice() {
                        ["device", d, pr] => match (d.parse::<usize>(), pr.parse::<usize>()) {
                            (Ok(device), Ok(param))
                                if s.song.tracks[tr]
                                    .devices
                                    .get(device)
                                    .is_some_and(|d| param < d.param_count()) =>
                            {
                                AutoTarget::Device { device, param }
                            }
                            _ => return Err(err(format!("no such device parameter in {other:?}"))),
                        },
                        _ => return Err(err("target must be \"volume\", \"pan\" or \"device:D:P\"")),
                    }
                }
            };
            let lines = s.song.patterns[p].number_of_lines as f32;
            let mut lane = Automation::new(tr, target);
            for pair in points.sequence_values::<Table>() {
                let pair = pair?;
                let (line, value): (f32, f32) = (pair.get(1)?, pair.get(2)?);
                if !line.is_finite()
                    || !value.is_finite()
                    || line < 0.0
                    || line > lines
                    || !(0.0..=1.0).contains(&value)
                {
                    return Err(err(
                        "automation points are { line, value } with line inside the pattern and value 0..1",
                    ));
                }
                lane.set_point(line, value);
            }
            let pat = pattern_mut(&mut s, p)?;
            pat.automation.retain(|a| !(a.track == tr && a.target == target));
            if !lane.points.is_empty() {
                pat.automation.push(lane);
            }
            Ok(())
        })?,
    )?;

    // ---- sequence ----
    let st = state.clone();
    t.set(
        "set_sequence",
        lua.create_function(move |_, list: Table| {
            let mut s = st.borrow_mut();
            let mut seq = Vec::new();
            for v in list.sequence_values::<i64>() {
                let p = index(v?, "pattern")?;
                if p >= s.song.patterns.len() {
                    return Err(err(format!("pattern {p} does not exist")));
                }
                seq.push(p);
            }
            if seq.is_empty() || seq.len() > MAX_SEQUENCE {
                return Err(err(format!("the sequence needs 1..{MAX_SEQUENCE} entries")));
            }
            s.song.sequence = seq;
            s.song.loop_range = None;
            s.changed = true;
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Song {
        let mut s = Song::demo();
        s.name = "T".into();
        s
    }

    fn go(src: &str) -> Result<ScriptOutput, ScriptError> {
        run(&song(), &ScriptCtx::default(), src)
    }

    #[test]
    fn reads_and_writes_cells() {
        let out = go(r#"
            local n = trakr.get_note(0, 0, 0, 0)
            assert(n.name == "C-4" and n.instrument == 0)
            trakr.set_note(0, 1, 5, 0, { note = "E-4", instrument = 0, volume = 64, fx = "0a40" })
            local m = trakr.get_note(0, 1, 5, 0)
            assert(m.note == 52 and m.volume == 64 and m.fx == "0A40", m.name)
            trakr.set_note(0, 1, 5, 0, { volume = false })
            assert(trakr.get_note(0, 1, 5, 0).volume == nil)
            trakr.set_effect(0, 1, 6, 0, "0C03")
            assert(trakr.get_effect(0, 1, 6, 0) == "0C03")
            print("ok", 1, true)
        "#)
        .unwrap();
        assert!(out.changed);
        assert_eq!(out.output, vec!["ok\t1\ttrue".to_string()]);
        let l = out.song.patterns[0].line(1, 5).unwrap();
        assert_eq!((l.notes[0].note, l.notes[0].volume, l.notes[0].instrument), (52, EMPTY, 0));
        assert_eq!(out.song.patterns[0].line(1, 6).unwrap().effects[0].amount, 3);
    }

    #[test]
    fn errors_report_the_problem_and_change_nothing() {
        for (src, needle) in [
            ("trakr.set_note(0, 0, 0, 0, { note = 'H-9' })", "bad note"),
            ("trakr.get_note(5, 0, 0, 0)", "pattern 5"),
            ("trakr.get_note(0, 99, 0, 0)", "track 99"),
            ("trakr.get_note(0, 0, 999, 0)", "line 999"),
            ("trakr.set_effect(0, 0, 0, 0, 'zz')", "bad effect"),
            ("trakr.set_song{ bpm = 5 }", "bpm"),
            ("error('boom')", "boom"),
            ("this is not lua", "script"),
            ("trakr.get_note(-1, 0, 0, 0)", "negative"),
        ] {
            let e = go(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src}: {e}");
        }
    }

    #[test]
    fn song_level_edits_and_patterns() {
        let out = go(r#"
            trakr.set_song{ bpm = 140, lpb = 8, name = "Renamed" }
            local p = trakr.add_pattern(32)
            assert(p == 1 and trakr.pattern_lines(p) == 32)
            trakr.append_to_sequence(p)
            trakr.set_pattern_lines(p, 16)
            local s = trakr.song()
            assert(s.patterns == 2 and #s.sequence == 2 and s.bpm == 140)
        "#)
        .unwrap();
        assert_eq!((out.song.bpm, out.song.lpb, out.song.name.as_str()), (140.0, 8, "Renamed"));
        assert_eq!(out.song.sequence, vec![0, 1]);
        assert_eq!(out.song.patterns[1].number_of_lines, 16);
    }

    #[test]
    fn requests_status_and_context() {
        let ctx = ScriptCtx {
            pattern: 0,
            track: 2,
            line: 7,
            selection: Some(Selection { track0: 1, track1: 3, line0: 4, line1: 9 }),
            ..ScriptCtx::default()
        };
        let out = run(&song(), &ctx, "assert(trakr.ctx.track == 2 and trakr.ctx.selection.line1 == 9); trakr.status('hi'); trakr.play()").unwrap();
        assert_eq!(out.status.as_deref(), Some("hi"));
        assert_eq!(out.requests, vec![Request::Play]);
        assert!(!out.changed, "reading does not count as a change");
    }

    #[test]
    fn sandbox_blocks_files_os_and_code_loading() {
        for src in [
            "io.open('/etc/passwd')",
            "os.execute('true')",
            "dofile('x')",
            "load('return 1')()",
            "require('os')",
            "debug.getinfo(1)",
            "package.loadlib('a','b')",
        ] {
            assert!(go(src).is_err(), "{src} must fail");
        }
    }

    #[test]
    fn runaway_scripts_are_stopped() {
        let t = Instant::now();
        let e = go("while true do end").unwrap_err().to_string();
        assert!(e.contains("too long"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(15));
        let e = go("local t = {} for i = 1, 1e9 do t[i] = ('x'):rep(1000) .. i end").unwrap_err().to_string();
        assert!(e.to_lowercase().contains("memory") || e.contains("too long"), "{e}");
    }

    #[test]
    fn shipped_examples_run() {
        let mut s = Song::demo();
        s.patterns[0] = Arc::new(Pattern::new(s.tracks.len(), 32));
        let ctx = ScriptCtx {
            track: 1,
            selection: Some(Selection { track0: 0, track1: 1, line0: 0, line1: 31 }),
            ..ScriptCtx::default()
        };
        for (name, src) in EXAMPLES {
            let out = run(&s, &ctx, src).unwrap_or_else(|e| panic!("{name}: {e}"));
            if *name == "Euclidean rhythm" {
                let hits = (0..32).filter(|&l| out.song.patterns[0].line(1, l).is_some()).count();
                assert!(hits >= 5 && out.changed, "{hits}");
            }
            if *name == "Song report" {
                assert!(out.output.len() >= 3, "{:?}", out.output);
            }
        }
    }

    #[test]
    fn instruments_tracks_devices_and_automation() {
        let out = go(r#"
            local i = trakr.instrument(0)
            assert(i.name == "Sine C-4" and i.samples == 1 and i.nna == "cut")
            assert(trakr.instrument(99) == nil)
            trakr.set_instrument(0, { name = "Lead", volume = 0.5, nna = "note-off" })
            assert(trakr.instrument(0).name == "Lead" and trakr.instrument(0).nna == "note-off")

            local t = trakr.track(0)
            assert(t.kind == "track" and t.volume == 1 and t.mute == "active")
            assert(trakr.track(8).kind == "master")
            trakr.set_track(0, { name = "Bass", volume = 0.8, panning = -0.5, mute = "muted", solo = true })
            local t2 = trakr.track(0)
            assert(t2.name == "Bass" and t2.panning == -0.5 and t2.mute == "muted" and t2.solo)

            local d = trakr.add_device(0, "native.filter")
            assert(d == 0)
            trakr.set_device_param(0, d, 1, 2000)
            local dev = trakr.devices(0)[0]
            assert(dev.id == "native.filter" and dev.params["Cutoff"] == 2000, dev.params["Cutoff"])
            trakr.set_device_bypass(0, 0, true)
            assert(trakr.devices(0)[0].bypass)
            local e = trakr.add_device(0, "native.gainer")
            trakr.remove_device(0, 0)
            assert(trakr.devices(0)[0].id == "native.gainer" and trakr.track(0).devices == 1)

            trakr.automate(0, 0, "volume", { {0, 0.5}, {8, 1.0} })
            trakr.automate(0, 0, "device:0:0", { {0, 0}, {16, 1} })
            trakr.automate(0, 0, "device:0:0", {})
            local p2 = trakr.add_pattern(16)
            trakr.set_sequence{ 0, p2, 0 }
        "#)
        .unwrap();
        let s = &out.song;
        assert_eq!((s.instruments[0].name.as_str(), s.instruments[0].volume), ("Lead", 0.5));
        assert_eq!(s.tracks[0].name, "Bass");
        assert_eq!(s.tracks[0].devices.len(), 1);
        let lanes = &s.patterns[0].automation;
        assert_eq!(lanes.len(), 1, "device lane removed again: {lanes:?}");
        assert_eq!(s.sequence, vec![0, 1, 0]);
    }

    #[test]
    fn mixer_api_validates_arguments() {
        for (src, needle) in [
            ("trakr.set_track(0, { volume = 9 })", "range"),
            ("trakr.set_track(0, { mute = 'loud' })", "mute"),
            ("trakr.set_track(99, { volume = 1 })", "track 99"),
            ("trakr.add_device(0, 'nope')", "unknown device"),
            ("trakr.add_device(0, 'native.filter'); trakr.set_device_param(0, 0, 1, 99999999)", "between"),
            ("trakr.add_device(0, 'native.filter'); trakr.set_device_param(0, 0, 9, 1)", "parameter 9"),
            ("trakr.remove_device(0, 3)", "device 3"),
            ("trakr.automate(0, 0, 'device:0:0', {})", "no such device"),
            ("trakr.automate(0, 0, 'volume', { {0, 7} })", "0..1"),
            ("trakr.automate(0, 0, 'zzz', {})", "target"),
            ("trakr.set_sequence{ 9 }", "pattern 9"),
            ("trakr.set_sequence{}", "needs"),
            ("trakr.set_instrument(0, { nna = 'x' })", "nna"),
            ("trakr.set_instrument(0, { volume = 99 })", "range"),
        ] {
            let e = go(src).unwrap_err().to_string();
            assert!(e.contains(needle), "{src}: {e}");
        }
    }

    #[test]
    fn touched_patterns_are_compacted() {
        let out = go("trakr.set_note(0, 0, 3, 0, { note = 48 }); trakr.clear_note(0, 0, 3, 0)").unwrap();
        assert!(out.song.patterns[0].line(0, 3).is_none());
    }
}
