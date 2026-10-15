# Renoise parity checklist

Compiled from memory of Renoise 3.x behavior. Verify each item against the official manual before implementing; add anything missing.
`[x]` = working in the scaffold, `[~]` = partial, `[ ]` = not started.

## Song and sequencer
- [x] BPM, LPB, TPL stored per song
- [~] Pattern sequence (play order) with song loop; no loop range
- [x] Sequence editor: insert, clone, delete, reorder, pick pattern per slot; named sections (Loop section plays one); pattern matrix dock (tracks x slots, click to mute a track in one slot); per-slot track mutes follow slot edits and undo
- [~] Pattern length 1..512 editing and pattern names work; resize with content scaling does not
- [~] Sub-line delay column works (1/256 line); tick-quantized timing exists only for the effects `0Q`, `0C`, `0R`; per-tick voice processing (slides, vibrato) does not
- [x] Song loop range (slot range), pattern loop and block loop (Play > Loop selected block) work
- [~] Follow playback, play from cursor, play pattern, stop work; play block, panic do not
- [~] Song title, artist and comments (Pattern > Song properties); no tags
- [~] Groove / shuffle (up to eight lines per cycle, in Song properties) works; BPM/LPB automation via lanes does not

## Pattern editor
- [x] 12 note columns per track (data model), 8 effect columns (data model)
- [x] Note, instrument, volume, panning, delay, per-column sample effect fields
- [~] Cell entry: piano keys, hex digits, effect id and amount, clear
- [~] Add/remove note and effect columns work; per-sub-column (volume, panning, delay) show/hide does not
- [~] Block selection (multi-track, Shift+arrows or mouse), cut/copy/paste (overwrite only); no mix/flood paste, no track/line quick-select
- [x] Undo/redo (500 steps, snapshot based)
- [~] Block ops: transpose, interpolate (volume, pan, effect amounts), reverse, clear, randomize volumes, expand (double), shrink (half) and repeat-to-end work (Block menu); duplicate-pattern-range does not
- [~] Insert/delete line in a track works; note-column row insert/delete and column shifting do not
- [ ] Pattern/track/line highlight settings, beat/bar markers, hex or decimal line numbers
- [~] Track operations: add, remove, rename, mute flag work (mute not yet audible); color, collapse, move, group, solo do not
- [~] Find / replace (Ctrl-F) by note, instrument or effect id in the selection, pattern or song; command palette (Ctrl-P). "Pattern properties" and sub-pattern transposition do not exist
- [~] Phrases: per-instrument phrases (own LPB, note/effect columns, loop region, base note, key range) played by key-mapped trigger notes with transposition, editable in the Phrases dock; undo, `.trkr` roundtrip. Missing: program mode, shuffle, phrase-local `Z` commands, phrase `.xrns`/`.xrni` I/O, phrase recording

