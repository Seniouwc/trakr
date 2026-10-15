# Roadmap

Each phase should leave `cargo test --workspace` green and be usable on its own.

0. **Scaffold** (done): model, engine with sample voices, GUI grid and note entry, `.trkr` roundtrip, minimal `.xrns` import.
1. **Editor core**: edit/undo command layer, selection, clipboard, block ops (transpose, interpolate, randomize), sequence editor (insert/clone/delete/reorder), pattern length, track add/remove/rename/colors, column visibility, follow-playback.
2. **Samples and instruments**: WAV/FLAC/OGG/MP3 import (symphonia), sample editor (loops, slices, normalize, fades), key/velocity zones, instrument settings, fine-tune, interpolation modes.
3. **Sequencer accuracy**: tick timing, delay column, volume/pan column commands, the full effect command set, NNA modes, song loop range, pattern/line commands (break, jump, delay).
4. **Mixer and DSP**: track DSP chains, native device set, sends, groups, master, meters, mute/solo, pre/post routing, latency compensation.
5. **Modulation**: instrument volume/pan/pitch/cutoff envelopes, LFOs, filters, macros; phrases.
6. **Automation**: pattern-level automation envelopes for any device parameter, recording, interpolation modes, LFO/device-based modulation.
7. **Plugin hosting**: CLAP first, then VST3, LV2, LADSPA; parameter automation, state save/load, preferably sandboxed in a separate process.
8. **IO and control**: MIDI in/out, MIDI mapping, OSC server, Lua scripting API (mlua), JACK/PipeWire native backends.
9. **Render and export**: offline render to WAV/FLAC/OGG/MP3, stems per track, selection render, sample-accurate loops.
10. **Compatibility**: complete XRNS import and export, XRNI/XRNT, MOD/XM/IT/S3M import.
11. **Polish**: themes, configurable keymap, mouse support, help system, performance passes. Optional GUI frontend on the same core/engine.
