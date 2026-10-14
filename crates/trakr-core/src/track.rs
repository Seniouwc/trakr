use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Sequencer,
    Group,
    Send,
    Master,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MuteState {
    #[default]
    Active,
    Muted,
    Off,
}

/// A device in a track's DSP chain. Placeholder until the device system exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceSpec {
    /// Stable device identifier, e.g. `"native.gainer"` or `"clap:com.vendor.plugin"`.
    pub id: String,
    pub bypass: bool,
    pub params: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub visible_note_columns: u8,
    pub visible_effect_columns: u8,
    /// Linear gain, 1.0 = 0 dB.
    pub volume: f32,
    /// -1.0 (left) .. 1.0 (right).
    pub panning: f32,
    pub mute: MuteState,
    pub color: Option<[u8; 3]>,
    pub devices: Vec<DeviceSpec>,
    /// Index of the owning group track, if any.
    pub group: Option<usize>,
}

impl Track {
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            name: name.into(),
            kind,
            visible_note_columns: 1,
            visible_effect_columns: 1,
            volume: 1.0,
            panning: 0.0,
            mute: MuteState::Active,
            color: None,
            devices: Vec::new(),
            group: None,
        }
    }
}
