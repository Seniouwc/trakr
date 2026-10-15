# Guide for agents: making music with the native `.trkr` format

You can compose a complete song without the GUI by writing a `.trkr` file, then checking and rendering
it headlessly. The loop is: **write -> `--check` -> `--render` -> inspect numbers -> revise.**

## 1. Tools

```
cargo build -p trakr                       # binary: target/debug/trakr
trakr --example out.trkr                   # a small valid project to look at or start from
trakr --example-full out.trkr              # a project that uses EVERY feature: the exact JSON spelling of every field/enum
trakr --check song.trkr                    # load + validate + render; prints bpm/tracks/peak/rms
trakr --render out.<ext> song.trkr [--rate N] [--seconds N] [--quality 0-10] [--bits 16|24|32] [--stems DIR]
                                           # ext: wav flac ogg mp3 aac opus; --stems writes one file per sequencer track
trakr --devices                            # native effect ids, parameters in order, ranges, defaults
trakr --effects                            # pattern effect commands (two-letter ids)
```

None of these need a display or an audio device. `--check`/`--render` exit non-zero with the reason when
the file is malformed. They also open `.xrns`, `.mod`, `.s3m`, `.xm`, `.it`. The render covers the whole
sequence once; when it ends, notes still held (looped samples awaiting a note-off) are released and given 8 s
to ring out, so a missing note-off cannot hang a render. Song title/author/comments become the file's tags.
Hosted CLAP plugins are not loaded headlessly (those devices pass audio through).

Fastest path: use the helper library `docs/agents/trakr_song.py` (Python standard library only; section 7 lists
its API) and copy `docs/agents/example_beat.py`, a worked four-bar groove (drums, bass, lead, devices, automation):

```
python3 docs/agents/example_beat.py target/debug/trakr out.trkr out.wav
```

`python3 docs/agents/test_trakr_song.py` exercises every helper against the binary (it also runs under
`cargo test -p trakr`). You cannot listen. Judge the result from the numbers `--check` prints and, if needed, from the WAV
(e.g. analyse it with numpy/scipy if available): duration, peak, rms, silence.

## 2. File layout

A `.trkr` is a zip:

- `song.json`: `{"format_version": 1, "song": {...}}`
- `samples/{instrument}_{sample}.f32`: raw little-endian interleaved `f32` PCM (-1..1) for every sample of
  every instrument (0-based indices). A missing file makes the project fail to load.

Unknown JSON fields are ignored; most newer fields are optional (`#[serde(default)]`). To be safe against
schema changes, **start from `trakr --example`** (the helper does) and edit it rather than hand-writing the JSON.

## 3. Data model (what matters when composing)

All indices are 0-based. Unless stated, JSON keys are the Rust field names.

**Song**: `name, author, comments, bpm, lpb, tpl, edo, tracks, patterns, sequence, instruments, loop_range, slot_meta, groove`.
- `bpm` beats per minute; `lpb` lines per beat (4 -> a 16-line pattern is one 4/4 bar). `tpl` ticks per line (12).
- `sequence` is the play order: a list of pattern indices. The song plays the sequence once, top to bottom.
- `edo`: equal divisions of the octave (default 12). See *Microtonal tunings* below.
- `loop_range`: `[first_slot, last_slot]` or `null`: the editor loops that part of the sequence (renders ignore it).

