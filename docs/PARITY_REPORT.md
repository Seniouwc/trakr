# trakr vs Renoise: parity report

Scores are out of 10 and mean "how much of what a Renoise 3.x user relies on in this area works in trakr
today", not code quality. **Caveat:** the reference is my memory of Renoise 3.x behavior, not the official
manual or a side-by-side session, so treat scores as informed estimates. Command ids and `.xrns` element
names are unverified (see `docs/PARITY.md`, which lists every item).

Evidence base: about 23,000 lines of Rust in 16 crates and 226 automated tests (engine rendering and fuzzing,
editor operations and fuzzing, GUI input routing, file formats, a real CLAP test plugin loaded from disk,
sandboxed Lua). The GUI was driven live under a virtual display. **Not tested:**
real speakers or audio devices, recording from a real microphone, a real MIDI keyboard, Windows/macOS, long
sessions, large songs.

## Scorecard

| Area | Score | Where it stands |
|---|---|---|
| Pattern editor and note entry | 6.3 | Notes, instrument, volume, pan, delay, effect columns; hex entry; selection, copy/cut/paste, transpose, reverse, interpolate, fade ramps, insert/delete line; unlimited-ish undo; add/remove tracks and columns; MIDI recording. Phrase editor (Phrases dock) and Lua scripts for bulk edits. Find/replace, randomize, expand/shrink/repeat block and a command palette. Missing: per-sub-column visibility, randomize/expand/shrink/duplicate, mix and flood paste, line highlight settings, track colors/groups/collapse. |
| Song structure and sequencing | 4.0 | Sequence list (insert, clone, delete, move, pick pattern), pattern length and names, loop range, play song / pattern / from cursor, follow. Missing: pattern matrix, sections, per-slot track mutes, groove/shuffle, block loop, play block, tempo/LPB automation beyond `ZT`/`ZL`. |
| Effect commands | 4.3 | 24 commands (track volume/pan, note volume, auto pan, maybe, stop track, ticks per line; tempo, break, sample offset, backwards, cut, delay, retrigger, pitch slides, glide, vibrato, tremolo, arpeggio, volume slide, fade in/out). Volume and pan columns work on notes and running notes. Missing: special volume-column commands, envelope control, track-level commands (volume, pan, width), auto pan, effects on delayed notes, ghost notes, most of the Z set. |
| Instruments and playback | 6.8 | Multiple samples per instrument with key and velocity zones, NNA modes, base note and fine tune, forward/reverse/ping-pong loops, volume/pan/pitch/cutoff envelopes with a hold point, per-instrument filter, per-note effects. Phrases (own speed, columns, loop region, key-mapped and transposed). Instruments can also send MIDI notes. Per-note LFOs (volume, pan, pitch, cutoff), key tracking and velocity-to-cutoff. Missing: stepper and random modulators, phrase program mode and shuffle, sample FX chains, oversampling and anti-aliased sinc for large pitch-ups, instrument bank, plugin instruments. |
| Sample editor | 6.0 | Zoomable waveform, selection, cut/copy/paste/delete/crop, silence, fades, gain, normalize, invert, DC removal, reverse, mono/stereo, tape pitch shift, resample, slicing with split-to-keys, loop editing, WAV export (16/24/32f), import of WAV/AIFF/FLAC/OGG/MP3, live recording. Missing: spectrum view, drawing, loop crossfade, dither, time-stretch, transient detection, beat-sync. |
| Mixer and DSP | 6.2 | Per-track bus, device chain, fader with mute/solo/off, pan, master chain and volume, meters with peak hold. Twenty-eight native devices (gainer, filter, ladder filter, 3- and 10-band EQ, gate, compressor, limiter, delay, reverb, chorus/flanger, phaser, distortion, bit crusher, ring mod, stereo width, tremolo/auto-pan, exciter, plus LFO and envelope-follower modulators) against Renoise's roughly fifty, and local CLAP effect plugins. Groups and sends work (nested groups, pre/post-fader feeds). Missing: sidechain, arbitrary routing, track delay, plugin delay compensation, most devices. |
| Automation and modulation | 5.6 | Per-pattern lanes for track volume/pan and any native device parameter with a graph editor and playhead. Plugin parameters can be automated too. LFO and envelope-follower devices drive any parameter in the same chain (control rate, one block of delay). Missing: curves and interpolation modes, recording knob moves, envelope copy/paste, range tools, formula, macros, cross-track modulation targets. Hydra and a tempo-synced LFO exist. |
| Plugin hosting | 2.5 | Local CLAP audio effects (mono/stereo): scan, add to a track, sliders, automation, saved with the song, used in offline render; tested against a real shared library. Missing: instruments, plugin windows, plugin state, latency compensation, sidechain, VST2/VST3/LV2/AU, sandboxing. |
| MIDI, OSC, scripting | 6.0 | MIDI input for notes, CC7/CC10 and transport; MIDI output per instrument with program change; MIDI clock and start/stop master with a latency setting; sandboxed Lua 5.4 scripting (song, patterns, cells, selection, undoable runs, console, user script folder). MIDI clock input follows an external tempo; MIDI learn maps controllers to faders, device parameters and commands; OSC server for transport, mixer, devices and notes; the Lua API reaches instruments, tracks, devices and automation. Missing: pitch bend, OSC feedback/client, script callbacks and menu hooks, sample access from scripts. |
| File formats and project | 5.8 | Native `.trkr` roundtrip, WAV export, song bounce to sample, `.xrns` import of metadata, tracks, patterns, notes, sequence and instruments with sample audio (no envelopes, devices or automation; element names and sample paths are from memory and only tested against synthetic fixtures, so real-world files may import differently). Autosave with crash recovery and a recent-files list work. `.xrns` export (metadata, tracks, patterns, sequence, instruments with 24-bit samples; roundtrip-tested through the importer; envelopes, devices, automation, phrases and routing are not exported and are reported). `.s3m` and `.xm` import; backups on save; per-track stem and loop-range WAV export. Missing: `.xrni`/`.xrnt` instrument and preset formats, S3M/XM/IT import (MOD import works), per-track/stem/selection render, FLAC/OGG/MP3 export, backups. |
| Interface and workflow | 6.2 | Native GUI with menus showing shortcuts, context menu, mouse selection, continuous scrolling pattern view, dockable mixer/devices/automation/instrument panels, help window with effect table. Preferences are stored (octave, step, follow, recents). Themes, scale, font size and a rebindable keymap exist. Command palette, remembered window size and bottom panel. Missing: a Renoise-compatible keymap preset, accessibility and scaling options. |
| Engine quality and reliability | 4.2 | Real-time-safe design, sample-accurate delay column, tick timing, smoothing against clicks, automated render tests. Weak spots: sinc interpolation does not low-pass for pitch-ups, 256 voices fixed, 64 track limit, a CPU/voice meter now exists, voices free large buffers on the audio thread in rare cases, no JACK/PipeWire backend or buffer-size UI, no CPU/latency tools, unverified on real hardware. |

