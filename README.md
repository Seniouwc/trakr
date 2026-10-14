# trakr

An open source tracker DAW for the terminal, written in Rust. The goal is a free, Linux-first alternative to Renoise with full feature parity.

**Status: early scaffold (v0.0.1).** The core model, a basic audio engine, a terminal UI, a native project format and a minimal XRNS importer work. Most sequencer effects, DSP, automation and plugin hosting are not implemented yet. See the [roadmap](docs/ROADMAP.md) and [parity checklist](docs/PARITY.md).

## Features

- Pattern editor with note, instrument, volume, panning, delay and effect columns
- Sample playback with linear interpolation and forward loops
- Real-time engine with sequencer, voices and mixer (cpal audio output)
- Native `.trkr` project format (zip of `song.json` and raw sample data)
- Best-effort `.xrns` import for interoperability

## Building

Requirements: a recent stable Rust toolchain (1.80+) and ALSA headers on Linux (`libasound2-dev` on Debian/Ubuntu). With Nix, `nix develop` provides everything.

```
cargo run -p trakr                 # play/edit the built-in demo song
cargo run -p trakr -- song.trkr    # open a native project
cargo run -p trakr -- song.xrns    # import an XRNS file
```

## Keys

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
| `trakr-tui` | ratatui frontend |
| `trakr` | Binary that wires everything together |

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
