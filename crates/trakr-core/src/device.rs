//! Native DSP device catalogue: ids, parameter ranges and defaults. The engine implements the
//! processing; this table lets every frontend build parameter UIs and lets patterns automate them.

use crate::track::DeviceSpec;

pub struct ParamInfo {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    /// Sliders and automation map this parameter logarithmically (frequencies, times).
    pub log: bool,
    /// Whole-number parameter (modes, switches).
    pub stepped: bool,
}

impl ParamInfo {
    /// Position of `v` within the range, 0..=1, following the slider mapping (log for frequencies and times).
    pub fn to_norm(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        let x = if self.log && self.min > 0.0 {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else if self.max > self.min {
            (v - self.min) / (self.max - self.min)
        } else {
            0.0
        };
        x.clamp(0.0, 1.0)
    }

    /// Inverse of [`to_norm`](Self::to_norm); whole-number parameters are rounded.
    pub fn from_norm(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let v = if self.log && self.min > 0.0 {
            self.min * (self.max / self.min).powf(x)
        } else {
            self.min + (self.max - self.min) * x
        };
        if self.stepped {
            v.round().clamp(self.min, self.max)
        } else {
            v.clamp(self.min, self.max)
        }
    }
}

pub struct DeviceInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub params: &'static [ParamInfo],
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamInfo {
    ParamInfo { name, min, max, default, unit, log: false, stepped: false }
}

const fn plog(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamInfo {
    ParamInfo { name, min, max, default, unit, log: true, stepped: false }
}

const fn pstep(name: &'static str, min: f32, max: f32, default: f32) -> ParamInfo {
    ParamInfo { name, min, max, default, unit: "", log: false, stepped: true }
}

/// Ids of devices that modulate another device's parameter instead of (only) shaping audio.
pub const MODULATORS: [&str; 7] = [
    "native.lfo",
    "native.follower",
    "native.hydra",
    "native.keytracker",
    "native.veltracker",
    "native.metamixer",
    "native.formula",
];

pub const FILTER_MODES: [&str; 4] = ["Low-pass", "High-pass", "Band-pass", "Notch"];

pub const DEVICES: &[DeviceInfo] = &[
    DeviceInfo {
        id: "native.gainer",
        name: "Gainer",
        params: &[p("Gain", -60.0, 12.0, 0.0, "dB"), p("Pan", -1.0, 1.0, 0.0, "")],
    },
    DeviceInfo {
        id: "native.filter",
        name: "Filter",
        params: &[
            pstep("Mode (0 LP, 1 HP, 2 BP, 3 Notch)", 0.0, 3.0, 0.0),
            plog("Cutoff", 20.0, 20_000.0, 8000.0, "Hz"),
            plog("Resonance (Q)", 0.1, 10.0, 0.707, ""),
        ],
    },
    DeviceInfo {
        id: "native.eq",
        name: "3-band EQ",
        params: &[
            p("Low gain", -18.0, 18.0, 0.0, "dB"),
            plog("Low freq", 20.0, 500.0, 100.0, "Hz"),
            p("Mid gain", -18.0, 18.0, 0.0, "dB"),
            plog("Mid freq", 200.0, 8000.0, 1000.0, "Hz"),
            plog("Mid Q", 0.2, 8.0, 1.0, ""),
            p("High gain", -18.0, 18.0, 0.0, "dB"),
            plog("High freq", 2000.0, 16_000.0, 6000.0, "Hz"),
        ],
    },
    DeviceInfo {
        id: "native.delay",
        name: "Delay",
        params: &[
            plog("Time", 1.0, 2000.0, 300.0, "ms"),
            p("Feedback", 0.0, 0.95, 0.4, ""),
            p("Mix", 0.0, 1.0, 0.3, ""),
            p("Ping-pong", 0.0, 1.0, 0.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.reverb",
        name: "Reverb",
        params: &[
            p("Size", 0.0, 1.0, 0.6, ""),
            p("Damping", 0.0, 1.0, 0.4, ""),
            p("Width", 0.0, 1.0, 1.0, ""),
            p("Mix", 0.0, 1.0, 0.25, ""),
        ],
    },
    DeviceInfo {
        id: "native.chorus",
        name: "Chorus / flanger",
        params: &[
            plog("Rate", 0.05, 5.0, 0.8, "Hz"),
            p("Depth", 0.1, 10.0, 3.0, "ms"),
            p("Mix", 0.0, 1.0, 0.5, ""),
            p("Feedback", 0.0, 0.9, 0.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.distortion",
        name: "Distortion",
        params: &[
            p("Drive", 0.0, 40.0, 12.0, "dB"),
            p("Mix", 0.0, 1.0, 1.0, ""),
            p("Output", -24.0, 6.0, -6.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.compressor",
        name: "Compressor",
        params: &[
            p("Threshold", -60.0, 0.0, -18.0, "dB"),
            plog("Ratio", 1.0, 20.0, 4.0, ":1"),
            plog("Attack", 0.1, 200.0, 10.0, "ms"),
            plog("Release", 5.0, 2000.0, 120.0, "ms"),
            p("Makeup", 0.0, 24.0, 0.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.eq10",
        name: "10-band EQ",
        params: &[
            p("31 Hz", -18.0, 18.0, 0.0, "dB"),
            p("62 Hz", -18.0, 18.0, 0.0, "dB"),
            p("125 Hz", -18.0, 18.0, 0.0, "dB"),
            p("250 Hz", -18.0, 18.0, 0.0, "dB"),
            p("500 Hz", -18.0, 18.0, 0.0, "dB"),
            p("1 kHz", -18.0, 18.0, 0.0, "dB"),
            p("2 kHz", -18.0, 18.0, 0.0, "dB"),
            p("4 kHz", -18.0, 18.0, 0.0, "dB"),
            p("8 kHz", -18.0, 18.0, 0.0, "dB"),
            p("16 kHz", -18.0, 18.0, 0.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.ladder",
        name: "Ladder filter (24 dB)",
        params: &[
            plog("Cutoff", 20.0, 20_000.0, 2000.0, "Hz"),
            p("Resonance", 0.0, 1.0, 0.3, ""),
            p("Drive", 0.0, 24.0, 0.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.gate",
        name: "Gate",
        params: &[
            p("Threshold", -80.0, 0.0, -50.0, "dB"),
            plog("Attack", 0.1, 50.0, 1.0, "ms"),
            plog("Release", 5.0, 1000.0, 100.0, "ms"),
            p("Range", 0.0, 80.0, 60.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.limiter",
        name: "Limiter",
        params: &[
            p("Input gain", 0.0, 24.0, 0.0, "dB"),
            p("Ceiling", -24.0, 0.0, -1.0, "dB"),
            plog("Release", 10.0, 1000.0, 80.0, "ms"),
        ],
    },
    DeviceInfo {
        id: "native.phaser",
        name: "Phaser",
        params: &[
            plog("Rate", 0.05, 5.0, 0.4, "Hz"),
            p("Depth", 0.0, 1.0, 0.7, ""),
            p("Feedback", -0.9, 0.9, 0.5, ""),
            p("Mix", 0.0, 1.0, 0.5, ""),
        ],
    },
    DeviceInfo {
        id: "native.ringmod",
        name: "Ring modulator",
        params: &[plog("Frequency", 1.0, 5000.0, 440.0, "Hz"), p("Mix", 0.0, 1.0, 1.0, "")],
    },
    DeviceInfo {
        id: "native.bitcrusher",
        name: "Bit crusher",
        params: &[
            pstep("Bits", 1.0, 16.0, 8.0),
            plog("Rate reduction", 1.0, 64.0, 1.0, "x"),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo { id: "native.width", name: "Stereo width", params: &[p("Width", 0.0, 2.0, 1.0, "")] },
    DeviceInfo {
        id: "native.autopan",
        name: "Tremolo / auto-pan",
        params: &[
            plog("Rate", 0.05, 20.0, 2.0, "Hz"),
            p("Depth", 0.0, 1.0, 0.5, ""),
            pstep("Mode (0 tremolo, 1 auto-pan)", 0.0, 1.0, 0.0),
        ],
    },
    DeviceInfo {
        id: "native.lfo",
        name: "LFO (modulator)",
        params: &[
            pstep("Target device (0 off, 1-8 chain slot)", 0.0, 8.0, 0.0),
            pstep("Target parameter (0 = first)", 0.0, 15.0, 0.0),
            pstep("Shape (0 sine, 1 tri, 2 saw, 3 square, 4 random)", 0.0, 4.0, 0.0),
            plog("Rate", 0.05, 20.0, 1.0, "Hz"),
            p("Depth", -1.0, 1.0, 0.5, ""),
            p("Phase", 0.0, 1.0, 0.0, ""),
            pstep("Sync (0 off/Hz, 1: 4 beats ... 6: 1/32, 7-8: triplets)", 0.0, 8.0, 0.0),
        ],
    },
    DeviceInfo {
        id: "native.follower",
        name: "Envelope follower (modulator)",
        params: &[
            pstep("Target device (0 off, 1-8 chain slot)", 0.0, 8.0, 0.0),
            pstep("Target parameter (0 = first)", 0.0, 15.0, 0.0),
            plog("Attack", 1.0, 500.0, 10.0, "ms"),
            plog("Release", 5.0, 2000.0, 150.0, "ms"),
            p("Sensitivity", 0.1, 16.0, 4.0, ""),
            p("Depth", -1.0, 1.0, 0.5, ""),
        ],
    },
    DeviceInfo {
        id: "native.sidechain",
        name: "Sidechain compressor",
        params: &[
            pstep("Source track (0 = own signal)", 0.0, 64.0, 0.0),
            p("Threshold", -60.0, 0.0, -24.0, "dB"),
            plog("Ratio", 1.0, 20.0, 4.0, ":1"),
            plog("Attack", 0.1, 200.0, 5.0, "ms"),
            plog("Release", 5.0, 2000.0, 150.0, "ms"),
            p("Makeup", 0.0, 24.0, 0.0, "dB"),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.buscomp",
        name: "Bus compressor",
        params: &[
            p("Threshold", -40.0, 0.0, -18.0, "dB"),
            plog("Ratio", 1.0, 10.0, 2.0, ":1"),
            plog("Attack", 0.1, 100.0, 10.0, "ms"),
            plog("Release", 20.0, 1200.0, 200.0, "ms"),
            p("Knee", 0.0, 24.0, 6.0, "dB"),
            p("Makeup", 0.0, 24.0, 0.0, "dB"),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.multiband",
        name: "Multiband compressor",
        params: &[
            plog("Crossover 1", 40.0, 2000.0, 200.0, "Hz"),
            plog("Crossover 2", 400.0, 16_000.0, 2500.0, "Hz"),
            p("Low threshold", -60.0, 0.0, -24.0, "dB"),
            p("Mid threshold", -60.0, 0.0, -24.0, "dB"),
            p("High threshold", -60.0, 0.0, -24.0, "dB"),
            plog("Ratio", 1.0, 20.0, 3.0, ":1"),
            plog("Attack", 0.1, 200.0, 10.0, "ms"),
            plog("Release", 5.0, 2000.0, 150.0, "ms"),
            p("Makeup", 0.0, 24.0, 0.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.multitap",
        name: "Multitap delay",
        params: &[
            plog("Tap 1 time", 10.0, 1500.0, 150.0, "ms"),
            plog("Tap 2 time", 10.0, 1500.0, 300.0, "ms"),
            plog("Tap 3 time", 10.0, 1500.0, 450.0, "ms"),
            p("Tap 1 level", 0.0, 1.0, 0.6, ""),
            p("Tap 2 level", 0.0, 1.0, 0.4, ""),
            p("Tap 3 level", 0.0, 1.0, 0.25, ""),
            p("Feedback", 0.0, 0.85, 0.2, ""),
            p("Mix", 0.0, 1.0, 0.4, ""),
        ],
    },
    DeviceInfo {
        id: "native.repeater",
        name: "Repeater",
        params: &[
            plog("Slice length", 5.0, 1000.0, 250.0, "ms"),
            pstep("Hold (automate this)", 0.0, 1.0, 0.0),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.cabinet",
        name: "Cabinet",
        params: &[
            pstep("Type (0 combo, 1 stack, 2 small, 3 radio)", 0.0, 3.0, 0.0),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.dcoffset",
        name: "DC offset",
        params: &[p("Offset", -1.0, 1.0, 0.0, ""), pstep("Remove DC first", 0.0, 1.0, 0.0)],
    },
    DeviceInfo {
        id: "native.hydra",
        name: "Hydra (modulator)",
        params: &[
            pstep("Shape (0 sine, 1 tri, 2 saw, 3 square, 4 random)", 0.0, 4.0, 0.0),
            plog("Rate", 0.05, 20.0, 1.0, "Hz"),
            pstep("Target 1 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target 1 parameter", 0.0, 15.0, 0.0),
            p("Target 1 depth", -1.0, 1.0, 0.5, ""),
            pstep("Target 2 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target 2 parameter", 0.0, 15.0, 0.0),
            p("Target 2 depth", -1.0, 1.0, 0.5, ""),
            pstep("Target 3 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target 3 parameter", 0.0, 15.0, 0.0),
            p("Target 3 depth", -1.0, 1.0, 0.5, ""),
        ],
    },
    DeviceInfo {
        id: "native.maximizer",
        name: "Maximizer",
        params: &[
            p("Gain", 0.0, 24.0, 6.0, "dB"),
            p("Ceiling", -12.0, 0.0, -0.3, "dB"),
            plog("Release", 10.0, 500.0, 100.0, "ms"),
            p("Soft clip", 0.0, 1.0, 0.3, ""),
        ],
    },
    DeviceInfo {
        id: "native.analogfilter",
        name: "Analog filter",
        params: &[
            pstep("Mode (0 low-pass, 1 band-pass, 2 high-pass, 3 notch)", 0.0, 3.0, 0.0),
            plog("Cutoff", 20.0, 20_000.0, 1000.0, "Hz"),
            p("Resonance", 0.0, 1.0, 0.3, ""),
            p("Drive", 0.0, 36.0, 0.0, "dB"),
            p("Mix", 0.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.midside",
        name: "Mid / side",
        params: &[
            p("Mid gain", -48.0, 18.0, 0.0, "dB"),
            p("Side gain", -48.0, 18.0, 0.0, "dB"),
            p("Bass mono below (0 = off)", 0.0, 500.0, 0.0, "Hz"),
        ],
    },
    DeviceInfo {
        id: "native.convolver",
        name: "Convolver",
        params: &[
            pstep("Response: instrument", 0.0, 254.0, 0.0),
            pstep("Response: sample", 0.0, 15.0, 0.0),
            p("Mix", 0.0, 1.0, 0.3, ""),
            p("Gain", -48.0, 18.0, 0.0, "dB"),
        ],
    },
    DeviceInfo {
        id: "native.keytracker",
        name: "Key tracker (modulator)",
        params: &[
            pstep("Target device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target parameter", 0.0, 15.0, 0.0),
            p("Depth", -1.0, 1.0, 1.0, ""),
            pstep("Lowest note", 0.0, 119.0, 24.0),
            pstep("Highest note", 0.0, 119.0, 96.0),
            pstep("Bipolar (-1..1)", 0.0, 1.0, 0.0),
        ],
    },
    DeviceInfo {
        id: "native.veltracker",
        name: "Velocity tracker (modulator)",
        params: &[
            pstep("Target device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target parameter", 0.0, 15.0, 0.0),
            p("Depth", -1.0, 1.0, 1.0, ""),
            pstep("Bipolar (-1..1)", 0.0, 1.0, 0.0),
        ],
    },
    DeviceInfo {
        id: "native.metamixer",
        name: "Meta mixer (modulator)",
        params: &[
            pstep("Knob 1 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Knob 1 parameter", 0.0, 15.0, 0.0),
            p("Knob 1", -1.0, 1.0, 0.0, ""),
            pstep("Knob 2 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Knob 2 parameter", 0.0, 15.0, 0.0),
            p("Knob 2", -1.0, 1.0, 0.0, ""),
            pstep("Knob 3 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Knob 3 parameter", 0.0, 15.0, 0.0),
            p("Knob 3", -1.0, 1.0, 0.0, ""),
            pstep("Knob 4 device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Knob 4 parameter", 0.0, 15.0, 0.0),
            p("Knob 4", -1.0, 1.0, 0.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.formula",
        name: "Formula (modulator)",
        params: &[
            pstep("Target device (0 off, 1-8 slot)", 0.0, 8.0, 0.0),
            pstep("Target parameter", 0.0, 15.0, 0.0),
            p("A", 0.0, 1.0, 0.0, ""),
            p("B", 0.0, 1.0, 0.0, ""),
            p("C", 0.0, 1.0, 0.0, ""),
            p("D", 0.0, 1.0, 0.0, ""),
            p("Depth", -1.0, 1.0, 1.0, ""),
        ],
    },
    DeviceInfo {
        id: "native.send",
        name: "Send (#Send)",
        params: &[
            pstep("Send track number", 1.0, 64.0, 1.0),
            p("Level", -60.0, 6.0, 0.0, "dB"),
            pstep("Mute the signal after the send", 0.0, 1.0, 0.0),
        ],
    },
    DeviceInfo {
        id: "native.exciter",
        name: "Exciter",
        params: &[plog("Frequency", 1000.0, 10_000.0, 3000.0, "Hz"), p("Amount", 0.0, 1.0, 0.3, "")],
    },
];

pub fn device_info(id: &str) -> Option<&'static DeviceInfo> {
    DEVICES.iter().find(|d| d.id == id)
}

impl DeviceInfo {
    pub fn defaults(&self) -> Vec<f32> {
        self.params.iter().map(|p| p.default).collect()
    }
}

impl ParamInfo {
    /// Map a normalized 0..=1 position to the parameter's range (respecting log scaling and steps).
    pub fn denormalize(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let v = if self.log && self.min > 0.0 {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + (self.max - self.min) * t
        };
        if self.stepped {
            v.round()
        } else {
            v
        }
    }

    /// Inverse of [`denormalize`](Self::denormalize).
    pub fn normalize(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        if self.log && self.min > 0.0 {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min).max(1e-9)
        }
    }
}

impl DeviceSpec {
    /// A device with default parameters, or `None` for an unknown id.
    pub fn new(id: &str) -> Option<Self> {
        let info = device_info(id)?;
        Some(Self {
            id: info.id.to_string(),
            bypass: false,
            params: info.defaults(),
            plugin: None,
            text: String::new(),
        })
    }

    /// A device that runs a hosted plugin, with the plugin's default parameter values.
    pub fn from_plugin(plugin: crate::plugin::PluginRef) -> Self {
        let params = plugin.params.iter().map(|p| p.default).collect();
        Self {
            id: format!("{}{}", crate::plugin::PLUGIN_ID_PREFIX, plugin.plugin_id),
            bypass: false,
            params,
            plugin: Some(plugin),
            text: String::new(),
        }
    }

    /// Human-readable name: the catalogue name, or the plugin's name.
    pub fn display_name(&self) -> String {
        match (&self.plugin, device_info(&self.id)) {
            (Some(p), _) => p.name.clone(),
            (None, Some(i)) => i.name.to_string(),
            (None, None) => self.id.clone(),
        }
    }

    /// Number of parameters this device exposes.
    pub fn param_count(&self) -> usize {
        match (&self.plugin, device_info(&self.id)) {
            (Some(p), _) => p.params.len(),
            (None, Some(i)) => i.params.len(),
            (None, None) => 0,
        }
    }

    /// Parameter `i` at normalized position `t` (0..=1), for automation.
    pub fn denormalize_param(&self, i: usize, t: f32) -> Option<f32> {
        if let Some(p) = &self.plugin {
            return p.params.get(i).map(|p| p.denormalize(t));
        }
        device_info(&self.id)?.params.get(i).map(|p| p.denormalize(t))
    }

    /// (name, min, max, default, logarithmic, stepped, unit) of parameter `i`.
    pub fn param_meta(&self, i: usize) -> Option<(String, f32, f32, f32, bool, bool, &'static str)> {
        if let Some(p) = &self.plugin {
            let p = p.params.get(i)?;
            return Some((p.name.clone(), p.min, p.max, p.default, false, p.stepped, ""));
        }
        let p = device_info(&self.id)?.params.get(i)?;
        Some((p.name.to_string(), p.min, p.max, p.default, p.log, p.stepped, p.unit))
    }

    /// Parameter `i`, falling back to the catalogue default (so short or old parameter lists stay valid).
    pub fn param(&self, i: usize) -> f32 {
        self.params.get(i).copied().or_else(|| self.param_meta(i).map(|m| m.3)).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_is_consistent() {
        for d in DEVICES {
            assert!(!d.params.is_empty());
            for p in d.params {
                assert!(p.min < p.max && p.default >= p.min && p.default <= p.max, "{}: {}", d.id, p.name);
                assert!(
                    (p.denormalize(p.normalize(p.default)) - p.default).abs()
                        <= p.default.abs() * 1e-3 + 1e-3
                );
            }
            assert_eq!(DeviceSpec::new(d.id).unwrap().params.len(), d.params.len());
        }
        assert!(DeviceSpec::new("native.nothing").is_none());
    }

    #[test]
    fn log_mapping() {
        let p = &device_info("native.filter").unwrap().params[1];
        assert!((p.denormalize(0.0) - 20.0).abs() < 1e-3);
        assert!((p.denormalize(1.0) - 20_000.0).abs() < 1.0);
        assert!((p.denormalize(0.5) - 632.4).abs() < 1.0);
    }
}

#[cfg(test)]
mod norm_tests {
    use super::*;

    #[test]
    fn normalised_positions_roundtrip_for_every_parameter() {
        for d in DEVICES {
            for i in d.params {
                for x in [0.0f32, 0.25, 0.5, 1.0] {
                    let v = i.from_norm(x);
                    assert!(v >= i.min && v <= i.max, "{} {}", d.id, i.name);
                    if !i.stepped {
                        assert!((i.to_norm(v) - x).abs() < 1e-4, "{} {}: {x} -> {v}", d.id, i.name);
                    }
                }
            }
        }
    }
}
