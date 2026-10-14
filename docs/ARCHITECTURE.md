# Architecture

## Crate graph

```
trakr (bin) -> trakr-tui -> trakr-engine -> trakr-core
            -> trakr-xrns -----------------> trakr-core
            -> trakr-format ---------------> trakr-core
```

## Threads

- **UI thread** owns the authoritative `Song` (`App.song: Arc<Song>`), handles input, renders.
- **Audio thread** (cpal callback) owns `Engine`, which holds its own `Arc<Song>` snapshot and all playback state (voices, position, tempo overrides).
- UI -> audio: `rtrb` SPSC queue of `Command`. Audio -> UI: atomics (`SharedPos`) for transport position, and a garbage queue returning replaced `Arc<Song>`s so they are freed off the RT thread.

## Planned edit model (replaces whole-song snapshots)

1. Every user edit becomes an `Edit` value (set cell, paste block, change pattern length, ...) applied by one function that mutates the UI-side song and returns its inverse. That gives undo/redo for free.
2. The same `Edit` (or a compact RT-safe form of it) is sent to the engine, which applies it to its own copy of the data. Pattern data must then be engine-owned and mutated in place, with large structures (sample data, new patterns) allocated on the UI thread and handed over by pointer.
3. Song-level structural changes (add track, load song) can still use a snapshot swap.

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
- TUI is a thin client: all behavior that is not presentation belongs in core/engine so a future GUI can reuse it.
