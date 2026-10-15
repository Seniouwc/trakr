#!/usr/bin/env python3
"""Tiny helper library for writing trakr `.trkr` projects from Python (standard library only).

It starts from the project `trakr --example` writes, so every field the current build needs is present.
See docs/AGENT_GUIDE.md. Usage:

    from trakr_song import Project, synth
    p = Project(bpm=124, lpb=4)
    kick = p.add_instrument("Kick", synth.kick(), one_shot=True)
    p.set_tracks(4)
    pat = p.add_pattern(lines=64)
    p.note(pat, track=0, line=0, note="C-4", instrument=kick)
    p.sequence = [pat]
    p.save("song.trkr")
"""
import copy
import json
import math
import os
import random
import struct
import subprocess
import tempfile
import wave
import zipfile

NOTE_EMPTY, NOTE_OFF, EMPTY = 121, 120, 255
NAMES = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"]
RATE = 44100


def note_number(n):
    """'C-4' -> 48, 'F#3' -> 42, 'OFF' -> 120, '---' -> 121, ints pass through."""
    if isinstance(n, int):
        return n
    n = n.strip().upper()
    if n == "OFF":
        return NOTE_OFF
    if n in ("---", "", ".."):
        return NOTE_EMPTY
    return int(n[2]) * 12 + NAMES.index(n[:2])


def step_to_note(step, edo):
    """(note, micro) that sounds step `step` (counted from C-0) of an `edo`-step octave, as the engine
    plays it: nearest semitone plus a micro offset in 1/128 semitone. Mirrors trakr_core::step_to_note."""
    cents = step * 1200.0 / edo
    note = round(cents / 100.0)
    if not 0 <= note <= 119:
        raise ValueError(f"step {step} of {edo}-EDO is outside the note range")
    return note, max(-64, min(63, round((cents - note * 100) * 128 / 100)))


def col_cmd(letter, digit=0):
    """Volume/panning column byte for a letter command: G glide, U slide up, D slide down, I fade in,
    O fade out, B backwards, C cut (digit 0..15). Pass it as `volume_byte=` / `panning_byte=`."""
    letters = "GUDIOBC"
    return 0x81 + 16 * letters.index(letter.upper()) + digit


def load_wav(path):
    """Read a PCM WAV (8/16/24/32-bit int) into (floats -1..1 interleaved, rate, channels)."""
    with wave.open(path, "rb") as w:
        n, ch, width, rate = w.getnframes(), w.getnchannels(), w.getsampwidth(), w.getframerate()
        raw = w.readframes(n)
    out = []
    for i in range(n * ch):
        b = raw[i * width:(i + 1) * width]
        if width == 1:
            out.append((b[0] - 128) / 128.0)
        else:
            out.append(int.from_bytes(b, "little", signed=True) / float(1 << (8 * width - 1)))
    return out, rate, ch


def envelope(points, sustain=None, loop=None, curve="Linear"):
    """Instrument envelope. points: [(seconds, value 0..1)]; sustain: index held until note-off;
    loop: (from_index, to_index) repeated while held; curve: Linear|Step|Smooth|EaseIn|EaseOut.
    Volume: value = gain. Pan/pitch: 0.5 = centre / no change (pitch range +-12 semitones). Cutoff: 0.5 = none."""
    return {"enabled": True, "points": [[float(a), float(b)] for a, b in points],
            "sustain": sustain, "loop_points": list(loop) if loop else None, "curve": curve}


def adsr(attack, decay, sustain_level, release):
    return envelope([(0, 0), (attack, 1), (attack + decay, sustain_level), (attack + decay + release, 0)], sustain=2)


def lfo(target="Volume", shape="Sine", rate=4.0, depth=0.5, fade_in=0.0, steps=None):
    """Per-note LFO (max 4 per instrument). target: Volume|Pan|Pitch|Cutoff|Resonance|Drive.
    shape: Sine|Triangle|Saw|Square|Random|SmoothRandom|Stepper (steps: up to 16 levels -1..1)."""
    vals = list(steps or [1.0, -1.0] * 4)
    return {"target": target, "shape": shape, "rate": rate, "depth": depth, "fade_in": fade_in,
            "step_values": (vals + [0.0] * 16)[:16], "steps": max(1, min(16, len(vals)))}


def mod(source, target, depth):
    """Modulation routing (max 8). source: "Key" | "Velocity" | {"Macro": 0..3}; target as for lfo()."""
    return {"source": source, "target": target, "depth": depth}


