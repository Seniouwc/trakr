# trakr vs Renoise: parity report

Scores are out of 10 and mean "how much of what a Renoise 3.x user relies on in this area works in trakr
today", not code quality. **Caveat:** the reference is my memory of Renoise 3.x behavior, not the official
manual or a side-by-side session, so treat scores as informed estimates. Command ids and `.xrns` element
names are unverified (see `docs/PARITY.md`, which lists every item).

Evidence base: about 10,500 lines of Rust in 10 crates and 94 automated tests (engine rendering, editor
operations, GUI input routing, file formats). The GUI was driven live under a virtual display. **Not tested:**
real speakers or audio devices, recording from a real microphone, a real MIDI keyboard, Windows/macOS, long
sessions, large songs.

## Scorecard

| Area | Score | Where it stands |
|---|---|---|
| Pattern editor and note entry | 5.5 | Notes, instrument, volume, pan, delay, effect columns; hex entry; selection, copy/cut/paste, transpose, reverse, interpolate, fade ramps, insert/delete line; unlimited-ish undo; add/remove tracks and columns; MIDI recording. Missing: phrase editor, find/replace, per-sub-column visibility, randomize/expand/shrink/duplicate, mix and flood paste, line highlight settings, track colors/groups/collapse. |
| Song structure and sequencing | 4.0 | Sequence list (insert, clone, delete, move, pick pattern), pattern length and names, loop range, play song / pattern / from cursor, follow. Missing: pattern matrix, sections, per-slot track mutes, groove/shuffle, block loop, play block, tempo/LPB automation beyond `ZT`/`ZL`. |
| Effect commands | 3.5 | 17 commands (tempo, break, sample offset, backwards, cut, delay, retrigger, pitch slides, glide, vibrato, tremolo, arpeggio, volume slide, fade in/out). Volume and pan columns work on notes and running notes. Missing: special volume-column commands, envelope control, track-level commands (volume, pan, width), auto pan, effects on delayed notes, ghost notes, most of the Z set. |
| Instruments and playback | 5.5 | Multiple samples per instrument with key and velocity zones, NNA modes, base note and fine tune, forward/reverse/ping-pong loops, volume/pan/pitch/cutoff envelopes with a hold point, per-instrument filter, per-note effects. Missing: modulation sets (LFO, stepper, key/velocity tracking), sample FX chains, interpolation modes beyond linear, phrases, instrument bank, plugin instruments. |
| Sample editor | 6.0 | Zoomable waveform, selection, cut/copy/paste/delete/crop, silence, fades, gain, normalize, invert, DC removal, reverse, mono/stereo, tape pitch shift, resample, slicing with split-to-keys, loop editing, WAV export (16/24/32f), import of WAV/AIFF/FLAC/OGG/MP3, live recording. Missing: spectrum view, drawing, loop crossfade, dither, time-stretch, transient detection, beat-sync. |
| Mixer and DSP | 5.0 | Per-track bus, device chain, fader with mute/solo/off, pan, master chain and volume, meters with peak hold. Eighteen native devices (gainer, filter, ladder filter, 3- and 10-band EQ, gate, compressor, limiter, delay, reverb, chorus/flanger, phaser, distortion, bit crusher, ring mod, stereo width, tremolo/auto-pan, exciter) against Renoise's roughly fifty. Groups and sends work (nested groups, pre/post-fader feeds). Missing: sidechain, arbitrary routing, track delay, plugin delay compensation, most devices. |
| Automation and modulation | 4.0 | Per-pattern lanes for track volume/pan and any native device parameter with a graph editor and playhead. Missing: curves and interpolation modes, recording knob moves, envelope copy/paste, range tools, modulation devices (LFO, envelope follower, hydra, formula, macros). |
| Plugin hosting | 0.0 | None: no CLAP, VST3, LV2, LADSPA. |
| MIDI, OSC, scripting | 2.5 | MIDI input for notes, CC7/CC10 and transport. Missing: MIDI output and clock, mapping, OSC, Lua scripting. |
| File formats and project | 4.5 | Native `.trkr` roundtrip, WAV export, song bounce to sample, `.xrns` import of metadata, tracks, patterns, notes, sequence and instruments with sample audio (no envelopes, devices or automation; element names and sample paths are from memory and only tested against synthetic fixtures, so real-world files may import differently). Autosave with crash recovery and a recent-files list work. Missing: `.xrns`/`.xrni`/`.xrnt` export and instrument formats, S3M/XM/IT import (MOD import works), per-track/stem/selection render, FLAC/OGG/MP3 export, backups. |
| Interface and workflow | 5.0 | Native GUI with menus showing shortcuts, context menu, mouse selection, continuous scrolling pattern view, dockable mixer/devices/automation/instrument panels, help window with effect table; plus a terminal UI. Preferences are stored (octave, step, follow, recents). Missing: themes, configurable keymap, window/session restore, command palette, a Renoise-compatible keymap preset, TUI views for everything except the pattern editor, accessibility and scaling options. |
| Engine quality and reliability | 3.5 | Real-time-safe design, sample-accurate delay column, tick timing, smoothing against clicks, automated render tests. Weak spots: linear interpolation only, 128 voices fixed, 64 track limit, voices free large buffers on the audio thread in rare cases, no JACK/PipeWire backend or buffer-size UI, no CPU/latency tools, unverified on real hardware. |

## Overall

**About 4.2 / 10 weighted by what a typical Renoise song uses day to day** (pattern editing, instruments,
sample editing, mixing and effects count most; plugins, scripting and format breadth count less). In plain
terms: trakr can already be used to sketch and finish simple sample-based tracker songs on Linux, and its
sample editing and pattern workflow are the closest to Renoise. It cannot yet replace Renoise for anyone who
depends on plugins, a full effect rack, sends and groups, phrases, or opening existing Renoise projects.

## Highest-value gaps, in order

1. **Verify `.xrns` import against real files** (instruments and samples are now imported but only checked against synthetic fixtures), then add envelopes, devices and automation to it.
2. **Sidechain routing and a `#Send`-style device**, then more native devices (multiband, convolver, bus compressor, modulation devices).
3. **Plugin hosting (CLAP first)**: the largest single missing capability.
4. **Remaining effect commands and special volume/pan column commands**, checked against the manual.
5. **Instrument modulation sets (LFO, stepper, key/velocity tracking), phrases.**
6. **Preferences, configurable keymap (with a Renoise-style preset), themes, autosave and recent files.**
7. **MIDI output/clock/mapping, OSC, Lua scripting.**
8. **Hardware verification pass**: real audio devices, latency, JACK/PipeWire, real MIDI keyboards, long-session soak tests.