## Overall

**About 5.8 / 10 weighted by what a typical Renoise song uses day to day** (pattern editing, instruments,
sample editing, mixing and effects count most; plugins, scripting and format breadth count less). In plain
terms: trakr can already be used to sketch and finish simple sample-based tracker songs on Linux, and its
sample editing and pattern workflow are the closest to Renoise. It cannot yet replace Renoise for anyone who
depends on plugin instruments or VST/LV2 plugins, a full effect rack, or opening existing Renoise projects faithfully.

## Highest-value gaps, in order

1. **Verify `.xrns` import against real files** (instruments and samples are now imported but only checked against synthetic fixtures), then add envelopes, devices and automation to it.
2. **Sidechain routing and a `#Send`-style device**, then more native devices (multiband, convolver, bus compressor, modulation devices).
3. **Plugin instruments and plugin windows (CLAP note ports, GUI embedding), then VST3/LV2**: effects now work, instruments do not.
4. **Remaining effect commands and special volume/pan column commands**, checked against the manual.
5. **Instrument modulation sets (LFO, stepper, key/velocity tracking), phrase program mode and shuffle.**
6. **A Renoise-style keymap preset, window/session restore, command palette.**
7. **MIDI clock slave and mapping, OSC, script callbacks and wider script API (instruments, devices, automation).**
8. **Hardware verification pass**: real audio devices, latency, JACK/PipeWire, real MIDI keyboards, long-session soak tests.
