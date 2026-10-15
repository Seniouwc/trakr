# Renoise parity checklist

Compiled from memory of Renoise 3.x behavior. Verify each item against the official manual before implementing; add anything missing.
`[x]` = working in the scaffold, `[~]` = partial, `[ ]` = not started.

## Song and sequencer
- [x] BPM, LPB, TPL stored per song
- [~] Pattern sequence (play order) with song loop; no loop range
- [x] Sequence editor: insert, clone, delete, reorder, pick pattern per slot; named sections (Loop section plays one; a section repeat count plays it N times before the song moves on); pattern matrix dock (tracks x slots, click to mute a track in one slot, drag a slot number to move the slot); per-slot track mutes follow slot edits and undo
- [~] Pattern length 1..512 editing and pattern names work; resize with content scaling does not
- [~] Sub-line delay column works (1/256 line); tick-quantized timing exists only for the effects `0Q`, `0C`, `0R`; per-tick voice processing (slides, vibrato) does not
- [x] Song loop range (slot range), pattern loop and block loop (Play > Loop selected block) work
- [~] Follow playback, play from cursor, play pattern, stop work; play block, panic do not
- [x] Song title, artist and comments (Pattern > Song properties); exports carry them as tags (WAV INFO, FLAC/Vorbis/Opus comments, ID3 for MP3; AAC/ADTS has no tag place)
- [x] Groove / shuffle (up to eight lines per cycle, in Song properties) and tempo / lines-per-beat automation lanes (Automation dock: Tempo and Lines per beat, with curves)

## Pattern editor
- [x] 12 note columns per track (data model), 8 effect columns (data model)
- [x] Note, instrument, volume, panning, delay, per-column sample effect fields
- [~] Cell entry: piano keys, hex digits, effect id and amount, clear
- [x] Add/remove note and effect columns; per-track show/hide of the volume, panning and delay sub-columns (context menu in the pattern view; the cursor skips hidden fields)
- [~] Block selection (multi-track, Shift+arrows or mouse), cut/copy/paste, mix paste (Ctrl-Shift-V keeps what is already there) and flood paste (Ctrl-Alt-V repeats the clipboard to the end of the selection or pattern); no track/line quick-select
- [x] Undo/redo (500 steps, snapshot based)
- [~] Block ops: transpose, interpolate (volume, pan, effect amounts), reverse, clear, randomize volumes, expand (double), shrink (half) and repeat-to-end work (Block menu); duplicate-pattern-range does not
- [~] Insert/delete line in a track works; note-column row insert/delete and column shifting do not
- [x] Pattern/track/line highlight settings, beat/bar markers, hex or decimal line numbers (Preferences)
- [~] Track operations: add, remove, rename, mute, solo, group and track colors (context menu) work; collapse and move do not
- [~] Find / replace (Ctrl-F) by note, instrument or effect id in the selection, pattern or song; command palette (Ctrl-P). "Pattern properties" and sub-pattern transposition do not exist
- [~] Phrases: per-instrument phrases (own LPB, note/effect columns, loop region, base note, key range) played by key-mapped trigger notes with transposition, editable in the Phrases dock; undo, `.trkr` roundtrip. Program mode (notes C-0..B-0 pick a phrase, other notes play it transposed) and per-phrase shuffle (swing) exist. Missing: phrase-local `Z` commands, phrase `.xrns`/`.xrni` I/O, phrase recording

