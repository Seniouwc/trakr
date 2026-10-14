//! Instrument envelopes: piecewise-linear curves over time, with an optional sustain point.

use serde::{Deserialize, Serialize};

/// Points are `(seconds since note start, value 0..=1)`, ascending in time.
/// While the key is held, playback stops at `sustain` (if set); on note-off it continues past it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub enabled: bool,
    pub points: Vec<(f32, f32)>,
    /// Index into `points` where the envelope waits for note-off.
    pub sustain: Option<usize>,
}

impl Envelope {
    /// A classic attack/decay/sustain/release volume shape.
    pub fn adsr(attack: f32, decay: f32, sustain_level: f32, release: f32) -> Self {
        Self {
            enabled: true,
            points: vec![(0.0, 0.0), (attack, 1.0), (attack + decay, sustain_level), (attack + decay + release, 0.0)],
            sustain: Some(2),
        }
    }

    pub fn end_time(&self) -> f32 {
        self.points.last().map_or(0.0, |p| p.0)
    }

    /// Time at which the sustain point sits, if any.
    pub fn sustain_time(&self) -> Option<f32> {
        self.sustain.and_then(|i| self.points.get(i)).map(|p| p.0)
    }

    /// Map elapsed note time to envelope time: held at the sustain point until `released_at`.
    pub fn effective_time(&self, age: f32, released_at: Option<f32>) -> f32 {
        match (self.sustain_time(), released_at) {
            (Some(st), None) => age.min(st),
            (Some(st), Some(r)) => st + (age - r).max(0.0),
            (None, _) => age,
        }
    }

    /// Value at envelope time `t` (linear between points, flat outside). 1.0 when there are no points.
    pub fn value_at(&self, t: f32) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => 1.0,
            1 => p[0].1,
            _ => {
                if t <= p[0].0 {
                    return p[0].1;
                }
                for w in p.windows(2) {
                    if t <= w[1].0 {
                        let span = (w[1].0 - w[0].0).max(1e-6);
                        return w[0].1 + (w[1].1 - w[0].1) * ((t - w[0].0) / span);
                    }
                }
                p[p.len() - 1].1
            }
        }
    }

    /// Insert a point keeping time order; returns its index.
    pub fn insert(&mut self, time: f32, value: f32) -> usize {
        let i = self.points.iter().position(|p| p.0 > time).unwrap_or(self.points.len());
        self.points.insert(i, (time.max(0.0), value.clamp(0.0, 1.0)));
        if let Some(s) = self.sustain.as_mut() {
            if i <= *s {
                *s += 1;
            }
        }
        i
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.points.len() {
            self.points.remove(i);
            self.sustain = match self.sustain {
                Some(s) if s == i => None,
                Some(s) if s > i => Some(s - 1),
                other => other,
            };
        }
    }

    /// Move point `i`, keeping it between its neighbours in time.
    pub fn move_point(&mut self, i: usize, time: f32, value: f32) {
        let lo = if i == 0 { 0.0 } else { self.points[i - 1].0 };
        let hi = self.points.get(i + 1).map_or(f32::MAX, |p| p.0);
        if let Some(p) = self.points.get_mut(i) {
            *p = (time.clamp(lo, hi), value.clamp(0.0, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_shape() {
        let e = Envelope::adsr(0.1, 0.1, 0.5, 0.2);
        assert!((e.value_at(0.05) - 0.5).abs() < 1e-6);
        assert!((e.value_at(0.1) - 1.0).abs() < 1e-6);
        assert!((e.value_at(0.2) - 0.5).abs() < 1e-6);
        // held at sustain while the key is down
        assert!((e.value_at(e.effective_time(5.0, None)) - 0.5).abs() < 1e-6);
        // released at 1.0s: halfway through the 0.2s release
        let t = e.effective_time(1.1, Some(1.0));
        assert!((e.value_at(t) - 0.25).abs() < 1e-5);
        assert_eq!(e.value_at(10.0), 0.0);
    }

    #[test]
    fn editing_keeps_order_and_sustain() {
        let mut e = Envelope::adsr(0.1, 0.1, 0.5, 0.2);
        let i = e.insert(0.05, 0.3);
        assert_eq!(i, 1);
        assert_eq!(e.sustain, Some(3));
        e.remove(1);
        assert_eq!(e.sustain, Some(2));
        e.move_point(1, 5.0, 2.0);
        assert!(e.points[1].0 <= e.points[2].0 && e.points[1].1 == 1.0);
        e.remove(2);
        assert_eq!(e.sustain, None);
    }
}
