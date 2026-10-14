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
