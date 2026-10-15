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
    /// Song tempo (0..1 maps to 32..255 BPM). Lives on track 0; applies while the pattern plays.
    Bpm,
    /// Lines per beat (0..1 maps to 1..16).
    Lpb,
}

impl AutoTarget {
    /// Tempo for a normalized lane value.
    pub fn bpm_for(v: f32) -> f32 {
        32.0 + (255.0 - 32.0) * v.clamp(0.0, 1.0)
    }

    /// Lines per beat for a normalized lane value.
    pub fn lpb_for(v: f32) -> u8 {
        (1.0 + (v.clamp(0.0, 1.0) * 15.0).round()) as u8
    }

    /// Normalized value for a tempo, for drawing and entry.
    pub fn bpm_value(bpm: f32) -> f32 {
        ((bpm - 32.0) / (255.0 - 32.0)).clamp(0.0, 1.0)
    }
}

/// How a lane moves between two points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoCurve {
    /// Straight lines.
    #[default]
    Linear,
    /// Hold each value until the next point.
    Step,
    /// Ease in and out (smoothstep) between points.
    Smooth,
    /// Slow start, fast finish (quadratic).
    EaseIn,
    /// Fast start, slow finish (quadratic).
    EaseOut,
}

impl AutoCurve {
    pub const ALL: [AutoCurve; 5] =
        [AutoCurve::Linear, AutoCurve::Step, AutoCurve::Smooth, AutoCurve::EaseIn, AutoCurve::EaseOut];

    pub fn label(self) -> &'static str {
        match self {
            AutoCurve::Linear => "Linear",
            AutoCurve::Step => "Step (hold)",
            AutoCurve::Smooth => "Smooth",
            AutoCurve::EaseIn => "Ease in",
            AutoCurve::EaseOut => "Ease out",
        }
    }

    /// Shape a 0..1 position within a segment.
    pub fn shape(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            AutoCurve::Linear => t,
            AutoCurve::Step => (t >= 1.0) as u8 as f32,
            AutoCurve::Smooth => t * t * (3.0 - 2.0 * t),
            AutoCurve::EaseIn => t * t,
            AutoCurve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        }
    }
}

/// Points are `(line position, value 0..=1)`; positions may be fractional, ascending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Automation {
    pub track: usize,
    pub target: AutoTarget,
    pub points: Vec<(f32, f32)>,
    /// Shape of the segments between points.
    #[serde(default)]
    pub curve: AutoCurve,
}

