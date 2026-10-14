# Renoise parity checklist

Compiled from memory of Renoise 3.x behavior. Verify each item against the official manual before implementing; add anything missing.
`[x]` = working in the scaffold, `[~]` = partial, `[ ]` = not started.

## Song and sequencer
- [x] BPM, LPB, TPL stored per song
- [~] Pattern sequence (play order) with song loop; no loop range
- [~] Sequence editor: insert, clone, delete, reorder, pick pattern per slot work; section markers and pattern matrix do not
- [~] Pattern length 1..512 editing and pattern names work; resize with content scaling does not
- [ ] Tick-accurate timing (TPL) and sub-line delay column
- [ ] Song loop range / pattern loop / block loop
- [~] Follow playback, play from cursor, play pattern, stop work; play block, panic do not
- [ ] Song comments, metadata, tags
- [ ] Groove / shuffle, global BPM/LPB automation via Zxx commands

## Pattern editor
- [x] 12 note columns per track (data model), 8 effect columns (data model)
- [x] Note, instrument, volume, panning, delay, per-column sample effect fields
- [~] Cell entry: piano keys, hex digits, effect id and amount, clear
- [~] Add/remove note and effect columns work; per-sub-column (volume, panning, delay) show/hide does not
- [~] Block selection (multi-track, Shift+arrows or mouse), cut/copy/paste (overwrite only); no mix/flood paste, no track/line quick-select
- [x] Undo/redo (500 steps, snapshot based)
- [~] Block ops: transpose, interpolate (volume, pan, effect amounts), reverse, clear work; randomize, double/halve, expand/shrink, duplicate do not
- [~] Insert/delete line in a track works; note-column row insert/delete and column shifting do not
- [ ] Pattern/track/line highlight settings, beat/bar markers, hex or decimal line numbers
- [~] Track operations: add, remove, rename, mute flag work (mute not yet audible); color, collapse, move, group, solo do not
- [ ] Find/replace in patterns, "pattern properties", sub-pattern transposition
- [ ] Phrase editor and phrase playback (per-instrument phrases, keymapping, program/loop modes)

## Volume, panning, and effect commands
- [ ] Volume column: 00-80 volume, special commands (verify list: Gxx glide, Uxx/Dxx slide, Ixx/Oxx fade, Axx/Bxx? etc.)
- [ ] Panning column: 00-80 plus special commands
- [ ] Per-note effects (verify full list against manual): arpeggio, pitch slide up/down, glide to note, volume slide, cut, delay, retrigger, sample offset, backwards, vibrato, tremolo, auto pan, envelope control, track volume/pan/width
- [~] Global Z-commands: ZT (BPM) and ZL (LPB) work; pattern break, delay, ghost-note, and the rest do not
- [ ] Effect commands as device parameter targets (`Xnn` style device addressing, verify)

## Samples and instruments
- [~] Sample playback with linear interpolation, forward loop
- [x] Import WAV, AIFF, FLAC, OGG, MP3 (symphonia); RX2/REX not required initially
- [ ] Reverse and ping-pong loop, loop release, one-shot, sustain loop
- [ ] Interpolation modes (none, linear, cubic, sinc), oversampling
- [ ] Sample editor: waveform, selection, cut/copy/paste, fades, normalize, DC offset, reverse, crop, silence, resample, bit-depth/dither on export
- [ ] Slicing and slice markers, beat-sync (repitch/time-stretch), auto-seek, transient detection
- [ ] Key zones and velocity zones, layers, per-sample volume/pan/tune/NNA/mute-group
- [ ] Instrument volume/pan/pitch/cutoff/resonance/drive modulation sets (envelopes, LFOs, key/velocity tracking, stepper, random)
- [ ] Instrument filters (types per manual), per-instrument FX chain (sample FX chain)
- [ ] Macros (instrument macros) and MIDI input properties
- [ ] Plugin instruments (VST/CLAP/LV2) and MIDI instrument routing; instrument bank/XRNI import/export
- [ ] Sample/instrument disk browser, favorites

## Mixer and DSP
- [~] Track volume and panning applied in mix
- [ ] Pre/post mixer split, track width, track delay (ms compensation)
- [ ] Master track with its own DSP chain
- [ ] Send tracks and `#Send` device routing, group tracks, routing to arbitrary tracks, sidechain
- [ ] Track mute, solo, off (with proper latency/CPU behavior), mixer view with meters and peak hold
- [ ] Plugin delay compensation
- [ ] Native devices (verify against manual): EQ (5/10 band, mixer EQ), filters (analog, digital, notch, scream), gainer, stereo expander, compressor, gate, limiter, bus compressor, maximizer, exciter, delay, multitap, reverb, chorus, flanger, phaser, ring mod, distortion, cabinet sim, convolver, repeater, DC offset, LFO, envelope follower, hydra, key tracker, velocity tracker, formula, meta mixer, doofer, #Line Input, #Multiband Send, #Send, #Sidechain, signal follower

## Automation and modulation
- [ ] Per-pattern automation envelopes for any device/track parameter
- [ ] Envelope editing: points, lines, curves, interpolation modes, scaling, copy/paste, record from UI/MIDI
- [ ] Automation playback, ranges, and "write to pattern" (envelope to effect commands)
- [ ] Modulation devices: LFO, envelope follower, hydra, formula, key/velocity tracker, signal follower, meta mixer

## Plugins
- [ ] CLAP hosting (instrument + effect), parameter automation, state save/load, GUI embedding (likely external window; TUI shows parameter list)
- [ ] VST3 hosting; VST2 only if legally and practically viable
- [ ] LV2, LADSPA/DSSI
- [ ] Out-of-process sandboxing, crash recovery, plugin scan cache, blocklist

## IO, control, scripting
- [ ] MIDI input (notes, CC, clock slave), MIDI output, MIDI clock master, MIDI mapping for any parameter
- [ ] OSC server and client
- [ ] Lua scripting API with tool/menu entries (mlua), API surface equivalent to the official one
- [ ] Audio backends: ALSA (cpal, working), JACK, PipeWire native; device selection and buffer size UI
- [ ] Record: live audio input to sample, "record to pattern" from MIDI

## Render, export, files
- [ ] Offline render to WAV/FLAC/OGG/MP3: whole song, selection, single track, per-track stems, per-pattern, with bit depth/dither options
- [ ] Sample-accurate loop export
- [~] Native `.trkr` project save/load
- [~] `.xrns` import (metadata, tracks, patterns, notes, effects, sequence); missing instruments, samples, devices, automation, phrases, groups, sends
- [ ] `.xrns` export; `.xrni` and `.xrnt` import/export; track DSP presets; theme files
- [ ] MOD, S3M, XM, IT import
- [ ] Autosave, backup, crash recovery, recent files

## Application
- [~] TUI pattern editor (single pattern view, cursor, edit mode)
- [ ] TUI views: sequencer/matrix, mixer, instrument editor, sample editor, automation editor, plugin browser, disk browser, settings, help
- [ ] Configurable keymap (Renoise-compatible preset), themes, mouse support, command palette
- [ ] Preferences storage, session restore
- [ ] Optional GUI frontend reusing core/engine
