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
        Some(Self { id: info.id.to_string(), bypass: false, params: info.defaults() })
    }

    /// Parameter `i`, falling back to the catalogue default (so short or old parameter lists stay valid).
    pub fn param(&self, i: usize) -> f32 {
        self.params
            .get(i)
            .copied()
            .or_else(|| device_info(&self.id).and_then(|d| d.params.get(i)).map(|p| p.default))
            .unwrap_or(0.0)
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
