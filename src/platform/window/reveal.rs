use std::cell::Cell;

/// How the Vestigo window is coming forward. `Held` and `Asked`: a transition's shape is growing
/// and the window waits (`Asked`: a show came meanwhile). `Under` and `Shown`: the shape rests on
/// the window, which appears beneath it without a shadow, before and after its show. `Appearing`:
/// a show without a transition, whose shadow waits for the window's first frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reveal {
    Plain,
    Appearing,
    Held,
    Asked,
    Under,
    Shown,
}

thread_local! {
    static STATE: Cell<Reveal> = const { Cell::new(Reveal::Plain) };
}

pub(super) fn current() -> Reveal {
    STATE.get()
}

/// Applies a step and returns the state before it.
pub(super) fn step(change: fn(Reveal) -> Reveal) -> Reveal {
    STATE.replace(change(STATE.get()))
}

impl Reveal {
    pub(super) fn hold(self) -> Self {
        Self::Held
    }

    pub(super) fn ask(self) -> Self {
        match self {
            Self::Held | Self::Asked => Self::Asked,
            Self::Under | Self::Shown => Self::Shown,
            Self::Plain | Self::Appearing => self,
        }
    }

    pub(super) fn release(self) -> Self {
        match self {
            Self::Held | Self::Asked => Self::Under,
            Self::Plain | Self::Appearing | Self::Under | Self::Shown => self,
        }
    }

    pub(super) fn appear(self) -> Self {
        match self {
            Self::Plain => Self::Appearing,
            Self::Appearing | Self::Held | Self::Asked | Self::Under | Self::Shown => self,
        }
    }

    pub(super) fn appeared(self) -> Self {
        match self {
            Self::Appearing => Self::Plain,
            Self::Plain | Self::Held | Self::Asked | Self::Under | Self::Shown => self,
        }
    }

    pub(super) fn end(self) -> Self {
        Self::Plain
    }

    pub(super) fn waits(self) -> bool {
        matches!(self, Self::Held | Self::Asked)
    }

    pub(super) fn revealed(self) -> bool {
        self == Self::Shown
    }

    pub(super) fn withholds_shadow(self) -> bool {
        matches!(self, Self::Appearing | Self::Under | Self::Shown)
    }
}

#[cfg(test)]
mod tests {
    use super::Reveal;

    #[test]
    fn a_show_during_the_growth_waits_and_counts_once_released() {
        let held = Reveal::Plain.hold();
        assert!(held.waits());
        let asked = held.ask();
        assert!(asked.waits());
        assert!(!asked.revealed());
        let under = asked.release();
        assert!(!under.waits());
        assert!(under.withholds_shadow());
        let shown = under.ask();
        assert!(shown.revealed());
        assert!(shown.withholds_shadow());
        assert_eq!(shown.ask(), Reveal::Shown);
        assert_eq!(shown.end(), Reveal::Plain);
    }

    #[test]
    fn a_window_released_before_its_show_is_not_yet_revealed() {
        let under = Reveal::Plain.hold().release();
        assert_eq!(under, Reveal::Under);
        assert!(!under.revealed());
        assert!(under.ask().revealed());
    }

    #[test]
    fn a_show_without_a_transition_withholds_the_shadow_only_while_appearing() {
        let plain = Reveal::Plain.ask();
        assert_eq!(plain, Reveal::Plain);
        assert!(!plain.withholds_shadow());
        let appearing = plain.appear();
        assert!(appearing.withholds_shadow());
        assert_eq!(appearing.appeared(), Reveal::Plain);
    }

    #[test]
    fn the_shadow_stays_withheld_under_the_shape_after_the_window_appears() {
        let shown = Reveal::Under.ask().appear().appeared();
        assert_eq!(shown, Reveal::Shown);
        assert!(shown.withholds_shadow());
    }

    #[test]
    fn a_release_without_a_hold_changes_nothing() {
        assert_eq!(Reveal::Plain.release(), Reveal::Plain);
        assert_eq!(Reveal::Shown.release(), Reveal::Shown);
    }

    #[test]
    fn a_new_transition_starts_a_new_hold() {
        assert_eq!(Reveal::Shown.hold(), Reveal::Held);
        assert!(!Reveal::Shown.hold().revealed());
    }
}
