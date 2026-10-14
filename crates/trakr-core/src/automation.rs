//! Pattern automation: breakpoint curves that drive a track or device parameter while a pattern plays.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoTarget {
    TrackVolume,
    TrackPan,
    /// Parameter `param` of the track's `device`-th device.
    Device {
        device: usize,
        param: usize,
    },
}

/// Points are `(line position, value 0..=1)`; positions may be fractional, ascending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub track: usize,
    pub target: AutoTarget,
    pub points: Vec<(f32, f32)>,
}

impl Automation {
    pub fn new(track: usize, target: AutoTarget) -> Self {
        Self { track, target, points: Vec::new() }
    }

    /// Value (0..=1) at fractional line position `pos`; holds the first/last value outside the curve.
    pub fn value_at(&self, pos: f32) -> Option<f32> {
        let p = &self.points;
        match p.len() {
            0 => None,
            1 => Some(p[0].1),
            _ => {
                if pos <= p[0].0 {
                    return Some(p[0].1);
                }
                for w in p.windows(2) {
                    if pos <= w[1].0 {
                        let span = (w[1].0 - w[0].0).max(1e-6);
                        return Some(w[0].1 + (w[1].1 - w[0].1) * ((pos - w[0].0) / span));
                    }
                }
                Some(p[p.len() - 1].1)
            }
        }
    }

    /// Insert or replace a point at `line`, keeping order; returns its index.
    pub fn set_point(&mut self, line: f32, value: f32) -> usize {
        let line = line.max(0.0);
        let value = value.clamp(0.0, 1.0);
        if let Some(i) = self.points.iter().position(|p| (p.0 - line).abs() < 1e-3) {
            self.points[i].1 = value;
            return i;
        }
        let i = self.points.iter().position(|p| p.0 > line).unwrap_or(self.points.len());
        self.points.insert(i, (line, value));
        i
    }

    pub fn remove_point(&mut self, i: usize) {
        if i < self.points.len() {
            self.points.remove(i);
        }
    }

    pub fn move_point(&mut self, i: usize, line: f32, value: f32) {
        let lo = if i == 0 { 0.0 } else { self.points[i - 1].0 };
        let hi = self.points.get(i + 1).map_or(f32::MAX, |p| p.0);
        if let Some(p) = self.points.get_mut(i) {
            *p = (line.clamp(lo, hi), value.clamp(0.0, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_holds() {
        let mut a = Automation::new(0, AutoTarget::TrackVolume);
        assert_eq!(a.value_at(3.0), None);
        a.set_point(4.0, 0.0);
        a.set_point(12.0, 1.0);
        assert_eq!(a.value_at(0.0), Some(0.0));
        assert_eq!(a.value_at(8.0), Some(0.5));
        assert_eq!(a.value_at(40.0), Some(1.0));
        assert_eq!(a.set_point(8.0, 0.25), 1);
        assert_eq!(a.set_point(8.0, 0.75), 1); // replaces in place
        assert_eq!(a.points.len(), 3);
        a.move_point(1, 100.0, 2.0);
        assert_eq!(a.points[1], (12.0, 1.0));
    }
}
