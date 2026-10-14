# trakr

Open-source tracker DAW for the terminal, written in Rust, aiming for complete feature parity with Renoise. Linux first.
Native project format `.trkr`; imports `.xrns`.

## Commands

```
cargo run -p trakr                 # demo song; or: cargo run -p trakr -- song.trkr | song.xrns
cargo test --workspace             # must stay green
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test -p trakr-engine --no-default-features   # headless engine tests (no ALSA needed)
```
System dependency: ALSA headers (`libasound2-dev`, or `nix develop` using flake.nix).

## Layout

| crate | role |
|---|---|
| `trakr-core` | Data model only: Song, Pattern (sparse lines), Track, Instrument, Sample, note/effect types. No IO, no audio, no UI. |
| `trakr-engine` | Real-time engine: sequencer, voices, mixer, command queue, cpal backend (feature `audio`). |
| `trakr-format` | Native `.trkr` (zip: `song.json` + raw f32 samples). Versioned via `FORMAT_VERSION`. |
| `trakr-xrns` | Best-effort `.xrns` importer. Element names are from memory, unverified against real files. |
| `trakr-tui` | ratatui frontend. `app.rs` = state + input, `ui.rs` = rendering. |
| `trakr` | Binary: arg parsing, file loading, wires audio + TUI. |

Dependency direction: core <- engine/format/xrns <- tui <- trakr. Never make core depend on anything heavier than serde.

## Hard rules

1. **Audio thread is real-time.** Nothing reachable from `Engine::render` may allocate, lock, block, or do IO. UI to audio goes through the rtrb queues in `EngineHandle`; audio to UI through atomics in `SharedPos`. Frees happen on the UI thread (see the garbage queue).
2. **No Renoise code, assets, samples, or branding.** Implement from user-visible behavior and public documentation. Do not name the product Renoise. XRNS import exists for interoperability only.
3. **Parity work is tracked in `docs/PARITY.md`.** Tick items there when finished; add items you discover.
4. **Tests accompany behavior.** Engine behavior gets a headless render test; format changes get a roundtrip test; every new file format reader needs fixtures.
5. Keep `trakr-core` types serde-stable: adding a field needs `#[serde(default)]`, renaming/removing needs a `FORMAT_VERSION` bump plus a migration.

## Known scaffold shortcuts (fix these early)

- UI to engine sync sends a whole `Arc<Song>` snapshot on every edit (`App::sync`). Replace with incremental edit commands plus an undo/redo command stack (see docs/ARCHITECTURE.md).
- Engine ignores: delay column, volume/panning column commands, most effect commands (only `ZT`, `ZL` work), NNA modes, ticks, envelopes, filters, loop modes other than forward, DSP devices, automation, track mute/solo.
- No sample import yet (only generated demo sample), no song loop range, no pattern clipboard/selection.
- TUI keymap is mine, not Renoise's, apart from the tracker-style piano layout, F1/F2 octave, backtick note-off. Terminals cannot report key release or distinguish left/right modifiers; design around that.

## Conventions

- Rust 2021, `cargo fmt` (max_width 110), clippy clean.
- Errors: `thiserror` in libraries, `anyhow` only in binaries/TUI glue.
- Internal note numbering: 0..=119 = C-0..B-9, 48 = C-4 = MIDI 60, 120 = OFF, 121 = empty. Instrument/volume/pan empty = 0xFF; delay empty = 0.
- Effect ids are two ASCII bytes (`*b"0A"`), `[0,0]` = empty.
- Prefer small, reviewable commits that keep `cargo test --workspace` green.

## Suggested order of work

See docs/ROADMAP.md. Next up: undo/redo + incremental edit commands, selection/clipboard, sample import (symphonia), tick-accurate sequencer with the full effect set.
