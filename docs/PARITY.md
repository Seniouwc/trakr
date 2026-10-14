# Renoise parity checklist

Compiled from memory of Renoise 3.x behavior. Verify each item against the official manual before implementing; add anything missing.
`[x]` = working in the scaffold, `[~]` = partial, `[ ]` = not started.

## Song and sequencer
- [x] BPM, LPB, TPL stored per song
- [~] Pattern sequence (play order) with song loop; no loop range
- [~] Sequence editor: insert, clone, delete, reorder, pick pattern per slot work; section markers and pattern matrix do not
- [~] Pattern length 1..512 editing and pattern names work; resize with content scaling does not
- [~] Sub-line delay column works (1/256 line); tick-quantized timing exists only for the effects `0Q`, `0C`, `0R`; per-tick voice processing (slides, vibrato) does not
- [~] Song loop range (slot range) and pattern loop work; block loop does not
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
- [~] Phrases: per-instrument phrases (own LPB, note/effect columns, loop region, base note, key range) played by key-mapped trigger notes with transposition, editable in the Phrases dock; undo, `.trkr` roundtrip. Missing: program mode, shuffle, phrase-local `Z` commands, phrase `.xrns`/`.xrni` I/O, phrase recording

## Volume, panning, and effect commands
- [~] Volume column: 00-80 sets the note volume, and on lines without a note it changes the sounding note (usable as a per-line volume envelope; Block > Fade in/out writes a ramp). Special volume-column commands (glide, slide, fade letters) do not exist
- [~] Panning column: 00-80 works the same way (on notes and on running notes); special commands do not
- [~] Per-note effects implemented (effect column = whole track, per-note slot = that column; ids are trakr's own and unverified against Renoise, see `trakr_core::effects::EFFECTS`): `0S` sample offset, `0B` backwards, `0C` cut, `0Q` delay, `0R` retrigger, `0U`/`0D` pitch slide, `0G` glide, `0V` vibrato, `0T` tremolo, `0A` arpeggio, `0W` volume slide, `0I`/`0O` fade in/out. Still missing: envelope control, track volume/pan/width commands, auto pan, effects on delayed notes
- [~] Global Z-commands: ZT (BPM), ZL (LPB), ZB (pattern break to line xx in the next slot) work; delay, ghost-note, and the rest do not
- [ ] Effect commands as device parameter targets (`Xnn` style device addressing, verify)

## Samples and instruments
- [~] Sample playback with linear interpolation; forward, reverse and ping-pong loops
- [x] Import WAV, AIFF, FLAC, OGG, MP3 (symphonia); RX2/REX not required initially
- [~] Reverse and ping-pong loops work; loop release, one-shot, sustain loop do not
- [~] Interpolation modes: linear, cubic (Hermite, default) and 8-point sinc per instrument; no "none" mode, no oversampling, sinc does not low-pass when pitching up
- [~] Sample editor (GUI): zoomable waveform, selection, cut/copy/paste/delete/crop, silence, fades, gain, normalize, invert, DC removal, reverse, mono/stereo, tape pitch shift, resample, WAV export (16/24/32f). Missing: loop crossfade, dither, spectrum view, draw-in-waveform editing, TUI sample view
- [~] Slice markers (manual, shift-click, even split) and split-to-keys work; beat-sync, time-stretch, transient detection do not
- [~] Key zones and velocity zones (layers), per-sample volume/pan/tune, and per-instrument NNA (cut, note-off, continue) work; mute-groups do not
- [~] Instrument volume, pan and pitch envelopes (points, sustain hold, GUI editor) work; cutoff/resonance/drive, LFOs, key/velocity tracking, stepper, random, envelope loops and fade-out do not
- [~] Instrument filter (LP/HP/BP/Notch, cutoff, resonance, cutoff envelope) works; more filter types, drive, and a per-instrument FX chain do not
- [ ] Macros (instrument macros) and MIDI input properties
- [~] MIDI instrument routing works: an instrument can send its notes (channel, optional program change, velocity from the volume column, note-off from OFF/next note/stop) to the MIDI output port, with or without its samples; not yet CC/pitch-bend lanes or per-note properties
- [~] Plugin hosting, local only: CLAP audio effects (mono/stereo) load from `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`, `$CLAP_PATH` via Devices > Add plugin; parameters appear as sliders and can be automated; plugin and parameter ranges are saved in the song; offline render uses plugins. Missing: plugin instruments and note input, plugin GUIs, plugin state save/restore, latency compensation, multi-port/sidechain, VST2/VST3/LV2/AU, out-of-process sandboxing (a crashing plugin crashes trakr), remote/network plugins
- [ ] Plugin instruments (VST/CLAP/LV2); instrument bank/XRNI import/export
- [ ] Sample/instrument disk browser, favorites

- [x] Record from audio input (cpal) into a new instrument; render song to sample or WAV (offline)

## Mixer and DSP
- [~] Track volume and panning applied in mix; master track volume applied (master DSP chain not yet)
- [~] Device chain runs before the track fader (post-fader chain and pre/post split do not exist); track width and track delay do not
- [x] Master track with its own DSP chain
- [~] Group tracks (nested, with their own chain and fader) and send tracks (post- or pre-fader feeds, mixer UI) work; `#Send`-style devices, routing to arbitrary tracks, sidechain do not
- [x] Track mute, solo and off, mixer view with per-track and master meters and peak hold
- [ ] Plugin delay compensation
- [~] Native devices (20, incl. 2 modulators): Gainer, Filter, 3-band EQ, 10-band EQ, Ladder filter (24 dB), Gate, Compressor, Limiter, Delay (ping-pong), Reverb, Chorus/flanger, Phaser, Distortion, Bit crusher, Ring modulator, Stereo width, Tremolo/auto-pan, Exciter; GUI cards, bypass, reorder, undo, automation. Still missing (verify against manual): scream/analog filter models, stereo expander variants, bus compressor, maximizer, multitap, cabinet sim, convolver, repeater, DC offset, hydra, key/velocity tracker, formula, meta mixer, doofer, #Line Input, #Multiband Send, #Send, #Sidechain, signal follower

## Automation and modulation
- [x] Per-pattern automation lanes for track volume/pan and any native device parameter (breakpoint curves, GUI editor, playhead)
- [~] Envelope editing: points and straight lines work; curves, interpolation modes, scaling, copy/paste, recording from UI/MIDI do not
- [~] Automation playback works (evaluated per 512-frame block); ranges and "write to pattern" do not
- [~] Modulation devices: LFO (sine/tri/saw/square/random) and envelope follower, each routed to any parameter of a device in the same chain (control rate: one value per 512-frame block, applied one block later). Missing: hydra, formula, key/velocity tracker, signal follower, meta mixer, tempo-synced LFO, cross-track targets

## Plugins
- [ ] CLAP hosting (instrument + effect), parameter automation, state save/load, GUI embedding (likely external window; TUI shows parameter list)
- [ ] VST3 hosting; VST2 only if legally and practically viable
- [ ] LV2, LADSPA/DSSI
- [ ] Out-of-process sandboxing, crash recovery, plugin scan cache, blocklist

## IO, control, scripting
- [x] MIDI output and clock master: Play > MIDI output picks a port, sends 24 ppqn timing clock plus start/stop with the transport (optional), latency setting aligns it with the audio device; timing is stamped per frame by the engine and delivered by a helper thread (jitter about a millisecond, not hardware-locked)
- [~] MIDI input works (midir): notes play and record (velocity to the volume column), CC7/CC10 set cursor-track volume/pan, start/continue/stop control the transport, device picker in Play > MIDI input. Missing: clock slave (external sync), pitch bend, MIDI mapping of arbitrary parameters, channel filtering
- [ ] OSC server and client
- [~] Lua 5.4 scripting (mlua, vendored): sandboxed (no io/os/load, instruction/time/memory limits), runs on a copy of the song so a run is one undo step and errors change nothing; API covers song settings, patterns, sequence, note and effect cells, cursor/selection context, status, transport requests; Scripts menu with examples and `<config>/scripts/*.lua`, console window with reference. Missing: instrument/sample/device/automation access, track editing, callbacks and observers, custom menu entries/keybindings from scripts, GUI toolkit, OSC/websocket
- [ ] Audio backends: ALSA (cpal, working), JACK, PipeWire native; device selection and buffer size UI
- [~] Record: live audio input to sample works; recording MIDI into the pattern works in edit mode (no quantize, no chords-to-columns)

## Render, export, files
- [ ] Offline render to WAV/FLAC/OGG/MP3: whole song, selection, single track, per-track stems, per-pattern, with bit depth/dither options
- [ ] Sample-accurate loop export
- [~] Native `.trkr` project save/load
- [~] `.xrns` import: metadata, tracks, patterns, notes, effects, sequence, and instruments with decoded sample audio (volume, pan, transpose/finetune, loops, key mapping; warnings for anything skipped). Element names and the `SampleData/` layout are from memory and tested only against synthetic fixtures. Missing: instrument envelopes, filters, devices, automation, phrases, groups, sends
- [~] `.xrns` export (File > Export as .xrns): metadata, tracks, patterns, sequence, instruments with 24-bit WAV samples; roundtrip-tested through the importer; warnings for envelopes, devices, automation, routing
- [ ] `.xrni` and `.xrnt` import/export; track DSP presets; theme files
- [~] MOD import (4 to 99 channels; instruments, loops, finetune, volumes, Amiga panning, timing, common effects; unsupported effects reported); S3M, XM, IT do not
- [x] Autosave (every 60 s while there is unsaved work, beside the project or in the config dir), crash recovery prompt, recent files menu
- [ ] Backups of previous saves

## Application
- [~] TUI pattern editor (single pattern view, cursor, edit mode)
- [ ] TUI views: sequencer/matrix, mixer, instrument editor, sample editor, automation editor, plugin browser, disk browser, settings, help
- [~] Configurable keymap (Preferences > Keys: set, clear, reset per action; conflicts and reserved typing keys refused; saved), themes (dark, light, high contrast), interface scale, pattern font size, mouse support. No Renoise-compatible preset and no command palette
- [~] Preferences storage (octave, edit step, follow, recent files, autosave interval in `~/.config/trakr/prefs.json`); window layout and session restore are not saved
- [ ] Optional GUI frontend reusing core/engine
