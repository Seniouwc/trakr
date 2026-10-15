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
    /// Point indices `(from, to)`: while the key is held the envelope repeats the stretch between them.
    #[serde(default)]
    pub loop_points: Option<(usize, usize)>,
    /// Shape of the segments between points.
    #[serde(default)]
    pub curve: crate::automation::AutoCurve,
}

impl Envelope {
    /// A classic attack/decay/sustain/release volume shape.
    pub fn adsr(attack: f32, decay: f32, sustain_level: f32, release: f32) -> Self {
        Self {
            enabled: true,
            points: vec![
                (0.0, 0.0),
                (attack, 1.0),
                (attack + decay, sustain_level),
                (attack + decay + release, 0.0),
            ],
            sustain: Some(2),
            loop_points: None,
            curve: crate::automation::AutoCurve::Linear,
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
        if let Some((a, b)) = self.loop_points {
            if let (Some(pa), Some(pb)) = (self.points.get(a), self.points.get(b)) {
                let (ta, tb) = (pa.0, pb.0);
                if tb > ta + 1e-4 {
                    let looped = |t: f32| if t > tb { ta + (t - ta) % (tb - ta) } else { t };
                    return match released_at {
                        None => looped(age),
                        // after note-off the envelope carries on forward from where the loop was
                        Some(r) => looped(r) + (age - r).max(0.0),
                    };
                }
            }
        }
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
                        let k = self.curve.shape((t - w[0].0) / span);
                        return w[0].1 + (w[1].1 - w[0].1) * k;
                    }
                }
                p[p.len() - 1].1
            }
        }
    }

    /// Mirror every value (high becomes low).
    pub fn flip(&mut self) {
        self.points.iter_mut().for_each(|p| p.1 = 1.0 - p.1);
    }

    /// Play the shape backwards in time. Sustain and loop points follow their points.
    pub fn reverse(&mut self) {
        let end = self.end_time();
        let n = self.points.len();
        self.points.reverse();
        self.points.iter_mut().for_each(|p| p.0 = end - p.0);
        self.sustain = self.sustain.map(|s| n - 1 - s);
        self.loop_points = self.loop_points.map(|(a, b)| (n - 1 - b, n - 1 - a));
    }

    /// Scale the values around the middle (0.5) by `factor`.
    pub fn scale_values(&mut self, factor: f32) {
        if factor.is_finite() {
            self.points.iter_mut().for_each(|p| p.1 = (0.5 + (p.1 - 0.5) * factor).clamp(0.0, 1.0));
        }
    }

    /// Stretch or squeeze the time axis by `factor` (2.0 = twice as slow).
    pub fn scale_time(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.points.iter_mut().for_each(|p| p.0 *= factor);
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

    #[test]
    fn loop_repeats_while_held_and_carries_on_after_release() {
        let e = Envelope {
            enabled: true,
            points: vec![(0.0, 0.0), (0.1, 1.0), (0.2, 0.0), (0.5, 0.0)],
            sustain: None,
            loop_points: Some((1, 2)),
            curve: Default::default(),
        };
        // held: 0..0.1 plays once, then 0.1..0.2 repeats forever
        assert!((e.effective_time(0.05, None) - 0.05).abs() < 1e-6);
        assert!((e.effective_time(0.25, None) - 0.15).abs() < 1e-5);
        assert!((e.effective_time(1.05, None) - 0.15).abs() < 1e-4);
        // released at 0.25 (inside the loop at 0.15): continues forward from there
        assert!((e.effective_time(0.35, Some(0.25)) - 0.25).abs() < 1e-5);
        // a damaged loop (end before start, or out of range) is ignored
        let mut bad = e.clone();
        bad.loop_points = Some((2, 1));
        assert_eq!(bad.effective_time(0.3, None), 0.3);
        bad.loop_points = Some((1, 99));
        assert_eq!(bad.effective_time(0.3, None), 0.3);
    }

    #[test]
    fn curves_and_edit_helpers() {
        let mut e = Envelope::adsr(0.1, 0.1, 0.5, 0.2);
        assert!((e.value_at(0.05) - 0.5).abs() < 1e-6);
        e.curve = crate::automation::AutoCurve::EaseIn;
        assert!((e.value_at(0.05) - 0.25).abs() < 1e-6, "ease-in is below the straight line");
        e.curve = Default::default();
        let before = e.clone();
        e.flip();
        assert!((e.points[1].1 - 0.0).abs() < 1e-6 && (e.points[0].1 - 1.0).abs() < 1e-6);
        e.flip();
        assert_eq!(e, before);
        e.scale_values(0.5);
        assert!((e.points[1].1 - 0.75).abs() < 1e-6);
        e.scale_values(f32::NAN);
        assert!((e.points[1].1 - 0.75).abs() < 1e-6);
        let mut r = before.clone();
        r.reverse();
        assert!((r.points[0].0 - 0.0).abs() < 1e-6 && (r.points[3].0 - r.end_time()).abs() < 1e-6);
        assert_eq!(r.sustain, Some(1), "sustain follows its point (2 of 4 mirrors to 1)");
        assert!((r.points[3].1 - 0.0).abs() < 1e-6 && (r.points[0].1 - 0.0).abs() < 1e-6);
        r.reverse();
        assert_eq!(
            r.points.iter().map(|p| (p.0 * 1000.0).round()).collect::<Vec<_>>(),
            before.points.iter().map(|p| (p.0 * 1000.0).round()).collect::<Vec<_>>()
        );
        let mut t = before.clone();
        t.scale_time(2.0);
        assert!((t.end_time() - before.end_time() * 2.0).abs() < 1e-6);
        t.scale_time(-1.0);
        t.scale_time(0.0);
        assert!((t.end_time() - before.end_time() * 2.0).abs() < 1e-6, "bad factors are ignored");
    }
}
