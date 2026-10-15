# trakr

An open source tracker DAW written in Rust, with a native window (egui). The goal is a free, Linux-first alternative to Renoise with full feature parity.

**Status: early scaffold (v0.0.1).** The core model, a basic audio engine, a native window UI, a native project format and a minimal XRNS importer work. Most sequencer effects, DSP, automation and plugin hosting are not implemented yet. See the [roadmap](docs/ROADMAP.md), the [parity checklist](docs/PARITY.md) and the scored [parity report](docs/PARITY_REPORT.md).

## Goals

- **Easy to learn:** everything is in a menu with its shortcut, messages say why a key did nothing, and the demo song plays out of the box.
- **Powerful:** keyboard-first workflow and Renoise-class depth over time. Learning ease never removes capability.
- **Raw and direct:** plain tracker conventions; the pattern editor and sequence list are always in front of you.

## Features

- Pattern editor with note, instrument, volume, panning, delay and effect columns
- Undo/redo, block selection, copy/cut/paste, transpose, reverse, interpolate
- Sequence editing (insert, clone, delete, reorder), pattern length, add/remove tracks and columns
- Sample import (WAV, AIFF, FLAC, OGG, MP3), recording from your audio input (Ctrl-R), zoomable waveform editor (cut/copy/paste, fades, gain, normalize, reverse, pitch/resample, slices, key zones), WAV export
- Render the song to a sample or a WAV file; forward/reverse/ping-pong loops
- Delay column and timing effects (sample offset, backwards, cut, delay, retrigger)
- Pitch and volume effects: slides, glide, vibrato, tremolo, arpeggio, volume slide, fade in/out (`0I`/`0O`), plus Block > Fade in/out that ramps the volume column
- Instrument volume, pan, pitch and cutoff envelopes with a hold point, key and velocity zones, NNA modes, linear/cubic/sinc interpolation
- Phrases: per-instrument mini patterns with their own speed, triggered and transposed by a single note
- Group and send tracks with nested routing and pre/post-fader feeds
- A synthesized showcase song opens by default so there is something to play and edit straight away
- MIDI keyboard input (play and record notes, CC volume/pan, transport); MIDI output per instrument and MIDI clock/start/stop master
- Mixer with per-track volume, pan, mute, solo, level meters; song loop range
- 20 track devices (filters, EQs, dynamics, delay, reverb, modulation and distortion effects, plus LFO and envelope-follower modulators that drive any other device's parameter), instrument filter with cutoff envelope, and pattern automation of any device parameter
- Real-time engine with sequencer, voices and mixer (cpal audio output)
- Native `.trkr` project format (zip of `song.json` and raw sample data), autosave with crash recovery, recent files, saved preferences
- `.mod`, `.s3m` and `.xm` module import
- 18 colour themes (Catppuccin x4, Dracula, Gruvbox, Nord, Solarized, Tokyo Night, One Dark, Rose Pine, Everforest, Monokai) plus your own JSON theme files; Preferences (Ctrl-,) for keys, appearance, autosave, backups, OSC
- Command palette (Ctrl-P), find and replace (Ctrl-F), randomize, expand, shrink and repeat block tools
- Instrument LFOs, key tracking, velocity-to-cutoff; 28 native devices including sidechain, bus and multiband compressors
- MIDI learn, MIDI clock input (tempo follow), OSC remote control, per-track stem and loop-range WAV export, CPU meter
- Local CLAP effect plugin hosting (scan, add to a track, edit and automate parameters)
- Lua scripting (sandboxed, one undo step per run): Scripts menu, examples, your own `.lua` files in the config folder, a console window
- Best-effort `.xrns` import and export (patterns and instruments with samples) for interoperability

## Building

Requirements: a recent stable Rust toolchain (1.81+) and ALSA headers on Linux (`libasound2-dev` on Debian/Ubuntu). With Nix, `nix develop` provides everything.

```
cargo run -p trakr                 # native window, built-in demo song
cargo run -p trakr -- song.trkr    # open a native project
cargo run -p trakr -- song.xrns    # import an XRNS file
cargo run -p trakr -- song.mod     # import a ProTracker module
```

## Keys

The keymap is the same in both frontends and can be changed in Edit > Preferences (GUI) or `~/.config/trakr/prefs.json`. The GUI also has full menus (with these shortcuts shown), a context menu on the grid, mouse selection and a Help > Keyboard reference window.

| Key | Action |
|---|---|
| `Space` | Toggle edit mode |
| `z s x d c v g b h n j m` | Piano keys, lower octave |
| `q 2 w 3 e r 5 t 6 y 7 u` | Piano keys, upper octave |
| `` ` `` | Note off |
| Arrows, `Tab` | Move cursor |
| Hex digits | Enter instrument, volume, pan, delay values |
| `F1` / `F2` | Octave down / up |
| `F3` / `F4` | Edit step down / up |
| `F5` | Play song |
| `F6` | Loop pattern |
| `F7` | Play from cursor |
| `F8` | Stop |
| `F9` / `F10` | Previous / next instrument |
| `Ctrl-S` | Save |
| `Ctrl-Q` | Quit |

## Project layout

| Crate | Role |
|---|---|
| `trakr-core` | Data model: song, patterns, tracks, instruments, samples |
| `trakr-engine` | Real-time sequencer, voices, mixer, cpal backend |
| `trakr-format` | Native `.trkr` reader/writer |
| `trakr-xrns` | `.xrns` importer |
| `trakr-editor` | Frontend-neutral editor state, cursor and input handling |
| `trakr-gui` | egui/eframe frontend |
| `trakr` | Binary |

## Development

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test -p trakr-engine --no-default-features   # headless, no ALSA
```

Read [ARCHITECTURE.md](docs/ARCHITECTURE.md) and [CLAUDE.md](CLAUDE.md) (contributor guide and hard rules) before contributing. Key rules: the audio thread must never allocate, lock or block; no Renoise code, assets or branding; new behavior needs tests.

## License

MIT. See [LICENSE](LICENSE).

trakr is an independent project and is not affiliated with Renoise.