def vol(x):
    """Volume column byte from a 0..1 level (0x80 = full)."""
    return round(max(0.0, min(1.0, x)) * 128)


def pan(x):
    """Panning column byte from -1 (left) .. 1 (right); 0x40 = centre."""
    return round((max(-1.0, min(1.0, x)) + 1) * 64)


class synth:
    """Simple sample generators. Each returns a list of mono floats in -1..1 at 44100 Hz."""

    @staticmethod
    def kick(length=0.35):
        out, ph = [], 0.0
        for i in range(int(RATE * length)):
            t = i / RATE
            f = 45 + 110 * math.exp(-t * 28)
            ph += 2 * math.pi * f / RATE
            out.append(math.sin(ph) * math.exp(-t * 9) * 0.95)
        return out

    @staticmethod
    def snare(length=0.25, seed=1):
        rng = random.Random(seed)
        return [
            (rng.uniform(-1, 1) * 0.7 + math.sin(2 * math.pi * 190 * i / RATE) * 0.4) * math.exp(-i / RATE * 18) * 0.8
            for i in range(int(RATE * length))
        ]

    @staticmethod
    def hat(length=0.06, seed=2):
        rng = random.Random(seed)
        prev, out = 0.0, []
        for i in range(int(RATE * length)):
            x = rng.uniform(-1, 1)
            out.append((x - prev * 0.9) * math.exp(-i / RATE * 70) * 0.5)  # crude high-pass
            prev = x
        return out

    @staticmethod
    def saw_cycle(freq=130.8128, cycles=64):
        """A looping saw: set loop_mode Forward over the whole sample (see Project.add_instrument loop=True)."""
        n = round(RATE / freq)
        return [((i % n) / n * 2 - 1) * 0.5 for i in range(n * cycles)]

    @staticmethod
    def square_cycle(freq=261.6256, cycles=64, duty=0.5):
        n = round(RATE / freq)
        return [(0.4 if (i % n) / n < duty else -0.4) for i in range(n * cycles)]

    @staticmethod
    def sine_cycle(freq=261.6256, cycles=64):
        n = round(RATE / freq)
        return [math.sin(2 * math.pi * (i % n) / n) * 0.6 for i in range(n * cycles)]


