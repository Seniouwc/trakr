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

## Product goals

1. **Easy to learn.** A new user should be productive in minutes: every command is in a menu with its shortcut shown, status messages explain why something did nothing (e.g. "edit mode is off"), defaults are sensible, and the demo song works out of the box. Prefer one obvious way to do a thing.
2. **Powerful, no ceiling.** Keyboard-first speed and full Renoise-class depth (see docs/PARITY.md). Ease of learning must never remove capability; hide depth behind discoverable menus and shortcuts instead.
3. **Raw and direct.** Plain tracker conventions, no wizards, no modal hand-holding. The pattern editor is the center of the app, with the scrolling playhead view and the sequence list always visible.

New commands go into `trakr-editor::Action` (label + shortcut) so menus, keys and help stay in sync.

## Layout

| crate | role |
|---|---|
| `trakr-core` | Data model only: Song, Pattern (sparse lines), Track, Instrument, Sample, note/effect types. No IO, no audio, no UI. |
| `trakr-engine` | Real-time engine: sequencer, voices, mixer, command queue, cpal backend (feature `audio`). |
| `trakr-format` | Native `.trkr` (zip: `song.json` + raw f32 samples). Versioned via `FORMAT_VERSION`. |
| `trakr-sample` | Audio file decoding (symphonia) into `Sample`. |
| `trakr-xrns` | Best-effort `.xrns` importer. Element names are from memory, unverified against real files. |
| `trakr-editor` | Frontend-neutral `Editor`: cursor, cell entry, transport keys, save. Own `KeyEvent` type; no UI deps. All editing behavior lives here. |
| `trakr-tui` | ratatui frontend: key translation + rendering only. |
| `trakr-gui` | egui/eframe frontend (default): key translation + painter-based grid. |
| `trakr` | Binary: arg parsing (`--tui` selects the terminal UI, GUI otherwise), file loading, wires audio + frontend. |

Dependency direction: core <- engine/format/xrns <- editor <- tui/gui <- trakr. Never make core depend on anything heavier than serde.

## Hard rules

1. **Audio thread is real-time.** Nothing reachable from `Engine::render` may allocate, lock, block, or do IO. UI to audio goes through the rtrb queues in `EngineHandle`; audio to UI through atomics in `SharedPos`. Frees happen on the UI thread (see the garbage queue).
2. **No Renoise code, assets, samples, or branding.** Implement from user-visible behavior and public documentation. Do not name the product Renoise. XRNS import exists for interoperability only.
3. **Parity work is tracked in `docs/PARITY.md`.** Tick items there when finished; add items you discover.
4. **Tests accompany behavior.** Engine behavior gets a headless render test; format changes get a roundtrip test; every new file format reader needs fixtures.
5. Keep `trakr-core` types serde-stable: adding a field needs `#[serde(default)]`, renaming/removing needs a `FORMAT_VERSION` bump plus a migration.

## Known scaffold shortcuts (fix these early)

- UI to engine sync sends an `Arc<Song>` snapshot on every edit (`Editor::sync`), cheap because `Song.patterns` are `Arc<Pattern>` (an edit clones only the touched pattern). Undo is snapshot-based (`Snapshot`, 500 deep). Incremental edit commands would need dense or preallocated pattern storage in the engine (BTreeMap inserts allocate); not done.
- Engine ignores: special volume/panning column commands, NNA modes, filters, DSP devices, automation, effects on delayed notes. Implemented: delay column, `ZT ZL ZB 0S 0B 0C 0Q 0R 0U 0D 0G 0V 0T 0A 0W 0I 0O` (table in `trakr_core::effects`), volume/pan column on running notes, instrument volume/pan/pitch envelopes, all loop modes, track mute/solo/off, master volume, meters, song loop range. Per-tick effects run in `Voice::tick`; LFOs and envelopes run in real time in `Voice::retarget`. Mixing: voices write into one stereo bus per track (max 64, `MAX_METER_TRACKS`); each track's device chain (`devices.rs`, built on the UI thread via `devices::create_or_passthrough` and sent as `Command::SetChain`) runs on its bus, then the fader (volume/pan/mute/solo, ramped per block) sums into the master bus, which runs the master chain and master volume. Device parameters are read from the `Song` snapshot each block; pattern automation overrides them per block. `Editor::sync_chains` diffs device id lists per track and only rebuilds changed chains. Voices hold `Arc` clones of sample data and envelopes: dropping the last reference on the audio thread frees there (rare: old undo snapshots normally keep them alive).
- No song loop range. Sample editing lives in `trakr-core::Sample` (range ops, slices, resample) with undo via `Editor::edit_sample`; no resampling on import, no sample view in the TUI. Recording goes through `trakr_engine::InputRecorder` (cpal impl behind the `audio` feature, injected by the binary via `Editor::set_recorder_factory`).
- TUI keymap is mine, not Renoise's, apart from the tracker-style piano layout, F1/F2 octave, backtick note-off. Terminals cannot report key release or distinguish left/right modifiers; design around that.

## GUI notes

- `trakr-gui` has synthetic-input tests (`crates/trakr-gui/src/tests.rs`) that run real egui frames: use them for any input or layout change. They caught a freeze: never call `Response` methods (`clicked()`, `drag_started()`, ...) or `ctx.input` inside `ctx.memory_mut`/`ui.memory_mut` closures; the context lock is not reentrant.
- `ctx.wants_keyboard_input()` is true whenever any widget has focus (including the pattern grid), so it cannot gate editor keys. `GuiApp::text_editing` (IME output of the previous frame) does.
- Live GUI check without a display: `xvfb-run` + `openbox` + `xdotool` against `cargo run -p trakr-gui --example demo` (needs a window manager for focus).

## Conventions

- Rust 2021, `cargo fmt` (max_width 110), clippy clean.
- Errors: `thiserror` in libraries, `anyhow` only in binaries/TUI glue.
- Internal note numbering: 0..=119 = C-0..B-9, 48 = C-4 = MIDI 60, 120 = OFF, 121 = empty. Instrument/volume/pan empty = 0xFF; delay empty = 0.
- Effect ids are two ASCII bytes (`*b"0A"`), `[0,0]` = empty.
- Prefer small, reviewable commits that keep `cargo test --workspace` green.

## Suggested order of work

See docs/ROADMAP.md. Editor core (undo, selection, clipboard, block/sequence/track ops) is done; see ROADMAP for what is next.
