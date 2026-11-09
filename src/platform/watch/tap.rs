use std::time::{Duration, Instant};

const TAP_LIMIT: Duration = Duration::from_millis(350);
const DOUBLE_TAP_GAP: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(super) struct CtrlTap {
    pressed: Option<Instant>,
    clean: bool,
    last_tap: Option<Instant>,
}

impl CtrlTap {
    pub(super) fn press(&mut self, down: bool, now: Instant) -> bool {
        if down {
            if self.pressed.is_none() {
                self.pressed = Some(now);
                self.clean = true;
            }
            return false;
        }
        let Some(pressed) = self.pressed.take() else { return false };
        if !self.clean || now.duration_since(pressed) > TAP_LIMIT {
            self.last_tap = None;
            return false;
        }
        if self.last_tap.is_some_and(|last| now.duration_since(last) <= DOUBLE_TAP_GAP) {
            self.last_tap = None;
            return true;
        }
        self.last_tap = Some(now);
        false
    }

    pub(super) fn interrupt(&mut self) {
        self.clean = false;
        self.last_tap = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn two_quick_ctrl_taps_toggle() {
        let start = Instant::now();
        let mut tap = CtrlTap::default();
        assert!(!tap.press(true, at(start, 0)));
        assert!(!tap.press(false, at(start, 80)));
        assert!(!tap.press(true, at(start, 200)));
        assert!(tap.press(false, at(start, 260)));
    }

    #[test]
    fn ctrl_shortcuts_do_not_count_as_taps() {
        let start = Instant::now();
        let mut tap = CtrlTap::default();
        tap.press(true, at(start, 0));
        tap.press(false, at(start, 80));
        tap.press(true, at(start, 150));
        tap.interrupt();
        assert!(!tap.press(false, at(start, 220)));
    }

    #[test]
    fn slow_taps_do_not_toggle() {
        let start = Instant::now();
        let mut tap = CtrlTap::default();
        tap.press(true, at(start, 0));
        tap.press(false, at(start, 80));
        tap.press(true, at(start, 700));
        assert!(!tap.press(false, at(start, 760)));
    }

    #[test]
    fn autorepeat_keeps_the_first_press_time() {
        let start = Instant::now();
        let mut tap = CtrlTap::default();
        tap.press(true, at(start, 0));
        tap.press(true, at(start, 300));
        assert!(!tap.press(false, at(start, 500)));
    }
}
