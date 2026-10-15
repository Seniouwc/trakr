#!/usr/bin/env python3
"""A complete worked example: a four-bar minor groove. Run from the repository root:

    cargo build -p trakr
    python3 docs/agents/example_beat.py target/debug/trakr out.trkr out.wav
"""
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from trakr_song import Project, check, synth  # noqa: E402

trakr, out, wav = (sys.argv + [None] * 4)[1:4]
trakr = trakr or "trakr"
out = out or "example.trkr"

p = Project(bpm=122, lpb=4, name="Example groove", author="agent", trakr=trakr)
p.set_tracks(5)  # 0 kick, 1 snare, 2 hat, 3 bass, 4 lead
kick = p.add_instrument("Kick", synth.kick(), one_shot=True)
snare = p.add_instrument("Snare", synth.snare(), one_shot=True)
hat = p.add_instrument("Hat", synth.hat(), one_shot=True)
bass = p.add_instrument("Bass", synth.saw_cycle(65.41), loop=True, base_note="C-2")
lead = p.add_instrument("Lead", synth.square_cycle(261.63, duty=0.35), loop=True, volume=0.5)

for t, name in enumerate(["Kick", "Snare", "Hat", "Bass", "Lead"]):
    p.track(t)["name"] = name
p.track(3)["panning"] = -0.1
p.track(4)["panning"] = 0.3
for t, v in enumerate([0.55, 0.4, 0.25, 0.3, 0.2]):  # keep the sum below 1.0 (peak is reported by --check)
    p.track(t)["volume"] = v
p.add_device(4, "native.delay")  # defaults; see `trakr --devices` for parameters
p.add_device(4, "native.reverb")

# 16-line pattern = one bar at lpb 4. Bars: Am, Am, F, G (bass roots).
roots = ["A-2", "A-2", "F-2", "G-2"]
arps = [["A-4", "C-5", "E-5"], ["A-4", "C-5", "E-5"], ["F-4", "A-4", "C-5"], ["G-4", "B-4", "D-5"]]
seq = []
for bar in range(4):
    pat = p.add_pattern(lines=16, name=f"Bar {bar + 1}")
    seq.append(pat)
    for beat in range(4):
        p.note(pat, 0, beat * 4, "C-4", kick, volume=1.0)
    for line in (4, 12):
        p.note(pat, 1, line, "C-4", snare, volume=0.9)
    for line in range(0, 16, 2):
        p.note(pat, 2, line, "C-4", hat, volume=0.8 if line % 4 == 0 else 0.45, panning=0.2)
    for line in (0, 3, 6, 8, 11, 14):
        p.note(pat, 3, line, roots[bar], bass, volume=0.9)
    for line in range(0, 16, 2):
        p.note(pat, 4, line, arps[bar][(line // 2) % 3], lead, volume=0.7)
    # looping instruments sustain until a note-off: always end them, or the render never finishes
    p.note(pat, 3, 15, "OFF")
    p.note(pat, 4, 15, "OFF")
# a filter sweep on the bass over the last bar: track 3 gets a filter (device 0), cutoff is parameter 1
p.add_device(3, "native.filter", p1=900)
p.automate(seq[3], 3, ("device", 0, 1), [(0, 0.3), (15, 0.8)], curve="Smooth")
p.sequence = seq
p.save(out)
print(check(trakr, out, wav))