impl Automation {
    pub fn new(track: usize, target: AutoTarget) -> Self {
        Self { track, target, points: Vec::new(), curve: AutoCurve::Linear }
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
                        let t = self.curve.shape((pos - w[0].0) / span);
                        return Some(w[0].1 + (w[1].1 - w[0].1) * t);
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

    /// Points whose position lies in `[from, to]`.
    pub fn points_in(&self, from: f32, to: f32) -> Vec<(f32, f32)> {
        self.points.iter().copied().filter(|p| p.0 >= from && p.0 <= to).collect()
    }

    /// Remove the points in `[from, to]`.
    pub fn clear_range(&mut self, from: f32, to: f32) {
        self.points.retain(|p| p.0 < from || p.0 > to);
    }

    /// Insert `pts` (positions relative to 0) at `at`, replacing what was in that span. Positions past
    /// `max_line` are dropped.
    pub fn paste(&mut self, pts: &[(f32, f32)], at: f32, max_line: f32) {
        let Some(last) = pts.last().map(|p| p.0) else { return };
        self.clear_range(at, at + last);
        for &(l, v) in pts {
            let l = at + l;
            if l <= max_line {
                self.set_point(l, v);
            }
        }
    }

    /// Scale the values in `[from, to]` around `center` (0..1) by `factor`.
    pub fn scale_range(&mut self, from: f32, to: f32, center: f32, factor: f32) {
        for p in self.points.iter_mut().filter(|p| p.0 >= from && p.0 <= to) {
            p.1 = (center + (p.1 - center) * factor).clamp(0.0, 1.0);
        }
    }

    /// Mirror the values in `[from, to]` (high becomes low).
    pub fn flip_range(&mut self, from: f32, to: f32) {
        for p in self.points.iter_mut().filter(|p| p.0 >= from && p.0 <= to) {
            p.1 = 1.0 - p.1;
        }
    }

    /// Reverse the points of `[from, to]` in time.
    pub fn reverse_range(&mut self, from: f32, to: f32) {
        let mut moved: Vec<(f32, f32)> =
            self.points_in(from, to).iter().map(|p| (from + to - p.0, p.1)).collect();
        self.clear_range(from, to);
        moved.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (l, v) in moved {
            self.set_point(l, v);
        }
    }

    /// Move the points of `[from, to]` by `delta` lines (clamped to the pattern).
    pub fn shift_range(&mut self, from: f32, to: f32, delta: f32, max_line: f32) {
        let moved: Vec<(f32, f32)> = self.points_in(from, to);
        self.clear_range(from, to);
        for (l, v) in moved {
            let l = l + delta;
            if (0.0..=max_line).contains(&l) {
                self.set_point(l, v);
            }
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

    #[test]
    fn curves_shape_the_segments() {
        let mut a = Automation::new(0, AutoTarget::TrackVolume);
        a.set_point(0.0, 0.0);
        a.set_point(10.0, 1.0);
        a.curve = AutoCurve::Step;
        assert_eq!(a.value_at(9.0), Some(0.0));
        a.curve = AutoCurve::Smooth;
        assert_eq!(a.value_at(5.0), Some(0.5));
        assert!(a.value_at(2.5).unwrap() < 0.25, "smooth starts slowly");
        a.curve = AutoCurve::EaseIn;
        assert!((a.value_at(5.0).unwrap() - 0.25).abs() < 1e-6);
        a.curve = AutoCurve::EaseOut;
        assert!((a.value_at(5.0).unwrap() - 0.75).abs() < 1e-6);
        let old: Automation =
            serde_json::from_str(r#"{"track":0,"target":"TrackVolume","points":[[0.0,1.0]]}"#).unwrap();
        assert_eq!(old.curve, AutoCurve::Linear, "lanes saved before curves load as linear");
    }

    #[test]
    fn range_tools_copy_paste_scale_flip_reverse_shift() {
        let mut a = Automation::new(0, AutoTarget::TrackVolume);
        for (l, v) in [(0.0, 0.0), (2.0, 0.5), (4.0, 1.0), (8.0, 0.2)] {
            a.set_point(l, v);
        }
        let copy: Vec<(f32, f32)> = a.points_in(0.0, 4.0).iter().map(|p| (p.0, p.1)).collect();
        assert_eq!(copy.len(), 3);
        a.paste(&copy, 10.0, 16.0);
        assert_eq!(a.points.len(), 7);
        assert_eq!(a.value_at(14.0), Some(1.0));
        a.paste(&copy, 14.0, 16.0); // the part past the pattern end is dropped
        assert!(a.points.iter().all(|p| p.0 <= 16.0));
        let mut b = Automation::new(0, AutoTarget::TrackVolume);
        for (l, v) in [(0.0, 0.0), (2.0, 0.5), (4.0, 1.0)] {
            b.set_point(l, v);
        }
        b.scale_range(0.0, 4.0, 0.5, 0.5);
        assert_eq!(b.points, vec![(0.0, 0.25), (2.0, 0.5), (4.0, 0.75)]);
        b.flip_range(0.0, 4.0);
        assert_eq!(b.points, vec![(0.0, 0.75), (2.0, 0.5), (4.0, 0.25)]);
        b.reverse_range(0.0, 4.0);
        assert_eq!(b.points, vec![(0.0, 0.25), (2.0, 0.5), (4.0, 0.75)]);
        b.shift_range(0.0, 4.0, 3.0, 6.0);
        assert_eq!(b.points, vec![(3.0, 0.25), (5.0, 0.5)], "points shifted past the end are dropped");
        b.clear_range(0.0, 10.0);
        assert!(b.points.is_empty());
    }

    #[test]
    fn tempo_and_lpb_mapping() {
        assert_eq!(AutoTarget::bpm_for(0.0), 32.0);
        assert_eq!(AutoTarget::bpm_for(1.0), 255.0);
        assert!((AutoTarget::bpm_value(AutoTarget::bpm_for(0.37)) - 0.37).abs() < 1e-6);
        assert_eq!((AutoTarget::lpb_for(0.0), AutoTarget::lpb_for(1.0)), (1, 16));
    }
}