## Volume, panning, and effect commands
- [~] Volume column: 00-80 sets the note volume, and on lines without a note it changes the sounding note (usable as a per-line volume envelope; Block > Fade in/out writes a ramp). Special volume-column commands (glide, slide, fade letters) do not exist
- [~] Panning column: 00-80 works the same way (on notes and on running notes); special commands do not
- [~] Per-note effects implemented (effect column = whole track, per-note slot = that column; ids are trakr's own and unverified against Renoise, see `trakr_core::effects::EFFECTS`): `0S` sample offset, `0B` backwards, `0C` cut, `0Q` delay, `0R` retrigger, `0U`/`0D` pitch slide, `0G` glide, `0V` vibrato, `0T` tremolo, `0A` arpeggio, `0W` volume slide, `0I`/`0O` fade in/out. Also `0M` note volume, `0N` auto pan, `0Y` maybe (probability), `0L`/`0P` track volume and pan, `0X` stop track, `ZK` ticks per line (ids are from memory and unverified). Still missing: envelope control, track width, effects on delayed notes
- [~] Global Z-commands: ZT (BPM), ZL (LPB), ZB (pattern break to line xx in the next slot) work; delay, ghost-note, and the rest do not
- [ ] Effect commands as device parameter targets (`Xnn` style device addressing, verify)

## Samples and instruments
- [~] Sample playback with linear interpolation; forward, reverse and ping-pong loops
- [x] Import WAV, AIFF, FLAC, OGG, MP3 (symphonia); RX2/REX not required initially
- [~] Reverse and ping-pong loops work; loop release, one-shot, sustain loop do not
- [~] Interpolation modes: linear, cubic (Hermite, default) and 8-point sinc per instrument; no "none" mode, no oversampling, sinc does not low-pass when pitching up
- [~] Sample editor (GUI): zoomable waveform, selection, cut/copy/paste/delete/crop, silence, fades, gain, normalize, invert, DC removal, reverse, mono/stereo, tape pitch shift, resample, export to every format above. Also loop crossfade, time-stretch (WSOLA, pitch kept), slice at detected transients, bit-depth reduction with shaped dither, a spectrum view and beat sync (the sample is played so it lasts N lines at the song tempo; pitch follows speed). Missing: draw-in-waveform editing, formant-preserving stretch
- [x] Slice markers (manual, shift-click, even split, transient detection) and split-to-keys work
- [~] Key zones and velocity zones (layers), per-sample volume/pan/tune, and per-instrument NNA (cut, note-off, continue) work; mute-groups do not
- [~] Instrument volume, pan, pitch and cutoff envelopes (points, sustain hold, GUI editor) work. Per-note LFOs (up to four; volume, pan, pitch, cutoff; five shapes, fade-in), key tracking and velocity-to-cutoff work (LFOs tab). Stepper and smooth-random LFOs, resonance modulation, key / velocity / macro routings (four macro knobs, moved live by `M1`-`M4`), envelope loops while held, and note fade-out work. Drive modulation does not
- [~] Instrument filter (LP/HP/BP/Notch, cutoff, resonance, cutoff envelope) works; more filter types, drive, and a per-instrument FX chain do not
- [~] Instrument macros (four knobs routed to any target, pattern commands `M1`-`M4`); MIDI input properties do not exist
- [~] MIDI instrument routing works: an instrument can send its notes (channel, optional program change, velocity from the volume column, note-off from OFF/next note/stop) to the MIDI output port, with or without its samples; not yet CC/pitch-bend lanes or per-note properties
- [~] Plugin hosting, local only: CLAP audio effects (mono/stereo) load from `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`, `$CLAP_PATH` via Devices > Add plugin; parameters appear as sliders and can be automated; plugin and parameter ranges are saved in the song; offline render uses plugins. Missing: plugin instruments and note input, plugin GUIs, plugin state save/restore, latency compensation, multi-port/sidechain, VST2/VST3/LV2/AU, out-of-process sandboxing (a crashing plugin crashes trakr), remote/network plugins
- [ ] Plugin instruments (VST/CLAP/LV2); instrument bank/XRNI import/export
- [ ] Sample/instrument disk browser, favorites

- [x] Record from audio input (cpal) into a new instrument; render song to sample or WAV (offline)

## Mixer and DSP
- [~] Track volume and panning applied in mix; master track volume applied (master DSP chain not yet)
- [~] Device chain runs before the track fader (post-fader chain and pre/post split do not exist); track width and track delay do not
- [x] Master track with its own DSP chain
- [~] Group tracks (nested, with their own chain and fader) and send tracks (post- or pre-fader feeds, mixer UI) work; `#Send`-style devices, routing to arbitrary tracks, arbitrary sidechain taps beyond the sidechain compressor do not
- [x] Track mute, solo and off, mixer view with per-track and master meters and peak hold
- [ ] Plugin delay compensation
- [~] Native devices (37, incl. 7 modulators): Gainer, Filter, 3-band EQ, 10-band EQ, Ladder filter (24 dB), Gate, Compressor, Limiter, Delay (ping-pong), Reverb, Chorus/flanger, Phaser, Distortion, Bit crusher, Ring modulator, Stereo width, Tremolo/auto-pan, Exciter; GUI cards, bypass, reorder, undo, automation. Also Sidechain compressor (key from another track), Bus compressor (RMS, soft knee), Multiband compressor (3 bands, Linkwitz-Riley), Multitap delay, Repeater, Cabinet (filter model), DC offset, Hydra. Also Maximizer, Analog filter (state-variable, drive), Mid/side, Convolver (partitioned FFT, impulse response from any sample), Key tracker, Velocity tracker, Meta mixer (one knob, four targets), Formula (typed expression, 4 knobs), #Send marker. Still missing (verify against manual): doofer, #Line Input, #Multiband Send

## Automation and modulation
- [x] Per-pattern automation lanes for track volume/pan and any native device parameter (breakpoint curves, GUI editor, playhead)
- [~] Envelope editing: points and straight lines work; curves, interpolation modes, scaling, copy/paste, recording from UI/MIDI do not
- [~] Automation playback works (evaluated per 512-frame block); ranges and "write to pattern" do not
- [~] Modulation devices: LFO (sine/tri/saw/square/random) and envelope follower, each routed to any parameter of a device in the same chain (control rate: one value per 512-frame block, applied one block later). Hydra (one oscillator, three targets) and a tempo-synced LFO exist. Formula, key/velocity tracker and meta mixer exist. Missing: cross-track targets

## Plugins
- [ ] CLAP hosting (instrument + effect), parameter automation, state save/load, GUI embedding
- [ ] VST3 hosting; VST2 only if legally and practically viable
- [ ] LV2, LADSPA/DSSI
- [ ] Out-of-process sandboxing, crash recovery, plugin scan cache, blocklist

## IO, control, scripting
- [x] MIDI output and clock master: Play > MIDI output picks a port, sends 24 ppqn timing clock plus start/stop with the transport (optional), latency setting aligns it with the audio device; timing is stamped per frame by the engine and delivered by a helper thread (jitter about a millisecond, not hardware-locked)
- [~] MIDI input works (midir): notes play and record (velocity to the volume column), CC7/CC10 set cursor-track volume/pan, start/continue/stop control the transport, device picker in Play > MIDI input. Incoming MIDI clock can set the song tempo (Play > MIDI mapping). MIDI learn maps any controller to a track volume/pan, a device parameter or a command. Missing: pitch bend, channel filtering
- [~] OSC server (UDP, this machine only unless enabled otherwise in Preferences): transport, BPM, any command by name, track volume/pan/mute/solo, device parameters, note on/off (see the Preferences hint and `trakr-editor/src/osc_ops.rs`). No OSC client / feedback
- [~] Lua 5.4 scripting (mlua, vendored): sandboxed (no io/os/load, instruction/time/memory limits), runs on a copy of the song so a run is one undo step and errors change nothing; API covers song settings, patterns, sequence, note and effect cells, cursor/selection context, status, transport requests; Scripts menu with examples and `<config>/scripts/*.lua`, console window with reference. API also covers instruments, tracks, devices and automation lanes. Missing: sample access, adding or removing tracks, callbacks and observers, custom menu entries/keybindings from scripts, GUI toolkit, OSC/websocket
- [ ] Audio backends: ALSA (cpal, working), JACK, PipeWire native; device selection and buffer size UI
- [~] Record: live audio input to sample works; recording MIDI into the pattern works in edit mode (no quantize, no chords-to-columns)

## Render, export, files
- [~] Offline render: whole song, the loop range and each track as a stem, to WAV (16/24/32f), FLAC (16/24), Ogg Vorbis, MP3, AAC (ADTS) or Opus, with a 0-10 quality scale per lossy codec (10 = Vorbis q1.0, MP3 CBR 320, Opus 510 kbps, AAC 512 kbps), TPDF dither, and a render sample rate up to 192 kHz (songs render at that rate directly); CPU and voice meters in the status bar. Missing: tags/metadata, M4A container, per-pattern render, encoder-side loudness normalisation
- [ ] Sample-accurate loop export
- [~] Native `.trkr` project save/load
- [~] `.xrns` import: metadata, tracks, patterns, notes, effects, sequence, and instruments with decoded sample audio (volume, pan, transpose/finetune, loops, key mapping; warnings for anything skipped). Element names and the `SampleData/` layout are from memory and tested only against synthetic fixtures. Missing: instrument envelopes, filters, devices, automation, phrases, groups, sends
- [~] `.xrns` export (File > Export as .xrns): metadata, tracks, patterns, sequence, instruments with 24-bit WAV samples; roundtrip-tested through the importer; warnings for envelopes, devices, automation, routing
- [ ] `.xrni` and `.xrnt` import/export; track DSP presets
- [~] MOD import (4 to 99 channels; instruments, loops, finetune, volumes, Amiga panning, timing, common effects; unsupported effects reported); S3M (PCM channels, samples, common effects) and XM (key-mapped samples, delta decoding, common effects; envelopes not imported) import by content; IT does not
- [x] Autosave (every 60 s while there is unsaved work, beside the project or in the config dir), crash recovery prompt, recent files menu
- [x] Backup of the previous file as `<name>.bak` when saving over it (Preferences)

## Application
- [~] Configurable keymap (Preferences > Keys: set, clear, reset per action; conflicts and reserved typing keys refused; saved), themes (dark, light, high contrast plus Catppuccin Latte/Frappe/Macchiato/Mocha, Dracula, Gruvbox dark/light, Nord, Solarized dark/light, Tokyo Night, One Dark, Rose Pine, Everforest, Monokai, and your own JSON files in `<config>/themes`), interface scale, pattern font size, mouse support. Command palette (Ctrl-P). No Renoise-compatible preset (the real defaults are not documented here)
- [~] Preferences storage (octave, edit step, follow, recent files, autosave interval in `~/.config/trakr/prefs.json`); window size, bottom panel and an optional reopen-last-song setting are saved
- [x] The terminal UI was dropped; the egui GUI is the only frontend