**Tracks** (`song.tracks`): sequencer tracks first, then optional group/send tracks, and the `"Master"`
last. A track has `name, kind ("Sequencer"|"Group"|"Send"|"Master"), volume (1.0 = 0 dB), panning (-1..1),
mute ("Active"|"Muted"|"Off"), solo, devices, group, sends, visible_note_columns, visible_effect_columns`.
`visible_note_columns` / `visible_effect_columns` set how many columns the editor shows: raise them to
the number of columns you use so a human opening the file sees everything. Track order is: sequencer tracks, group tracks, **Master**, send tracks (the helper's `add_bus` keeps this). Patterns hold one entry per track in the same order: **if you change the track count, every
pattern's `tracks` list must match** (set tracks first, then create patterns).

**Patterns** (`song.patterns[i]`): `name, number_of_lines (1..512), tracks, automation`.
`tracks[t].lines` maps the line number *as a string* to a line object; empty lines are simply absent.
A line is `{"notes": [12 note columns], "effects": [8 effect columns]}`. Always write the full arrays
(12 and 8 entries), copying a template line.

**Note column** `{note, instrument, volume, panning, delay, micro, effect}`:

| field | values |
|---|---|
| `note` | 0..119 = C-0..B-9, **48 = C-4**, 57 = A-4 (440 Hz). 120 = OFF, 121 = empty |
| `instrument` | 0-based instrument index, **255 = empty** |
| `volume` | 0..128 (0x80 = full volume), **255 = empty**. 0x81..0xF0 are letter commands (below) |
| `panning` | 0..128 (64 = centre, 0 = left, 128 = right), 255 = empty |
| `delay` | 0..255 = fraction of a line (1/256) the note starts late; 0 = none |
| `micro` | -128..127 = extra pitch in 1/128 semitone added to `note` (default 0; used by microtonal tunings, usable in any song for fine detune) |
| `effect` | the small per-column slot: `{"id": [48, 67], "amount": 2}` is `0C02`. Empty = `{"id":[0,0],"amount":0}` |

Note numbers: `note = octave*12 + [C,C#,D,D#,E,F,F#,G,G#,A,A#,B].index(name)`, so C-4 = 48.
An instrument keeps playing its last note until something stops it. A note-off (`OFF`, 120) in the same
column releases it (envelope release); a new note in the same column ends the old one per the
instrument's `nna` (`Cut` | `NoteOff` | `Continue`). **Looped/sustained instruments never end by
themselves: write an `OFF` where the note should stop** (at the end of the song the renderer releases whatever is
still held, but mid-song a missing `OFF` lets the note ring until the column's next note). One-shot drum samples
(`one_shot: true`, no loop) end on their own.

**Effect columns** (`line.effects[c]`) and the per-note slot take two-character ids plus one amount byte:
`{"id": [ord('Z'), ord('T')], "amount": 0x90}` = `ZT90` (tempo 144). List with `trakr --effects`. Useful ones:
`ZT` set BPM, `ZL` set LPB, `ZB` pattern break, `ZJ` jump to slot, `0S` sample offset, `0C` cut after n ticks,
`0Q` delay n ticks, `0R` retrigger every n ticks, `0U/0D` pitch slide, `0G` glide, `0V` vibrato, `0T` tremolo,
`0A` arpeggio (`0A37` = major chord), `0W` volume slide, `0I/0O` fade in/out, `0L` track volume, `0P` track pan,
`0Y` probability, `M1..M4` instrument macros. In an effect column the effect applies to the whole track;
in a note's slot it applies to that note.

**Volume/panning letter commands**: a byte `0x81 + 16*letter + digit` with letters `G U D I O B C`
(glide, slide up, slide down, fade in, fade out, backwards, cut) and digit 0..15. e.g. glide speed 4 =
`0x81 + 0*16 + 4 = 0x85`.

**Instruments** (`song.instruments[i]`): `name, samples, volume, nna, interpolation, volume_env, pan_env,
pitch_env, cutoff_env, filter, lfos, phrases, mute_group, drive, fx, plugin, ...`.
- A sample: `name, sample_rate, channels (1|2), base_note (48 = plays at original pitch on C-4), fine_tune,
  volume, panning, loop_mode ("Off"|"Forward"|"Reverse"|"PingPong"), loop_start, loop_end (frames),
  one_shot, key_range [lo, hi], vel_range [0,127], slices, ...`. Add `key_range` splits for multisamples.
  A sample is played back resampled, so any note pitches it relative to `base_note`.
- Envelope: `{"enabled": true, "points": [[seconds, value 0..1], ...], "sustain": index|null,
  "loop_points": null, "curve": "Linear"}`. Volume envelope with `sustain` holds until note-off, then
  plays on. Pan/pitch envelopes: 0.5 = centre / no change.
- `filter`: `{"mode": "Off"|"LowPass"|"HighPass"|"BandPass"|"Notch", "cutoff": Hz, "resonance": Q}`. `mute_group` N != 0: a note cuts other sounding instruments of the same group (open/closed hats).
- Instruments can also carry their own FX chain (`fx`, list of devices) applied before the track.

**Devices** (`track.devices`): `{"id": "native.filter", "bypass": false, "params": [..], "plugin": null, "text": ""}`.
`params` are plain values in each parameter's own range, **in the order `trakr --devices` prints**; out-of-range
values are clamped by the engine. Always supply the full parameter list (start from the defaults).
Effects run in list order, before the track fader. Typical: filter, EQ, compressor, delay, reverb, limiter
(on the Master track to catch peaks).

**Automation** (`pattern.automation`): `{"track": t, "target": T, "points": [[line, value 0..1]], "curve": "Linear"}`.
`T` is `"TrackVolume"`, `"TrackPan"`, `"Bpm"` (on track 0; 0..1 maps to 32..255 BPM), `"Lpb"`, or
`{"Device": {"device": d, "param": p}}`. Device values are normalized over the parameter's range (log for
frequency-like parameters). Curves: `Linear, Step, Smooth, EaseIn, EaseOut`. Lines may be fractional. A lane
applies while its pattern plays.

**Structure**: `slot_meta` (optional, per sequence slot) holds `mutes` (bitmask of muted tracks), `section`
(name) and `repeat`. Leave it `[]` unless you need it. `groove` is a list of per-line shuffle fractions.

## 3b. Microtonal tunings (any equal division of the octave)

Set `song.edo` (1..240; 12 = ordinary). The pitch of a note cell is always `note + micro/128` semitones from the
instrument's base note, so any pitch is reachable in any song. For EDO tunings the helpers convert a *step number*
(counted from C-0, octave = `edo` steps) to the `(note, micro)` pair the engine plays:

```python
p = Project(edo=51)                       # 51 steps per octave, 23.53 cents each
p.note(pat, 0, 0, step=102, instrument=lead)   # step 102 = two octaves above C-0
```

In the editor, with `edo` != 12 note cells show the step number (`102`) instead of a name, piano keys play consecutive
steps (F1/F2 move by 12 steps), transpose moves one step (octave = `edo` steps), and MIDI keys are one step each.
`step_to_note(step, edo)` in `trakr_song.py` mirrors `trakr_core::step_to_note`; Lua scripts use
`trakr.set_note(p,t,l,c,{step=102})`, `micro=`, and `trakr.set_song{edo=51}`. Chord/scale material in other
tunings: pick scale steps for your EDO (e.g. 19-EDO major scale = steps 0,3,6,8,11,14,17 in a 19-step octave).
Effects that take semitone amounts (`0A` arpeggio, `0U/0D` slides) stay in semitones/16 and are not retuned.

## 4. Composing well within these tools

1. **Tempo and grid.** Pick `bpm` and `lpb` first. At lpb 4 one beat = 4 lines and a 16-line pattern = one bar;
   64 lines = four bars. Use `delay` or `lpb 6/8` for triplets and swing (`groove`).
2. **One idea per track**: drums on their own tracks (kick, snare, hats) so each has its own volume, pan and
   effects. Use extra note columns (up to 12) for chords: put chord notes in columns 0, 1, 2 of the same track.
3. **Pitch.** Compose in note numbers: `scale = [0,2,3,5,7,8,10]` (natural minor) + root. Keep bass in
   octaves 1-2 (C-2 = 24), chords in 3-4, melody in 4-5. Samples are repitched from `base_note`, so a sample
   recorded at C-4 should have `base_note: 48`.
4. **Variation.** Reuse patterns for repetition (`sequence: [0,0,1,0,2,...]`), and make variants (fills,
   different last bar) as new patterns. A song of 8-16 sequence slots with 3-4 distinct patterns reads as music.
5. **Dynamics.** Vary the volume column per hit (accents), and automate filter cutoff / track volume across
   patterns for builds and drops. Use `OFF` and `0C` cuts for staccato.
6. **Mix.** Sum of track volumes should keep the master `peak` below ~0.95 (shown by `--check`). Prefer lowering
   track volumes; add `native.limiter` on the Master track as a safety net. Pan supporting parts slightly
   (-0.3..0.3); keep kick/bass/snare centred.
7. **Samples.** Generate them (see `synth` in `trakr_song.py`: kick, snare, hat, saw/square/sine cycles) or
   decode any audio file to float and write it as an `.f32` sample. For tonal loops, store a whole number of
   cycles and set `loop_start 0`, `loop_end = frames`, `loop_mode "Forward"`, and give the instrument a
   volume envelope with a short release (`add_instrument(..., loop=True)` does this).
8. **Iterate.** After each change run `--check`. Zero or tiny `rms`, or `SILENT`, means nothing played (wrong
   instrument index, volume 0, muted track, empty `sequence`); `Project.validate()` catches most of these before saving. A render much longer than the song means notes ring on: add `OFF`s.

## 5. Pitfalls checklist

- Note 48 is C-4 (not MIDI 60 in the file; it maps to MIDI 60 externally).
- Empty markers: note 121, instrument/volume/panning 255. A cell of zeros is not empty (note 0 = C-0, instrument 0).
- `lines` keys are strings; values need the full 12+8 arrays.
- Track count and every pattern's `tracks` length must agree; order is sequencers, groups, Master, sends. Indices stored in `group`, `sends[].target`, automation `track` and `sidechain` devices refer to this order.
- Each sample needs its `samples/{i}_{s}.f32`; byte length must be a multiple of 4.
- Instruments are referenced by index in the instrument column: reordering them requires rewriting notes.
- Max 512 lines per pattern, 12 note columns, 8 effect columns, 64 tracks in the mixer.
- Loop points are in frames, not bytes or seconds; `loop_end <= frames`.
- Hosted plugins (CLAP) are saved by absolute path and are not loaded by `--render` (they pass audio through).

## 6. Feature coverage: where each capability lives

Everything below is in `trakr --example-full` (read its `song.json` for exact spelling) and can be set through the
helper or raw JSON.

| capability | JSON location | helper |
|---|---|---|
| notes, OFF, volume/pan, sub-line delay, micro pitch | pattern line `notes[c]` | `note(...)`, `hits(...)` |
| volume/pan letter commands (glide, slides, fades, backwards, cut) | `volume`/`panning` bytes 0x81..0xF0 | `col_cmd('G', 4)` with `volume_byte=` |
| per-note and per-track effect commands (`0A` arp, `0V` vibrato, `ZT` tempo, ...) | `notes[c].effect`, `line.effects[c]` | `fx="0A37"`, `effect(...)` |
| tempo / LPB / ticks per line, groove | `bpm lpb tpl groove` | `Project(...)`, `groove(...)`, `ZT/ZL/ZK` |
| pattern length, names, order, repeats, sections, per-slot track mutes, loop range | `patterns sequence slot_meta loop_range` | `add_pattern`, `slot_meta(...)` |
| tunings | `edo`, `micro` | `Project(edo=...)`, `note(step=...)` |
| samples: loops (Forward/Reverse/PingPong), loop release, one-shot, fine tune, slices, beat sync | instrument `samples[]` | `add_sample(...)` |
| multisample key zones, velocity layers | `samples[].key_range / vel_range` | `add_sample(..., key_range=, vel_range=)` |
| envelopes (volume/pan/pitch/cutoff: sustain, loop, curves), fade-out | `volume_env pan_env pitch_env cutoff_env fade_out` | `envelope(...)`, `adsr(...)` |
| filter (LP/HP/BP/Notch), drive, interpolation, NNA, key/velocity tracking | instrument fields | `add_instrument(..., filter=..., nna=...)` |
| LFOs (7 shapes, 6 targets), key/velocity/macro modulation, macros | `lfos mods macros` | `lfo(...)`, `mod(...)` |
| phrases (Keymap / Program), shuffle | `phrases phrase_mode` | `add_phrase(...)` |
| instrument FX chain, mute groups, MIDI out | `fx mute_group midi_out` | extra kwargs of `add_instrument` |
| devices (44 native, modulators, formula text) | `track.devices` | `add_device(track, id, p1=...)`, `trakr --devices` |
| group tracks, send tracks (pre/post fader), sidechain | `group sends` + `native.sidechain` | `add_bus`, `route` |
| track volume, pan, mute/solo, colors, hidden sub-columns, collapse | track fields | `p.track(i)[...] = ...` |
| automation (volume, pan, any device parameter, BPM, LPB; 5 curves) | `pattern.automation` | `automate(...)` |
| audio from files | `.f32` samples | `load_wav(path)` (convert other formats to WAV first) |
| export (WAV/FLAC/Ogg/MP3/AAC/Opus, stems, tags) | | `trakr --render` |
| CLAP plugins | `DeviceSpec.plugin` / `Instrument.plugin` | raw JSON; not rendered headlessly |
| import of other formats | | `trakr --check file.mod|.xm|.s3m|.it|.xrns` |

## 7. Helper API (`docs/agents/trakr_song.py`)

```
Project(bpm, lpb, name, author, trakr="trakr", edo=12, tpl=12)
  .set_tracks(n)                       n sequencer tracks + Master (call first)
  .add_bus("Group"|"Send", name)       returns the track index (call before patterns)
  .route(track, group=None, sends={send_index: level}, pre_fader=False)
  .track(i)                            the raw track dict: name, volume, panning, mute, solo, color, hide_*, collapsed
  .add_instrument(name, data=None, rate, base_note, loop, one_shot, volume, nna, channels, **instrument_fields)
  .add_sample(instrument, data, rate, base_note, loop, one_shot, channels, name, key_range, vel_range,
              loop_mode, loop_start, loop_end, loop_release, fine_tune, volume, panning, slices, beat_sync_lines)
  .add_phrase(instrument, name, lines, rows={line: (note, instrument)}, lpb, key_range, base_note, mode, looping, shuffle)
  .add_pattern(lines=64, name="")      returns the pattern index
  .note(pattern, track, line, note|step=, instrument, volume, panning, column, delay, fx, micro, volume_byte, panning_byte)
  .hits(pattern, track, "x...X...", note, instrument, volume, accent, column)   step-sequencer rows
  .effect(pattern, track, line, "ZT90", column=0)
  .automate(pattern, track, target, [(line, value01)], curve)
  .add_device(track, "native.reverb", p0=..., p3=...)   parameter values by index
  .slot_meta(slot, section, repeat, mutes), .groove(*offsets), .sequence = [...]
  .song                                the raw song dict: set any field the helpers do not cover
  .validate(), .save(path)             validate() runs before every save
synth.kick/snare/hat/saw_cycle/square_cycle/sine_cycle      generated samples
envelope(points, sustain, loop, curve), adsr(a, d, s, r), lfo(...), mod(...), col_cmd(letter, digit)
load_wav(path), step_to_note(step, edo), note_number("C-4"), vol(x), pan(x)
check(trakr, path, wav=None)         run --check or --render and return the text output
```

Anything the helper lacks is plain JSON in `p.song`; mirror the field spelling from `trakr --example-full`.

## 8. Alternatives

- **Lua scripting inside the app** (Scripts menu) edits a song copy with an API for patterns, notes (including
  `micro`/`step`), tracks, devices, samples and automation: reference in `crates/trakr-script/scripts/API.txt`.
  Good for generating or transforming material once a project is open.
- **Other formats**: `trakr --check` loads `.xrns`, `.mod`, `.s3m`, `.xm`, `.it`, so a tracker module
  can be rendered or used as a starting point.
