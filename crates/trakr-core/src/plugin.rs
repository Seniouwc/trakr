//! Descriptions of hosted plugins. The song stores which plugin a device is and the parameter ranges
//! it exposed when it was added, so automation, sliders and saved projects work without loading it.

use serde::{Deserialize, Serialize};

/// Parameters kept per hosted plugin (more are ignored).
pub const MAX_PLUGIN_PARAMS: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginParam {
    /// The plugin's own parameter id.
    pub id: u32,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    #[serde(default)]
    pub stepped: bool,
}

impl PluginParam {
    /// `v` limited to the parameter's range. Damaged ranges (NaN, infinite) and values fall back to the
    /// default, or 0, instead of panicking.
    pub fn clamp(&self, v: f32) -> f32 {
        if !(self.min.is_finite() && self.max.is_finite()) {
            return if self.default.is_finite() { self.default } else { 0.0 };
        }
        let (lo, hi) = (self.min.min(self.max), self.min.max(self.max));
        if v.is_finite() {
            v.clamp(lo, hi)
        } else if self.default.is_finite() {
            self.default.clamp(lo, hi)
        } else {
            lo
        }
    }

    /// Map a normalized 0..=1 position to the parameter's range.
    pub fn denormalize(&self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        let v = self.min + (self.max - self.min) * t;
        self.clamp(if self.stepped { v.round() } else { v })
    }

    pub fn normalize(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if !span.is_finite() || span.abs() < 1e-9 {
            0.0
        } else {
            ((self.clamp(v) - self.min) / span).clamp(0.0, 1.0)
        }
    }
}

/// Which plugin a device runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRef {
    /// Path of the plugin file (a `.clap` shared library).
    pub path: String,
    /// The plugin's id inside that file.
    pub plugin_id: String,
    pub name: String,
    pub params: Vec<PluginParam>,
}

/// A plugin found by scanning, before it is loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    /// True for instrument plugins (they make sound from notes instead of processing audio).
    pub instrument: bool,
    pub path: String,
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
}

/// Device ids of hosted plugins start with this.
pub const PLUGIN_ID_PREFIX: &str = "clap:";

#[cfg(test)]
mod tests {
    use super::*;

    fn p(min: f32, max: f32, default: f32, stepped: bool) -> PluginParam {
        PluginParam { id: 0, name: "x".into(), min, max, default, stepped }
    }

    #[test]
    fn ranges_clamp_and_map() {
        let a = p(-2.0, 2.0, 0.0, false);
        assert_eq!(a.clamp(5.0), 2.0);
        assert_eq!(a.denormalize(0.25), -1.0);
        assert_eq!(a.normalize(0.0), 0.5);
        let s = p(0.0, 4.0, 0.0, true);
        assert_eq!(s.denormalize(0.30), 1.0);
    }

    #[test]
    fn damaged_descriptions_never_panic() {
        for min in [f32::NAN, f32::INFINITY, 1.0, 5.0] {
            for max in [f32::NAN, f32::NEG_INFINITY, 0.0, 3.0, 1e30] {
                for d in [f32::NAN, 0.5, 1e9] {
                    let q = p(min, max, d, true);
                    for v in [f32::NAN, f32::INFINITY, -1e30, 0.5, 2.0] {
                        assert!(q.clamp(v).is_finite());
                        let _ = q.denormalize(v);
                        let _ = q.normalize(v);
                    }
                }
            }
        }
    }
}

/// Largest plugin state kept in a song (bytes before encoding).
pub const MAX_PLUGIN_STATE: usize = 4 << 20;

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Plugin state bytes as text (standard base64), for storing in a song's JSON.
pub fn encode_state(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Inverse of [`encode_state`]; text that is not valid base64 (or is too long) gives no bytes.
pub fn decode_state(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in text.bytes() {
        if ch == b'=' || ch.is_ascii_whitespace() {
            continue;
        }
        let Some(v) = B64.iter().position(|&b| b == ch) else { return Vec::new() };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
        if out.len() > MAX_PLUGIN_STATE {
            return Vec::new();
        }
    }
    out
}

#[cfg(test)]
mod state_tests {
    use super::*;

    #[test]
    fn state_text_roundtrips() {
        for n in 0..40usize {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode_state(&encode_state(&bytes)), bytes, "length {n}");
        }
        assert_eq!(encode_state(b"Man"), "TWFu");
        assert_eq!(encode_state(b"Ma"), "TWE=");
        assert!(decode_state("not base64!!").is_empty());
        assert!(decode_state("").is_empty());
    }
}
