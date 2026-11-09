use super::view;
use crate::index::Hit;
use crate::platform::{self, Anchor, Entrance, Host, HostKind, RunningApp};
use crate::settings::Settings;
use crate::theme::{Chrome, Theme};
use crate::ui::{self, Command};
use gpui::App;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Launcher,
    Dialog(Host, Side, usize),
    Explorer(Host),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Below,
    Above,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Appear {
    Focused,
    Unfocused,
}

impl Appear {
    pub(super) fn activates(self) -> bool {
        match self {
            Self::Focused => true,
            Self::Unfocused => false,
        }
    }
}

impl Mode {
    pub(super) fn docked(host: Host, cx: &App) -> Self {
        match host.kind {
            HostKind::Dialog => {
                let room = platform::room(host, ui::zoom(cx));
                let below = view::docked_rows_within(room.below);
                if below == view::MAX_ROWS || room.below >= room.above {
                    Self::Dialog(host, Side::Below, below)
                } else {
                    Self::Dialog(host, Side::Above, view::docked_rows_within(room.above))
                }
            }
            HostKind::Explorer | HostKind::Desktop => Self::Explorer(host),
        }
    }

    pub(super) fn host(self) -> Option<Host> {
        match self {
            Self::Launcher => None,
            Self::Dialog(host, ..) | Self::Explorer(host) => Some(host),
        }
    }

    pub(super) fn max_rows(self) -> usize {
        match self {
            Self::Dialog(_, _, rows) => rows,
            Self::Launcher | Self::Explorer(_) => view::MAX_ROWS,
        }
    }

    pub(super) fn anchor(self, settings: &Settings) -> Anchor {
        match self {
            Self::Launcher => settings.position.map_or(Anchor::Center, Anchor::At),
            Self::Dialog(host, Side::Below, _) => Anchor::BelowHost(host),
            Self::Dialog(host, Side::Above, _) => Anchor::AboveHost(host),
            Self::Explorer(host) => Anchor::InsideHost(host),
        }
    }

    pub(super) fn entrance(self) -> Entrance {
        match self {
            Self::Dialog(..) => Entrance::Fade,
            Self::Launcher | Self::Explorer(_) => Entrance::Instant,
        }
    }

    pub(super) fn chrome(self, theme: &Theme) -> Chrome {
        match self {
            Self::Launcher => theme.windows.launcher.clone(),
            Self::Dialog(_, Side::Below, _) => theme.windows.dialog.clone(),
            Self::Dialog(_, Side::Above, _) => {
                let chrome = &theme.windows.dialog;
                Chrome { corners: chrome.corners.flipped(), ..chrome.clone() }
            }
            Self::Explorer(_) => theme.windows.explorer.clone(),
        }
    }

    pub(super) fn command(self, hit: Hit, reveal: bool, running: Option<RunningApp>) -> Command {
        let folder = hit.folder().to_owned();
        match self {
            Self::Launcher if reveal => Command::Reveal(hit.path),
            Self::Launcher => match running {
                Some(app) => Command::Switch { app, path: hit.path },
                None => Command::Open(hit.path),
            },
            Self::Explorer(host) if reveal => Command::Navigate(host, folder),
            Self::Explorer(host) if hit.dir => Command::Navigate(host, hit.path),
            Self::Explorer(_) => Command::Open(hit.path),
            Self::Dialog(host, ..) if reveal => Command::Choose { host, path: folder, keep_name: true },
            Self::Dialog(host, ..) => Command::Choose { host, path: hit.path, keep_name: hit.dir },
        }
    }
}