class Project:
    def __init__(self, bpm=120.0, lpb=4, name="Untitled", author="", trakr="trakr", edo=12, tpl=12):
        self.trakr = trakr
        with tempfile.TemporaryDirectory() as d:
            ex = os.path.join(d, "example.trkr")
            subprocess.run([trakr, "--example", ex], check=True, capture_output=True)
            with zipfile.ZipFile(ex) as z:
                env = json.loads(z.read("song.json"))
        self.version = env["format_version"]
        self.song = env["song"]
        s = self.song
        self._track_tpl = next(t for t in s["tracks"] if t["kind"] == "Sequencer")
        self._master = next(t for t in s["tracks"] if t["kind"] == "Master")
        self._inst_tpl = copy.deepcopy(s["instruments"][0])
        self._line_tpl = copy.deepcopy(next(iter(s["patterns"][0]["tracks"][0]["lines"].values())))
        self._line_tpl["notes"] = [
            {"note": NOTE_EMPTY, "instrument": EMPTY, "volume": EMPTY, "panning": EMPTY, "delay": 0,
             "effect": {"id": [0, 0], "amount": 0}}
            for _ in self._line_tpl["notes"]
        ]
        self._line_tpl["effects"] = [{"id": [0, 0], "amount": 0} for _ in self._line_tpl["effects"]]
        self._pattern_tpl = copy.deepcopy(s["patterns"][0])
        s.update(name=name, author=author, bpm=float(bpm), lpb=lpb, tpl=tpl, edo=edo, instruments=[], patterns=[],
                 sequence=[], slot_meta=[], groove=[], loop_range=None, comments="")
        self.samples = {}  # (instrument, sample) -> list of floats
        self.set_tracks(8)

    # --- structure -------------------------------------------------------------------------------
    @property
    def sequence(self):
        return self.song["sequence"]

    @sequence.setter
    def sequence(self, v):
        self.song["sequence"] = list(v)

    def set_tracks(self, n):
        """Use `n` sequencer tracks plus the master (call before adding patterns)."""
        assert not self.song["patterns"], "set the track count before adding patterns"
        tracks = []
        for i in range(n):
            t = copy.deepcopy(self._track_tpl)
            t["name"] = f"Track {i + 1:02}"
            tracks.append(t)
        tracks.append(copy.deepcopy(self._master))
        self.song["tracks"] = tracks

    def track(self, i):
        return self.song["tracks"][i]

    def add_pattern(self, lines=64, name=""):
        p = copy.deepcopy(self._pattern_tpl)
        p.update(name=name, number_of_lines=lines, automation=[])
        p["tracks"] = [{"lines": {}} for _ in self.song["tracks"]]
        self.song["patterns"].append(p)
        return len(self.song["patterns"]) - 1

    def add_instrument(self, name, data=None, rate=RATE, base_note="C-4", loop=False, one_shot=False,
                       volume=1.0, nna="Cut", channels=1, **fields):
        """Add an instrument, with one sample when `data` is given. `data`: floats -1..1 (interleaved if
        stereo). Extra keyword arguments set instrument fields directly, e.g. `filter=`, `volume_env=`
        (see envelope()/adsr()), `lfos=[lfo(...)]`, `mods=[mod(...)]`, `macros=[..4..]`, `interpolation=`
        ("Nearest"|"Linear"|"Cubic"|"Sinc"), `mute_group=`, `key_track=`, `vel_cutoff=`, `fade_out=`, `drive=`,
        `fx=[device dicts]`, `midi_out={"channel":0,"program":None}`. Returns the instrument number."""
        inst = copy.deepcopy(self._inst_tpl)
        inst["samples"] = []
        inst.update(name=name, volume=volume, nna=nna)
        if loop:  # sustained sounds need an amplitude envelope so note-off is not a click
            inst["volume_env"] = adsr(0.01, 0.0, 1.0, 0.1)
        inst.update(fields)
        self.song["instruments"].append(inst)
        idx = len(self.song["instruments"]) - 1
        if data is not None:
            self.add_sample(idx, data, rate=rate, base_note=base_note, loop=loop, one_shot=one_shot,
                            channels=channels, name=name)
        return idx

    def add_sample(self, instrument, data, rate=RATE, base_note="C-4", loop=False, one_shot=False, channels=1,
                   name="sample", key_range=(0, 119), vel_range=(0, 127), loop_mode=None, loop_start=0,
                   loop_end=None, loop_release=False, fine_tune=0, volume=1.0, panning=0.0, slices=(),
                   beat_sync_lines=0):
        """Add a sample to an instrument and return its index. Several samples with different `key_range`
        / `vel_range` make a multisample. `loop_mode`: Off|Forward|Reverse|PingPong (default Forward if
        `loop`). Positions are in frames. `beat_sync_lines` > 0 stretches the sample to that many lines."""
        inst = self.song["instruments"][instrument]
        smp = copy.deepcopy(self._inst_tpl["samples"][0])
        frames = len(data) // channels
        smp.update(name=name, sample_rate=rate, channels=channels, base_note=note_number(base_note),
                   fine_tune=fine_tune, volume=volume, panning=panning,
                   loop_mode=loop_mode or ("Forward" if loop else "Off"), loop_start=loop_start,
                   loop_end=frames if loop_end is None else loop_end, loop_release=loop_release,
                   one_shot=one_shot, key_range=list(key_range), vel_range=list(vel_range),
                   slices=list(slices), beat_sync_lines=beat_sync_lines)
        inst["samples"].append(smp)
        idx = len(inst["samples"]) - 1
        self.samples[(instrument, idx)] = list(data)
        return idx

    def add_phrase(self, instrument, name, lines, rows=None, lpb=4, key_range=(0, 119), base_note="C-4",
                   mode="Keymap", looping=True, shuffle=0.0):
        """Add a phrase to an instrument. `rows`: {line: (note, [instrument])}. Phrases play when the
        instrument is triggered by a note in `key_range` (mode "Keymap"; "Program" picks by note C-0..)
        with the pattern note transposing relative to `base_note`. Returns the phrase number."""
        ph = {"name": name, "number_of_lines": lines, "lpb": lpb, "note_columns": 1, "effect_columns": 1,
              "looping": looping, "loop_start": 0, "loop_end": 0, "base_note": note_number(base_note),
              "key_range": list(key_range), "shuffle": shuffle, "lines": {}}
        for line, cell in (rows or {}).items():
            c = copy.deepcopy(self._line_tpl)
            c["notes"][0]["note"] = note_number(cell[0])
            if len(cell) > 1:
                c["notes"][0]["instrument"] = cell[1]
            ph["lines"][str(line)] = c
        inst = self.song["instruments"][instrument]
        inst.setdefault("phrases", []).append(ph)
        inst["phrase_mode"] = mode
        return len(inst["phrases"]) - 1

    # --- mixer routing -----------------------------------------------------------------------------
    def add_bus(self, kind, name, before_master=True):
        """Add a "Group" or "Send" track (call before add_pattern). Returns its track index."""
        assert not self.song["patterns"], "add buses before patterns"
        t = copy.deepcopy(self._track_tpl)
        t.update(name=name, kind=kind, visible_note_columns=0)
        tracks = self.song["tracks"]
        # order: sequencer tracks, groups, Master, sends
        if kind == "Group":
            pos = next(i for i, x in enumerate(tracks) if x["kind"] in ("Master", "Send"))
        else:
            pos = len(tracks)
        tracks.insert(pos, t)
        for x in tracks:  # keep stored indices valid
            if x["group"] is not None and x["group"] >= pos:
                x["group"] += 1
            for snd in x["sends"]:
                if snd["target"] >= pos:
                    snd["target"] += 1
        return pos

    def route(self, track, group=None, sends=None, pre_fader=False):
        """Send a track's output into a group track and/or feed send tracks: sends={send_index: level}."""
        t = self.track(track)
        t["group"] = group
        t["sends"] = [{"target": k, "level": v, "pre_fader": pre_fader} for k, v in (sends or {}).items()]

    # --- song structure ----------------------------------------------------------------------------
    def slot_meta(self, slot, section=None, repeat=0, mutes=()):
        """Per sequence slot: a named section starting here, how many times that section plays (`repeat`),
        and which tracks are muted while this slot plays."""
        meta = self.song["slot_meta"]
        meta.extend({"mutes": 0, "section": None, "repeat": 0} for _ in range(len(self.sequence) - len(meta)))
        meta[slot] = {"mutes": sum(1 << t for t in mutes), "section": section, "repeat": repeat}

    def groove(self, *offsets):
        """Shuffle: per-line start offsets (fractions of a line, + late, - early) repeating cyclically."""
        self.song["groove"] = [float(o) for o in offsets]

    # --- cells -----------------------------------------------------------------------------------
    def _line(self, pattern, track, line):
        lines = self.song["patterns"][pattern]["tracks"][track]["lines"]
        return lines.setdefault(str(line), copy.deepcopy(self._line_tpl))

    def note(self, pattern, track, line, note=None, instrument=None, volume=None, panning=None, column=0,
             delay=0, fx=None, step=None, micro=0, volume_byte=None, panning_byte=None):
        """Write a note cell. `note`: 'C-4', 48, 'OFF'. For a microtonal song (Project(edo=51)) use
        `step=` (step number counted from C-0) instead, or `micro=` (1/128 semitone) with `note`.
        `volume`/`panning` are 0..1 / -1..1 floats (or raw bytes via `volume_byte=` / `panning_byte=`, e.g.
        col_cmd('G', 4)); `delay` 0..255 = 1/256 line; `fx` like '0C02' goes in the cell's small effect slot."""
        c = self._line(pattern, track, line)["notes"][column]
        if step is not None:
            note, micro = step_to_note(step, self.song["edo"])
        c["note"] = note_number(note)
        c["micro"] = micro
        if volume_byte is not None:
            c["volume"] = volume_byte
        if panning_byte is not None:
            c["panning"] = panning_byte
        if instrument is not None:
            c["instrument"] = instrument
        if volume is not None:
            c["volume"] = vol(volume)
        if panning is not None:
            c["panning"] = pan(panning)
        c["delay"] = delay
        if fx:
            c["effect"] = {"id": [ord(fx[0]), ord(fx[1])], "amount": int(fx[2:4], 16)}

    def effect(self, pattern, track, line, fx, column=0):
        """Write an effect-column command such as 'ZT90' (tempo 0x90) or '0L40'."""
        e = self._line(pattern, track, line)["effects"][column]
        e["id"] = [ord(fx[0]), ord(fx[1])]
        e["amount"] = int(fx[2:4], 16)

    def automate(self, pattern, track, target, points, curve="Linear"):
        """Add an automation lane to a pattern. `target`: "TrackVolume", "TrackPan", "Bpm" (use track 0),
        "Lpb", or ("device", device_index, param_index). `points`: [(line, value 0..1)] ascending; device
        values are normalized over the parameter's range. `curve`: Linear, Step, Smooth, EaseIn, EaseOut."""
        if isinstance(target, tuple):
            target = {"Device": {"device": target[1], "param": target[2]}}
        self.song["patterns"][pattern]["automation"].append(
            {"track": track, "target": target, "points": [[float(a), float(b)] for a, b in points], "curve": curve})

    def add_device(self, track, device_id, **params):
        """Append a native device; `params` maps parameter index -> value (see `trakr --devices`)."""
        info = self._device_defaults(device_id)
        values = list(info)
        for k, v in params.items():
            values[int(k.lstrip("p"))] = float(v)
        self.track(track)["devices"].append(
            {"id": device_id, "bypass": False, "params": values, "plugin": None, "text": ""})

    def _device_defaults(self, device_id):
        out = subprocess.run([self.trakr, "--devices"], check=True, capture_output=True, text=True).stdout
        vals, on = [], False
        for ln in out.splitlines():
            if not ln.startswith(" "):
                on = ln.split()[0] == device_id
            elif on:
                vals.append(float(ln.rsplit("default ", 1)[1].rstrip(")")))
        if not vals:
            raise KeyError(device_id)
        return vals

    def hits(self, pattern, track, pattern_string, note, instrument, volume=1.0, accent=None, column=0):
        """Step-sequencer shorthand: one character per line. 'x' = hit, 'X' = accented hit (volume `accent`,
        default 1.0), '-' = note off, anything else (.) = nothing. Example: hits(p, 0, 'x...x...x...x...', 'C-4', kick)."""
        for line, ch in enumerate(pattern_string):
            if ch == "x":
                self.note(pattern, track, line, note, instrument, volume=volume, column=column)
            elif ch == "X":
                self.note(pattern, track, line, note, instrument, volume=1.0 if accent is None else accent, column=column)
            elif ch == "-":
                self.note(pattern, track, line, "OFF", column=column)

    def validate(self):
        """Raise ValueError listing mistakes that would make a project load badly or play silently."""
        s, errs = self.song, []
        nt = len(s["tracks"])
        kinds = [t["kind"] for t in s["tracks"]]
        if kinds.count("Master") != 1:
            errs.append("there must be exactly one Master track")
        else:
            order = {"Sequencer": 0, "Group": 1, "Master": 2, "Send": 3}
            if [order[k] for k in kinds] != sorted(order[k] for k in kinds):
                errs.append("track order must be: sequencer tracks, group tracks, Master, send tracks")
        for i, p in enumerate(s["patterns"]):
            if len(p["tracks"]) != nt:
                errs.append(f"pattern {i} has {len(p['tracks'])} tracks, song has {nt}")
            if not 1 <= p["number_of_lines"] <= 512:
                errs.append(f"pattern {i}: number_of_lines {p['number_of_lines']} outside 1..512")
            for t, pt in enumerate(p["tracks"]):
                for ln, line in pt["lines"].items():
                    if int(ln) >= p["number_of_lines"]:
                        errs.append(f"pattern {i} track {t}: line {ln} is past the end of the pattern")
                    for c in line["notes"]:
                        if c["note"] < 120 and c["instrument"] != EMPTY and c["instrument"] >= len(s["instruments"]):
                            errs.append(f"pattern {i} track {t} line {ln}: instrument {c['instrument']} does not exist")
        if not s["sequence"]:
            errs.append("the sequence is empty: nothing will play")
        for slot in s["sequence"]:
            if slot >= len(s["patterns"]):
                errs.append(f"sequence refers to missing pattern {slot}")
        for (i, j), d in self.samples.items():
            if i >= len(s["instruments"]) or j >= len(s["instruments"][i]["samples"]):
                errs.append(f"sample data for missing sample {i}/{j}")
            elif not d:
                errs.append(f"sample {i}/{j} is empty")
        for i, inst in enumerate(s["instruments"]):
            for j, smp in enumerate(inst["samples"]):
                if (i, j) not in self.samples:
                    errs.append(f"sample {i}/{j} has no data (add it with add_sample)")
        if errs:
            raise ValueError("; ".join(errs))

    # --- output ----------------------------------------------------------------------------------
    def save(self, path, validate=True):
        if validate:
            self.validate()
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
            z.writestr("song.json", json.dumps({"format_version": self.version, "song": self.song}, indent=1))
            for (i, s), data in self.samples.items():
                z.writestr(f"samples/{i}_{s}.f32", struct.pack(f"<{len(data)}f", *data))
        return path


def check(trakr, path, wav=None):
    """Run `trakr --check` (or `--render wav`) and return its text output; raises on a bad project."""
    cmd = [trakr, "--render", wav, path] if wav else [trakr, "--check", path]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr.strip() or r.stdout.strip())
    return r.stdout
