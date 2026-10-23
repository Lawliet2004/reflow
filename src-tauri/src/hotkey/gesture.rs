//! Binding-independent push-to-talk / double-tap recognition. Times come from
//! the input callback, so slow microphone admission cannot widen the gesture.
use std::time::{Duration, Instant};

const DOUBLE_TAP: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Start(Instant),
    Stop(Instant),
    Lock,
}

#[derive(Default)]
pub(crate) struct Gesture {
    down: bool,
    first_press: Option<Instant>,
    pending_release: Option<Instant>,
    locked: bool,
}

impl Gesture {
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.pending_release
            .and(self.first_press.map(|at| at + DOUBLE_TAP))
    }

    pub(crate) fn expire(&mut self) -> Vec<Action> {
        let released = self.pending_release.take();
        self.first_press = None;
        released
            .map(|at| vec![Action::Stop(at)])
            .unwrap_or_default()
    }

    pub(crate) fn input(&mut self, pressed: bool, at: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.deadline().is_some_and(|deadline| at > deadline) {
            actions.extend(self.expire());
        }
        if self.down == pressed {
            return actions; // OS repeat is not another tap.
        }
        self.down = pressed;
        if pressed {
            if self.locked {
                self.locked = false;
                actions.push(Action::Stop(at));
            } else if self.pending_release.take().is_some() {
                self.first_press = None;
                self.locked = true;
                actions.push(Action::Lock);
            } else {
                self.first_press = Some(at);
                actions.push(Action::Start(at));
            }
        } else if let Some(first) = self.first_press {
            if at.saturating_duration_since(first) >= DOUBLE_TAP {
                self.first_press = None;
                actions.push(Action::Stop(at));
            } else {
                self.pending_release = Some(at);
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_tap_locks_until_next_press_and_ignores_repeats() {
        let t = Instant::now();
        let mut g = Gesture::default();
        assert_eq!(g.input(true, t), vec![Action::Start(t)]);
        assert!(g.input(true, t + Duration::from_millis(40)).is_empty());
        assert!(g.input(false, t + Duration::from_millis(80)).is_empty());
        assert_eq!(
            g.input(true, t + Duration::from_millis(180)),
            vec![Action::Lock]
        );
        assert!(g.input(true, t + Duration::from_millis(200)).is_empty());
        assert!(g.input(false, t + Duration::from_millis(250)).is_empty());
        assert_eq!(g.deadline(), None);
        let stop = t + Duration::from_secs(20);
        assert_eq!(g.input(true, stop), vec![Action::Stop(stop)]);
        assert!(g.input(true, stop).is_empty());
        assert!(g.input(false, stop).is_empty());
    }

    #[test]
    fn long_hold_stops_on_release_without_delay() {
        let t = Instant::now();
        let mut g = Gesture::default();
        g.input(true, t);
        let release = t + Duration::from_secs(2);
        assert_eq!(g.input(false, release), vec![Action::Stop(release)]);
        assert_eq!(g.deadline(), None);
    }

    #[test]
    fn single_tap_timeout_preserves_release_timestamp() {
        let t = Instant::now();
        let mut g = Gesture::default();
        g.input(true, t);
        let release = t + Duration::from_millis(50);
        g.input(false, release);
        assert_eq!(g.deadline(), Some(t + DOUBLE_TAP));
        assert_eq!(g.expire(), vec![Action::Stop(release)]);
        assert!(g.expire().is_empty());
    }

    #[test]
    fn late_second_press_is_two_sessions_even_when_worker_was_busy() {
        let t = Instant::now();
        let mut g = Gesture::default();
        g.input(true, t);
        let release = t + Duration::from_millis(300);
        g.input(false, release);
        let late = t + Duration::from_millis(501);
        assert_eq!(
            g.input(true, late),
            vec![Action::Stop(release), Action::Start(late)]
        );
    }

    #[test]
    fn second_press_at_window_boundary_locks() {
        let t = Instant::now();
        let mut g = Gesture::default();
        g.input(true, t);
        g.input(false, t + Duration::from_millis(100));
        assert_eq!(g.input(true, t + DOUBLE_TAP), vec![Action::Lock]);
    }

    #[test]
    fn stray_release_and_fresh_gesture_after_reset_are_harmless() {
        let t = Instant::now();
        let mut g = Gesture::default();
        assert!(g.input(false, t).is_empty());
        assert_eq!(g.input(true, t), vec![Action::Start(t)]);
    }
}
