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
    def __init__(self, bpm=120.0, lpb=4, name="Untitled", author="", trakr="trakr"):
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
        s.update(name=name, author=author, bpm=float(bpm), lpb=lpb, instruments=[], patterns=[], sequence=[])
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

    def add_instrument(self, name, data, rate=RATE, base_note="C-4", loop=False, one_shot=False,
                       volume=1.0, nna="Cut", channels=1):
        """Add a one-sample instrument. `data` is a list of floats in -1..1 (interleaved if stereo).
        Returns the instrument number to use in the instrument column."""
        inst = copy.deepcopy(self._inst_tpl)
        smp = inst["samples"][0]
        frames = len(data) // channels
        smp.update(name=name, sample_rate=rate, channels=channels, base_note=note_number(base_note),
                   loop_mode="Forward" if loop else "Off", loop_start=0, loop_end=frames,
                   one_shot=one_shot, key_range=[0, 119])
        inst.update(name=name, volume=volume, nna=nna)
        if loop:  # sustained sounds need an amplitude envelope so note-off is not a click
            inst["volume_env"] = {"enabled": True, "points": [[0.0, 0.0], [0.01, 1.0], [0.12, 0.0]],
                                  "sustain": 1, "loop_points": None, "curve": "Linear"}
        self.song["instruments"].append(inst)
        idx = len(self.song["instruments"]) - 1
        self.samples[(idx, 0)] = list(data)
        return idx

    # --- cells -----------------------------------------------------------------------------------
    def _line(self, pattern, track, line):
        lines = self.song["patterns"][pattern]["tracks"][track]["lines"]
        return lines.setdefault(str(line), copy.deepcopy(self._line_tpl))

    def note(self, pattern, track, line, note, instrument=None, volume=None, panning=None, column=0, delay=0, fx=None):
        """Write a note cell. `volume`/`panning` are 0..1 / -1..1 floats; `fx` like '0C02' goes in the cell's
        small effect slot."""
        c = self._line(pattern, track, line)["notes"][column]
        c["note"] = note_number(note)
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

    # --- output ----------------------------------------------------------------------------------
    def save(self, path):
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
