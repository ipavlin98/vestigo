use super::{Growth, MAX_ROWS, growth, metrics, panel_height_of};
use crate::assets;
use crate::launcher::{FolderMenu, Launcher, Mode};
use crate::platform::MenuAt;
use crate::theme::{Chrome, Corners, Theme, rem};
use crate::ui::{self, GLYPH, HINT_GAP, ICON_BUTTON, icon_toggle};
use gpui::{AnyElement, Context, MouseButton, div, prelude::*};

const FOLDER_MENUS: [FolderMenu; 2] = [FolderMenu::Favorites, FolderMenu::Recent];
const BUTTONS: f32 = 2.0 * GLYPH + HINT_GAP;
const BUTTON_REACH: f32 = (ICON_BUTTON - GLYPH) / 2.0;

pub(in crate::launcher) fn folder_menu_at(mode: Mode, menu: FolderMenu) -> MenuAt {
    let metrics = metrics(mode);
    let button = menu as usize;
    let end = metrics.width - metrics.gutter();
    let strip = [end - BUTTONS - BUTTON_REACH, end + BUTTON_REACH];
    let height = panel_height_of(mode, MAX_ROWS, metrics.row, true);
    match growth(mode) {
        Growth::Down => MenuAt::Search { button, strip, top: 0.0, bottom: metrics.input, upward: false },
        Growth::Up => MenuAt::Search { button, strip, top: height - metrics.input, bottom: height, upward: true },
    }
}

pub(in crate::launcher) fn bar_chrome(mode: Mode, joined: bool, theme: &Theme) -> Chrome {
    let chrome = mode.chrome(theme);
    if !joined {
        return chrome;
    }
    Chrome { corners: halves(chrome.corners, growth(mode)).0, shadow: Vec::new(), ..chrome }
}

pub(in crate::launcher) fn drop_chrome(mode: Mode, theme: &Theme) -> Chrome {
    let chrome = mode.chrome(theme);
    Chrome { corners: halves(chrome.corners, growth(mode)).1, ..chrome }
}

fn halves(corners: Corners, growth: Growth) -> (Corners, Corners) {
    let top = Corners { bottom_right: 0.0, bottom_left: 0.0, ..corners };
    let bottom = Corners { top_left: 0.0, top_right: 0.0, ..corners };
    match growth {
        Growth::Down => (top, bottom),
        Growth::Up => (bottom, top),
    }
}

impl Launcher {
    pub(in crate::launcher) fn joins_menu(&self, cx: &Context<Self>) -> bool {
        FOLDER_MENUS.iter().any(|&menu| self.folder_menu_at(menu).is_some_and(|at| ui::menu_shown_at(at, cx)))
    }

    pub(super) fn folder_buttons(&self, mode: Mode, theme: &Theme, cx: &Context<Self>) -> AnyElement {
        let buttons = FOLDER_MENUS.map(|menu| self.folder_button(menu, theme, cx));
        div()
            .flex_none()
            .w(rem(BUTTONS))
            .ml(rem(HINT_GAP - metrics(mode).gap))
            .flex()
            .items_center()
            .justify_between()
            .children(buttons)
            .into_any_element()
    }

    fn folder_button(&self, menu: FolderMenu, theme: &Theme, cx: &Context<Self>) -> impl IntoElement + use<> {
        let (id, glyph) = match menu {
            FolderMenu::Favorites => ("favorites", assets::STAR),
            FolderMenu::Recent => ("recent", assets::HISTORY),
        };
        let open = self.folder_menu_at(menu).is_some_and(|at| ui::menu_shown_at(at, cx));
        icon_toggle(id, glyph, open, theme).on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.press_folder_menu(menu, cx);
            }),
        )
    }
}
