//! Finite-duration wheel impulses. Input extends the tail, never restarts older motion.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
const DURATION: Duration = Duration::from_millis(140);
const MAX_IMPULSES: usize = 64;
#[derive(Debug)]
struct Impulse {
    start: Instant,
    amount: f32,
    applied: f32,
}
#[derive(Debug, Default)]
pub struct Controller {
    impulses: VecDeque<Impulse>,
}
impl Controller {
    pub fn remaining(&self) -> f32 {
        self.impulses.iter().map(|p| p.amount - p.applied).sum()
    }
    pub fn active(&self) -> bool {
        !self.impulses.is_empty()
    }
    pub fn cancel(&mut self) {
        self.impulses.clear();
    }
    pub fn push(&mut self, amount: f32, now: Instant) {
        if !amount.is_finite() || amount.abs() < f32::EPSILON {
            return;
        }
        // Bound hostile/high-rate input without dropping distance or resetting the older tail.
        if self.impulses.len() == MAX_IMPULSES {
            let tail = self.impulses.back_mut().unwrap();
            tail.amount += amount;
        } else {
            self.impulses.push_back(Impulse {
                start: now,
                amount,
                applied: 0.,
            });
        }
    }
    pub fn advance(&mut self, now: Instant) -> f32 {
        let mut delta = 0.;
        for p in &mut self.impulses {
            let t = (now.saturating_duration_since(p.start).as_secs_f32() / DURATION.as_secs_f32())
                .min(1.);
            let value = p.amount * (1. - (1. - t).powi(3));
            delta += value - p.applied;
            p.applied = value;
        }
        while self
            .impulses
            .front()
            .is_some_and(|p| now.saturating_duration_since(p.start) >= DURATION)
        {
            self.impulses.pop_front();
        }
        delta
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ease_out_finishes_and_stops() {
        let now = Instant::now();
        let mut c = Controller::default();
        c.push(60., now);
        assert_eq!(c.advance(now), 0.);
        let a = c.advance(now + Duration::from_millis(70));
        assert!((a - 52.5).abs() < 0.001);
        let b = c.advance(now + DURATION);
        assert!((a + b - 60.).abs() < 0.001);
        assert!(!c.active());
    }
    #[test]
    fn accumulation_does_not_restart_older_impulses() {
        let now = Instant::now();
        let mut c = Controller::default();
        c.push(60., now);
        let a = c.advance(now + Duration::from_millis(70));
        c.push(60., now + Duration::from_millis(70));
        let b = c.advance(now + DURATION);
        assert!((a + b - 112.5).abs() < 0.001);
        let d = c.advance(now + Duration::from_millis(210));
        assert!((a + b + d - 120.).abs() < 0.001);
        assert!(!c.active());
    }
    #[test]
    fn refresh_rate_independent_bounded_and_cancelable() {
        for hz in [60, 144, 240] {
            let now = Instant::now();
            let mut c = Controller::default();
            for _ in 0..1000 {
                c.push(0.25, now);
            }
            assert!(c.impulses.len() <= MAX_IMPULSES);
            let mut sum = 0.;
            for n in 0..=hz {
                sum += c.advance(now + Duration::from_secs_f64(n as f64 / hz as f64));
            }
            assert!((sum - 250.).abs() < 0.01);
            assert!(!c.active());
            c.push(-20., now);
            c.cancel();
            assert_eq!(c.remaining(), 0.);
        }
    }
}
