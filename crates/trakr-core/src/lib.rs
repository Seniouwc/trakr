//! Data model for trakr: songs, patterns, tracks, instruments.
//!
//! This crate is pure data plus small helpers. No audio, no IO, no UI.

pub mod automation;
pub mod device;
pub mod effects;
pub mod envelope;
pub mod fft;
pub mod instrument;
pub mod note;
pub mod pattern;
pub mod phrase;
pub mod plugin;
pub mod sample_fx;
pub mod song;
pub mod track;

pub use automation::{AutoTarget, Automation};
pub use device::{device_info, DeviceInfo, ParamInfo, DEVICES, MODULATORS};
pub use envelope::Envelope;
pub use instrument::{
    FilterMode, InstLfo, Instrument, InstrumentFilter, Interpolation, LfoShape, LfoTarget, LoopMode, MidiOut,
    Nna, Sample, MAX_INST_LFOS,
};
pub use note::{EffectCmd, NoteColumn, EMPTY, NOTE_EMPTY, NOTE_OFF};
pub use pattern::{Line, Pattern, PatternTrack, MAX_EFFECT_COLUMNS, MAX_LINES, MAX_NOTE_COLUMNS};
pub use phrase::{Phrase, PhraseMode};
pub use plugin::{PluginInfo, PluginParam, PluginRef, MAX_PLUGIN_PARAMS, PLUGIN_ID_PREFIX};
pub use song::{SlotMeta, Song};
pub use track::{DeviceSpec, MuteState, SendLevel, Track, TrackKind};
