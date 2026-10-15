#!/usr/bin/env python3
"""Smoke tests for trakr_song.py against a built `trakr` binary.

    cargo build -p trakr && python3 docs/agents/test_trakr_song.py            # uses target/debug/trakr
    TRAKR=/path/to/trakr python3 docs/agents/test_trakr_song.py
"""
import os
import sys
import tempfile
import unittest
import wave

sys.path.insert(0, os.path.dirname(__file__))
from trakr_song import (Project, adsr, check, col_cmd, envelope, lfo, load_wav, mod, step_to_note,  # noqa: E402
                        synth)

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
TRAKR = os.environ.get("TRAKR", os.path.join(ROOT, "target", "debug", "trakr"))


def levels(out):
    """(peak, rms) from `--check` output."""
    line = next(l for l in out.splitlines() if l.startswith("render:"))
    parts = line.replace(",", "").split()
    return float(parts[parts.index("peak") + 1]), float(parts[parts.index("rms") + 1])


class Songs(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "t.trkr")

    def tearDown(self):
        self.dir.cleanup()

    def build(self, p):
        p.save(self.path)
        return check(TRAKR, self.path)

    def basic(self, **kw):
        p = Project(trakr=TRAKR, **kw)
        p.set_tracks(3)
        return p

    def test_drum_hits_make_sound(self):
        p = self.basic(bpm=140)
        kick = p.add_instrument("Kick", synth.kick(), one_shot=True)
        pat = p.add_pattern(16)
        p.hits(pat, 0, "x...X...x..x....", "C-4", kick, accent=0.6)
        p.sequence = [pat]
        peak, rms = levels(self.build(p))
        self.assertGreater(peak, 0.2)
        self.assertGreater(rms, 0.01)

    def test_microtonal_51_edo(self):
        p = self.basic(edo=51)
        lead = p.add_instrument("Sine", synth.sine_cycle(261.6256), loop=True, base_note="C-4")
        pat = p.add_pattern(16)
        for i, step in enumerate([102, 105, 110, 115]):  # 51 steps = one octave; 102 = C-2 + an octave
            p.note(pat, 0, i * 4, step=step, instrument=lead)
        p.note(pat, 0, 15, "OFF")
        p.sequence = [pat]
        out = self.build(p)
        self.assertIn("51 EDO", out)
        self.assertEqual(step_to_note(51, 51), (12, 0))
        cell = p.song["patterns"][0]["tracks"][0]["lines"]["0"]["notes"][0]
        self.assertEqual(cell["note"], 24)
        self.assertGreater(levels(out)[0], 0.1)

    def test_instrument_features(self):
        p = self.basic()
        inst = p.add_instrument(
            "Rich", synth.saw_cycle(130.81), loop=True, base_note="C-3",
            volume_env=adsr(0.01, 0.2, 0.6, 0.3), filter={"mode": "LowPass", "cutoff": 2500.0, "resonance": 1.5},
            lfos=[lfo("Pitch", "Sine", 5, 0.1), lfo("Cutoff", "Stepper", 2, 0.4, steps=[1, -1, 0.5])],
            mods=[mod("Velocity", "Volume", 0.3), mod({"Macro": 0}, "Cutoff", 0.5)],
            interpolation="Sinc", drive=0.1, fade_out=0.05,
            pan_env=envelope([(0, 0.5), (1, 0.7)], curve="Smooth"))
        pat = p.add_pattern(16)
        p.note(pat, 0, 0, "C-3", inst, volume=0.8)
        p.note(pat, 0, 8, "OFF")
        p.sequence = [pat]
        self.assertGreater(levels(self.build(p))[0], 0.05)

    def test_multisample_zones_and_phrase(self):
        p = self.basic()
        inst = p.add_instrument("Multi")
        p.add_sample(inst, synth.saw_cycle(130.8), loop=True, base_note="C-3", key_range=(0, 47))
        p.add_sample(inst, synth.square_cycle(523.25), loop=True, base_note="C-5", key_range=(48, 119))
        p.add_phrase(inst, "arp", 4, {0: ("C-4", inst), 1: ("E-4", inst), 2: ("G-4", inst)}, lpb=8,
                     key_range=(96, 107), base_note="C-8")
        pat = p.add_pattern(16)
        p.note(pat, 0, 0, "C-2", inst)
        p.note(pat, 0, 4, "C-4", inst)
        p.note(pat, 1, 8, "C-8", inst)  # triggers the phrase
        for t in (0, 1):
            p.note(pat, t, 15, "OFF")
        p.sequence = [pat]
        self.assertGreater(levels(self.build(p))[0], 0.05)

    def test_routing_devices_automation_sections(self):
        p = Project(trakr=TRAKR)
        p.set_tracks(2)
        group = p.add_bus("Group", "Band")
        send = p.add_bus("Send", "Verb")
        p.route(0, group=group, sends={send: 0.4})
        p.route(1, group=group)
        p.add_device(0, "native.filter", p1=1200)
        p.add_device(send, "native.reverb")
        a = p.add_instrument("Pad", synth.saw_cycle(110), loop=True, base_note="A-2")
        pat = p.add_pattern(32)
        p.note(pat, 0, 0, "A-2", a, volume=0.5)
        p.note(pat, 0, 30, "OFF")
        p.automate(pat, 0, ("device", 0, 1), [(0, 0.2), (31, 0.9)], curve="Smooth")
        p.automate(pat, 0, "TrackPan", [(0, 0.2), (31, 0.8)])
        p.effect(pat, 0, 16, "ZT90")
        p.sequence = [pat, pat]
        p.slot_meta(0, section="Intro", repeat=1)
        p.slot_meta(1, section="Again", mutes=[1])
        p.groove(0.1, -0.1)
        out = self.build(p)
        self.assertGreater(levels(out)[0], 0.05)
        self.assertIn("5 tracks", out)

    def test_wav_loading_and_formats(self):
        wav = os.path.join(self.dir.name, "s.wav")
        with wave.open(wav, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(22050)
            w.writeframes(b"".join(int(8000 * (i % 50 < 25) - 4000).to_bytes(2, "little", signed=True)
                                   for i in range(5000)))
        data, rate, ch = load_wav(wav)
        self.assertEqual((rate, ch, len(data)), (22050, 1, 5000))
        p = self.basic()
        inst = p.add_instrument("Wav", data, rate=rate, channels=ch, one_shot=True)
        pat = p.add_pattern(8)
        p.note(pat, 0, 0, "C-4", inst)
        p.sequence = [pat]
        p.save(self.path)
        for ext in ("wav", "flac", "ogg", "mp3"):
            out = os.path.join(self.dir.name, f"o.{ext}")
            check(TRAKR, self.path, out)
            self.assertGreater(os.path.getsize(out), 500, ext)

    def test_validation_catches_mistakes(self):
        p = self.basic()
        pat = p.add_pattern(8)
        p.note(pat, 0, 0, "C-4", 3)  # no such instrument
        p.sequence = [pat, 9]  # no such pattern
        with self.assertRaises(ValueError) as cm:
            p.validate()
        self.assertIn("instrument 3", str(cm.exception))
        self.assertIn("missing pattern 9", str(cm.exception))
        self.assertEqual(col_cmd("G", 4), 0x85)


if __name__ == "__main__":
    unittest.main()
