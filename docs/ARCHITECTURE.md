# Architecture

## Crate graph

```
trakr (bin) -> trakr-gui -> trakr-editor -> trakr-engine -> trakr-core
            -> trakr-xrns -----------------> trakr-core
            -> trakr-format ---------------> trakr-core
```

## Threads

- **UI thread** owns the authoritative `Song` (`App.song: Arc<Song>`), handles input, renders.
- **Audio thread** (cpal callback) owns `Engine`, which holds its own `Arc<Song>` snapshot and all playback state (voices, position, tempo overrides).
- UI -> audio: `rtrb` SPSC queue of `Command`. Audio -> UI: atomics (`SharedPos`) for transport position, and a garbage queue returning replaced `Arc<Song>`s so they are freed off the RT thread.

## Edit model

- `trakr-editor::Editor` owns the authoritative `Arc<Song>`. Every user-level mutation calls `checkpoint()` (pushes a `Snapshot`) then mutates via `Arc::make_mut`, then `sync()`.
- `Song.patterns` is `Vec<Arc<Pattern>>`: a snapshot is a few `Arc` clones, an edit copies only the pattern it touches, and the engine's `SetSong` swap is cheap. Samples are `Arc<Vec<f32>>` too.
- Undo/redo swap whole `Snapshot`s (patterns, sequence, tracks, instruments, tempo). Song load/new clears history.
- `sync()` retries from `tick()` if the engine queue was full, so the engine never stays stale.
- Frontends map native input to `KeyEvent` / `Action` and only render. Menus call `Editor::do_action`.

## Engine internals

- `render(out)` produces interleaved stereo f32 in chunks that end at line boundaries; `frames_to_line` is a fractional accumulator so tempo is sample-accurate on average.
- Line trigger walks every pattern track at the current line: note columns -> voices, effect columns -> engine state.
- TODO (needed for parity): tick-level timing (`tpl`), per-voice effect state, NNA, per-track DSP chain with sends/groups, automation, plugin hosts. Mixer should become a graph of track buses processed in dependency order.

## Formats

- `.trkr`: zip with `song.json` (`{format_version, song}`) and raw f32 sample files. Planned: FLAC samples, plugin state blobs, migrations.
- `.xrns`: zip with `Song.xml` plus sample files. Importer is best-effort; verify against real files before trusting.

## Decisions

- Patterns are sparse (`BTreeMap<u16, Line>` per track): memory stays small at 512 lines x many tracks, and serialization stays compact. Revisit if line iteration shows up in profiles.
- Samples are `Arc<Vec<f32>>`, skipped by serde, stored as separate zip entries. Voices clone the Arc; dropping the last reference on the audio thread would free there, so sample deletion must go through the garbage path.
- The GUI is a thin client: all behavior that is not presentation belongs in the editor/engine/core crates.