## Volume, panning, and effect commands
- [~] Volume column: 00-80 sets the note volume, and on lines without a note it changes the sounding note (usable as a per-line volume envelope; Block > Fade in/out writes a ramp). Letter commands G (glide), U/D (slide), I/O (fade in/out), B (backwards) and C (cut) with a hex digit are typed with the letter key and shown as `G4`; they travel with delayed notes too. The letter set and scaling are trakr's own (unverified against Renoise)
- [~] Panning column: 00-80 works the same way (on notes and on running notes); it takes the same letter commands as the volume column
- [~] Per-note effects implemented (effect column = whole track, per-note slot = that column; ids are trakr's own and unverified against Renoise, see `trakr_core::effects::EFFECTS`): `0S` sample offset, `0B` backwards, `0C` cut, `0Q` delay, `0R` retrigger, `0U`/`0D` pitch slide, `0G` glide, `0V` vibrato, `0T` tremolo, `0A` arpeggio, `0W` volume slide, `0I`/`0O` fade in/out. Also `0M` note volume, `0N` auto pan, `0K` pan slide, `0Y` maybe (probability), `0L`/`0P`/`0H` track volume, pan and stereo width, `0X` stop track, `ZK` ticks per line (ids are from memory and unverified). Effects written beside a delayed note now apply to it. Still missing: envelope control
- [~] Global Z-commands: ZT (BPM), ZL (LPB), ZB (pattern break to line xx in the next slot) work; delay, ghost-note, and the rest do not
- [ ] Effect commands as device parameter targets (`Xnn` style device addressing, verify)

## Samples and instruments
- [~] Sample playback with linear interpolation; forward, reverse and ping-pong loops
- [x] Import WAV, AIFF, FLAC, OGG, MP3 (symphonia); RX2/REX not required initially
- [~] Reverse and ping-pong loops work; loop release (loop until note-off, then play on) and one-shot (play through, ignore note-off) are per-sample switches; sustain loop does not
- [~] Interpolation modes: none (nearest), linear, cubic (Hermite, default) and 8-point sinc per instrument (sinc is band-limited when pitching up, so high content does not alias); no oversampling
- [~] Sample editor (GUI): draw mode (drag on the waveform to draw samples), pitch shift keeping length or formants, zoomable waveform, selection, cut/copy/paste/delete/crop, silence, fades, gain, normalize, invert, DC removal, reverse, mono/stereo, tape pitch shift, resample, export to every format above. Also loop crossfade, time-stretch (WSOLA, pitch kept), slice at detected transients, bit-depth reduction with shaped dither, a spectrum view and beat sync (the sample is played so it lasts N lines at the song tempo; pitch follows speed). Missing: formant-preserving time stretch (pitch shift can keep formants)
- [x] Slice markers (manual, shift-click, even split, transient detection) and split-to-keys work
- [~] Key zones and velocity zones (layers), per-sample volume/pan/tune, and per-instrument NNA (cut, note-off, continue) work; mute-groups do not
- [~] Instrument volume, pan, pitch and cutoff envelopes (points, sustain hold, GUI editor) work. Per-note LFOs (up to four; volume, pan, pitch, cutoff; five shapes, fade-in), key tracking and velocity-to-cutoff work (LFOs tab). Stepper and smooth-random LFOs, resonance modulation, key / velocity / macro routings (four macro knobs, moved live by `M1`-`M4`), envelope loops while held, and note fade-out work. Drive modulation does not
- [~] Instrument filter (LP/HP/BP/Notch, cutoff, resonance, cutoff envelope) works; instrument drive (saturation, also an LFO/macro target) and a per-instrument FX chain (any native effect, FX tab) exist; more filter types do not. The FX chain of an instrument shared by several tracks mixes into the track of its latest voice
- [~] Instrument macros (four knobs routed to any target, pattern commands `M1`-`M4`); MIDI input properties do not exist
- [~] MIDI instrument routing works: an instrument can send its notes (channel, optional program change, velocity from the volume column, note-off from OFF/next note/stop) to the MIDI output port, with or without its samples; not yet CC/pitch-bend lanes or per-note properties
- [~] Plugin hosting, local only: CLAP audio effects (mono/stereo) load from `~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`, `$CLAP_PATH` via Devices > Add plugin; parameters appear as sliders and can be automated; plugin and parameter ranges are saved in the song; offline render uses plugins. Plugin instruments are supported (see above). Plugins open their own floating window (Open window on the device card or the instrument's Plugin tab; X11 or Wayland, host timers, file descriptors, threads checks and logging provided), parameters turned in the window flow back to the sliders, and the plugin's state is saved with the song and restored on load. Missing: embedded (non-floating) windows, window scaling and transient parenting, latency compensation, multi-port/sidechain, VST2/VST3/LV2/AU, out-of-process sandboxing (a crashing plugin crashes trakr), remote/network plugins
- [~] Plugin instruments: CLAP instruments (note input, mono/stereo output) play as an instrument's "Plugin" tab; notes from patterns, live keys and MIDI reach the plugin sample-accurately per chunk, its audio joins the track the note was played on, parameters are sliders saved in the song, offline render includes it. Missing: VST/LV2, polyphonic note expressions, MIDI input dialect plugins, multi-output; instrument bank/XRNI import/export
- [ ] Sample/instrument disk browser, favorites

- [x] Record from audio input (cpal) into a new instrument; render song to sample or WAV (offline)

## Mixer and DSP
- [~] Track volume and panning applied in mix; master track volume applied (master DSP chain not yet)
- [~] Device chain runs before the track fader (post-fader chain and pre/post split do not exist); track width (`0H` command or Stereo width) and a Track delay device exist
- [x] Master track with its own DSP chain
- [~] Group tracks (nested, with their own chain and fader) and send tracks (post- or pre-fader feeds, mixer UI) work; `#Send`-style devices, routing to arbitrary tracks, arbitrary sidechain taps beyond the sidechain compressor do not
- [x] Track mute, solo and off, mixer view with per-track and master meters and peak hold
- [~] Plugin delay compensation: hosted plugins report their latency and the other sequencer tracks are delayed to match (up to 8192 frames); groups and sends are not compensated
- [~] Native devices (44, incl. 7 modulators): Gainer, Filter, 3-band EQ, 10-band EQ, Ladder filter (24 dB), Gate, Compressor, Limiter, Delay (ping-pong), Reverb, Chorus/flanger, Phaser, Distortion, Bit crusher, Ring modulator, Stereo width, Tremolo/auto-pan, Exciter; GUI cards, bypass, reorder, undo, automation. Also Sidechain compressor (key from another track), Bus compressor (RMS, soft knee), Multiband compressor (3 bands, Linkwitz-Riley), Multitap delay, Repeater, Cabinet (filter model), DC offset, Hydra. Also Maximizer, Analog filter (state-variable, drive), Mid/side, Convolver (partitioned FFT, impulse response from any sample), Key tracker, Velocity tracker, Meta mixer (one knob, four targets), Formula (typed expression, 4 knobs), #Send marker. Also Comb filter, Flanger, Pitch shifter, Saturator, Transient shaper, Track delay and Auto filter. Still missing (verify against manual): doofer, #Line Input, #Multiband Send

## Automation and modulation
- [x] Per-pattern automation lanes for track volume/pan and any native device parameter (breakpoint curves, GUI editor, playhead)
- [~] Envelope editing: points with curve shapes (linear, step, smooth, ease in/out), flip, reverse, scale values and time, copy/paste between envelopes work; recording from UI/MIDI does not
- [~] Automation lanes have curve shapes, a range (copy, paste, clear, flip, reverse, scale, shift) and tempo / LPB targets; playback is evaluated per 512-frame block; "write to pattern" does not
- [~] Modulation devices: LFO (sine/tri/saw/square/random) and envelope follower, each routed to any parameter of a device in the same chain (control rate: one value per 512-frame block, applied one block later). Hydra (one oscillator, three targets) and a tempo-synced LFO exist. Formula, key/velocity tracker and meta mixer exist. Missing: cross-track targets

## Plugins
- [~] CLAP hosting: effects and instruments, parameter automation, state save/load and floating windows work (see above); embedded windows do not
- [ ] VST3 hosting; VST2 only if legally and practically viable
- [ ] LV2, LADSPA/DSSI
- [ ] Out-of-process sandboxing, crash recovery, plugin scan cache, blocklist

## IO, control, scripting
- [x] MIDI output and clock master: Play > MIDI output picks a port, sends 24 ppqn timing clock plus start/stop with the transport (optional), latency setting aligns it with the audio device; timing is stamped per frame by the engine and delivered by a helper thread (jitter about a millisecond, not hardware-locked)
- [~] MIDI input works (midir): notes play and record (velocity to the volume column), CC7/CC10 set cursor-track volume/pan, start/continue/stop control the transport, device picker in Play > MIDI input. Incoming MIDI clock can set the song tempo (Play > MIDI mapping). MIDI learn maps any controller to a track volume/pan, a device parameter or a command. Pitch wheel bends all sounding voices by +-2 semitones. Missing: channel filtering
- [~] OSC server (UDP, this machine only unless enabled otherwise in Preferences): transport, BPM, any command by name, track volume/pan/mute/solo, device parameters, note on/off (see the Preferences hint and `trakr-editor/src/osc_ops.rs`). OSC feedback to a control surface (Preferences > General: host:port) sends play state, position, tempo and each track's volume, pan, mute, solo and level whenever they change
- [~] Lua 5.4 scripting (mlua, vendored): sandboxed (no io/os/load, instruction/time/memory limits), runs on a copy of the song so a run is one undo step and errors change nothing; API covers song settings, patterns, sequence, note and effect cells, cursor/selection context, status, transport requests; Scripts menu with examples and `<config>/scripts/*.lua`, console window with reference. API also covers instruments, tracks, devices and automation lanes. Scripts can read and write sample audio (`trakr.sample*`) and hook events (scripts/hooks/load|save|play|stop.lua). Missing: adding or removing tracks, observers while playing, custom menu entries/keybindings from scripts, GUI toolkit, websocket
- [~] Audio backends: ALSA via cpal (also PulseAudio/PipeWire through ALSA); Preferences > Audio picks host, output and input device, sample rate (up to the device's maximum) and buffer size, and restarts the engine without losing the song, chains or MIDI output; JACK builds with `--features trakr-engine/jack` (needs libjack). Missing: PipeWire native, per-channel output routing, latency compensation, input monitoring/live effects
- [~] Record: live audio input to sample works; recording MIDI into the pattern works in edit mode (no quantize, no chords-to-columns)

## Render, export, files
- [~] Offline render: whole song, the loop range and each track as a stem, to WAV (16/24/32f), FLAC (16/24), Ogg Vorbis, MP3, AAC (ADTS) or Opus, with a 0-10 quality scale per lossy codec (10 = Vorbis q1.0, MP3 CBR 320, Opus 510 kbps, AAC 512 kbps), TPDF dither, and a render sample rate up to 192 kHz (songs render at that rate directly); CPU and voice meters in the status bar. Missing: tags/metadata, M4A container, per-pattern render, encoder-side loudness normalisation
- [ ] Sample-accurate loop export
- [~] Native `.trkr` project save/load
- [~] `.xrns` import: metadata, tracks, patterns, notes, effects, sequence, and instruments with decoded sample audio (volume, pan, transpose/finetune, loops, key mapping; warnings for anything skipped). Element names and the `SampleData/` layout are from memory and tested only against synthetic fixtures. Missing: instrument envelopes, filters, devices, automation, phrases, groups, sends
- [~] `.xrns` export (File > Export as .xrns): metadata, tracks, patterns, sequence, instruments with 24-bit WAV samples; roundtrip-tested through the importer; warnings for envelopes, devices, automation, routing
- [~] `.xrni` instrument import/export (File menu in the instrument panel; element names from memory, only tested through our own export), native `.trki` instrument files and an instrument bank folder; native `.trkt` track DSP presets (Presets menu in the device panel: save, add, replace); `.xrnt` is not read or written
- [~] MOD import (4 to 99 channels; instruments, loops, finetune, volumes, Amiga panning, timing, common effects; unsupported effects reported); S3M (PCM channels, samples, common effects), XM (key-mapped samples, delta decoding, common effects; envelopes not imported) and IT (samples incl. compressed ones, instruments with note maps and volume/pan envelopes, patterns, common effects; format details from memory, tested against built modules only) import by content
- [x] Autosave (every 60 s while there is unsaved work, beside the project or in the config dir), crash recovery prompt, recent files menu
- [x] Backup of the previous file as `<name>.bak` when saving over it (Preferences)

## Application
- [~] Configurable keymap (Preferences > Keys: set, clear, reset per action; conflicts and reserved typing keys refused; saved), themes (dark, light, high contrast plus Catppuccin Latte/Frappe/Macchiato/Mocha, Dracula, Gruvbox dark/light, Nord, Solarized dark/light, Tokyo Night, One Dark, Rose Pine, Everforest, Monokai, and your own JSON files in `<config>/themes`), interface scale, pattern font size, mouse support. Command palette (Ctrl-P). No Renoise-compatible preset (the real defaults are not documented here)
- [~] Preferences storage (octave, edit step, follow, recent files, autosave interval in `~/.config/trakr/prefs.json`); window size, bottom panel and an optional reopen-last-song setting are saved
- [x] The terminal UI was dropped; the egui GUI is the only frontend
