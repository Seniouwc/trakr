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
    /// Set for hosted plugins (`id` starts with `clap:`).
    #[serde(default)]
    pub plugin: Option<crate::plugin::PluginRef>,
    /// Free text a device needs (the formula modulator's expression).
    #[serde(default)]
    pub text: String,
}

/// A feed from a track into a send track.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendLevel {
    /// Index of the send track in `Song::tracks`.
    pub target: usize,
    /// Linear gain applied to the feed.
    pub level: f32,
    /// Tap before the track fader (volume and pan) so the send level is independent of it.
    #[serde(default)]
    pub pre_fader: bool,
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
    /// When any track is soloed, only soloed tracks (and the master) are heard.
    #[serde(default)]
    pub solo: bool,
    pub color: Option<[u8; 3]>,
    /// Hide the volume, panning or delay sub-column of this track's note columns in the pattern editor.
    #[serde(default)]
    pub hide_volume: bool,
    #[serde(default)]
    pub hide_panning: bool,
    #[serde(default)]
    pub hide_delay: bool,
    /// Show only the first column, without sub-columns, in the pattern editor.
    #[serde(default)]
    pub collapsed: bool,
    pub devices: Vec<DeviceSpec>,
    /// Index of the owning group track, if any.
    pub group: Option<usize>,
    /// Feeds into send tracks (after the device chain).
    #[serde(default)]
    pub sends: Vec<SendLevel>,
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
            solo: false,
            color: None,
            hide_volume: false,
            hide_panning: false,
            hide_delay: false,
            collapsed: false,
            devices: Vec::new(),
            group: None,
            sends: Vec::new(),
        }
    }
}
