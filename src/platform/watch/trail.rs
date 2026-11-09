use super::super::Host;

#[derive(Default)]
pub(super) struct Trail {
    explorer: Option<Host>,
    app: Option<u32>,
}

impl Trail {
    pub(super) fn explorer(&mut self, host: Host) {
        self.explorer = Some(host);
        self.app = None;
    }

    pub(super) fn app(&mut self, process: u32) {
        if *self.app.get_or_insert(process) != process {
            self.explorer = None;
        }
    }

    pub(super) fn explorer_host(&self) -> Option<Host> {
        self.explorer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::HostKind;
    use windows::Win32::Foundation::HWND;

    fn explorer() -> Host {
        Host::new(HWND(7 as _), HostKind::Explorer)
    }

    #[test]
    fn every_dialog_of_the_app_reached_from_explorer_gets_that_explorer() {
        let mut trail = Trail::default();
        trail.explorer(explorer());
        trail.app(40);
        assert_eq!(trail.explorer_host(), Some(explorer()));
        trail.app(40);
        assert_eq!(trail.explorer_host(), Some(explorer()));
    }

    #[test]
    fn moving_on_to_another_app_forgets_explorer() {
        let mut trail = Trail::default();
        trail.explorer(explorer());
        trail.app(40);
        trail.app(41);
        trail.app(40);
        assert_eq!(trail.explorer_host(), None);
    }

    #[test]
    fn coming_back_to_explorer_starts_a_new_trail() {
        let mut trail = Trail::default();
        trail.explorer(explorer());
        trail.app(40);
        trail.explorer(explorer());
        trail.app(41);
        assert_eq!(trail.explorer_host(), Some(explorer()));
    }
}
