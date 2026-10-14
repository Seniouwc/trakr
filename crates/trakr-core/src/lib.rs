//! Data model for trakr: songs, patterns, tracks, instruments.
//!
//! This crate is pure data plus small helpers. No audio, no IO, no UI.

pub mod instrument;
pub mod note;
pub mod pattern;
pub mod song;
pub mod track;

pub use instrument::{Instrument, LoopMode, Sample};
pub use note::{EffectCmd, NoteColumn, EMPTY, NOTE_EMPTY, NOTE_OFF};
pub use pattern::{Line, Pattern, PatternTrack, MAX_EFFECT_COLUMNS, MAX_LINES, MAX_NOTE_COLUMNS};
pub use song::Song;
pub use track::{DeviceSpec, MuteState, Track, TrackKind};
