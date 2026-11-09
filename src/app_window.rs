mod menus;
mod search_page;
mod settings_page;
mod view;

use search_page::{Presented, SearchPage};
pub(crate) use settings_page::Section;
use settings_page::{FiltersRenumbered, SettingsPage};

use crate::i18n::Language;
use crate::index::{ContentEstimate, Index, Status};
use crate::platform;
use crate::settings::{self, ActionSpec, Changes, History};
use crate::theme::{Catalog, Theme};
use crate::ui::{self, Icons};
use gpui::{
    App, AppContext, BorrowAppContext, Bounds, Context, Entity, FocusHandle, Focusable, Global, Subscription,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, px,
    size,
};
use std::sync::Arc;

const CONTEXT: &str = "AppWindow";
const TITLE: &str = "Vestigo";

pub(crate) fn bind_keys(actions: &[ActionSpec], cx: &mut App) {
    ui::bind_keys(CONTEXT, actions, cx);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Search,
    Settings(Section),
}

impl Page {
    pub(crate) const SETTINGS: Self = Self::Settings(Section::General);
}

#[derive(Default)]
struct Machine {
    autostart: Option<bool>,
    drives: Vec<char>,
    content_estimate: Option<ContentEstimate>,
}

impl Global for Machine {}

pub(crate) fn probe(cx: &mut App) {
    cx.set_global(Machine::default());
    refresh_autostart(cx);
    settings_page::load_drives(cx);
}

fn refresh_autostart(cx: &mut App) {
    cx.spawn(async move |cx| {
        let enabled = Some(cx.background_spawn(async move { platform::autostart() }).await);
        cx.update(|cx| {
            if cx.global::<Machine>().autostart != enabled {
                cx.update_global::<Machine, _>(|machine, _| machine.autostart = enabled);
            }
        });
    })
    .detach();
}

fn set_autostart(on: bool, cx: &mut App) {
    cx.update_global::<Machine, _>(|machine, _| machine.autostart = Some(on));
    cx.spawn(async move |cx| {
        let changed = cx.background_spawn(async move { platform::set_autostart(on) }).await;
        if let Err(error) = changed {
            log::warn!("Couldn't change the start with Windows: {error:#}");
        }
        cx.update(refresh_autostart);
    })
    .detach();
}

pub(crate) fn open(index: Arc<Index>, cx: &mut App) -> Option<WindowHandle<AppWindow>> {
    let zoom = ui::zoom(cx);
    let default_size = size(px(view::WIDTH * zoom), px(view::HEIGHT * zoom));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, default_size, cx))),
        titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), appears_transparent: true, ..Default::default() }),
        kind: WindowKind::Normal,
        focus: true,
        show: false,
        is_movable: true,
        is_resizable: true,
        is_minimizable: true,
        window_background: WindowBackgroundAppearance::Opaque,
        inactive_frame_interval: None,
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        platform::attach_app_window(window, default_size, cx);
        window.on_window_should_close(cx, |_, cx| {
            cx.spawn(async move |_| platform::hide_app_window()).detach();
            false
        });
        cx.new(|cx| AppWindow::new(index, window, cx))
    });
    match opened {
        Ok(handle) => Some(handle),
        Err(error) => {
            log::warn!("Couldn't open the Vestigo window: {error:#}");
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Search,
    Settings,
}

pub(crate) struct AppWindow {
    view: View,
    search: Entity<SearchPage>,
    settings: Entity<SettingsPage>,
    revealing: bool,
    _subscriptions: Vec<Subscription>,
}

impl AppWindow {
    fn new(index: Arc<Index>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ui::fit_rem(window, cx);
        let ratio = ui::pixel_ratio(window);
        let search = cx.new(|cx| SearchPage::new(index.clone(), ratio, cx));
        let settings = cx.new(|cx| SettingsPage::new(index, window, cx));
        let handle = window.window_handle();
        let this = cx.weak_entity();
        let subscriptions = vec![
            ui::paint_changes(cx),
            cx.subscribe(&search, |this, _, _: &Presented, cx| this.presented(cx)),
            cx.subscribe(&settings, |this, _, renumbered: &FiltersRenumbered, cx| {
                let renumber = renumbered.0;
                this.search.update(cx, |search, cx| search.renumber_filter(renumber, cx));
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                this.search.update(cx, |search, cx| search.set_blinking(active, cx));
                this.settings.update(cx, |settings, cx| settings.activated(active, cx));
            }),
            cx.observe_window_bounds(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Theme>(window, |this, window, cx| this.restyle(window, cx)),
            cx.observe_global_in::<Language>(window, |this, window, cx| this.apply_placeholder(window, cx)),
            cx.observe_global::<History>(|this, cx| this.suggestions_changed(cx)),
            cx.observe_global::<Status>(|_, cx| cx.notify()),
            cx.observe_global::<Icons>(|_, cx| cx.notify()),
            cx.observe_global::<Machine>(|_, cx| cx.notify()),
            cx.intercept_keystrokes(move |event, window, cx| {
                if window.window_handle() == handle
                    && this.update(cx, |this, cx| this.intercept(&event.keystroke, window, cx)).unwrap_or(false)
                {
                    cx.stop_propagation();
                }
            }),
        ];
        let mut app_window =
            Self { view: View::Search, search, settings, revealing: false, _subscriptions: subscriptions };
        app_window.apply_placeholder(window, cx);
        app_window.restyle(window, cx);
        app_window
    }

    pub(crate) fn show(&mut self, page: Page, query: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        self.revealing = platform::app_window_hidden() || platform::app_window_on_hold();
        self.open_page(page, window, cx);
        let select = self.revealing;
        self.search.update(cx, |search, cx| search.show(query, select, cx));
        if !self.revealing {
            window.activate_window();
        }
    }

    fn presented(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.revealing) {
            cx.spawn(async move |_, _| platform::show_app_window()).detach();
        }
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |_, _| platform::hide_app_window()).detach();
    }

    fn home_focus(&self, cx: &App) -> FocusHandle {
        match self.view {
            View::Search => self.search.focus_handle(cx),
            View::Settings => self.settings.focus_handle(cx),
        }
    }

    fn intercept(&mut self, keystroke: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.view {
            View::Search => false,
            View::Settings => self.settings.update(cx, |settings, cx| {
                settings.record_key(keystroke, cx) || settings.focus_finder_on(keystroke, window, cx)
            }),
        }
    }

    pub(crate) fn index_changed(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Search && !platform::app_window_hidden() {
            self.search.update(cx, |search, cx| search.rerun(cx));
        }
        cx.notify();
    }

    pub(crate) fn run_hotkey_action(&mut self, position: usize, active: bool, cx: &mut Context<Self>) -> bool {
        self.view == View::Search && self.search.update(cx, |search, cx| search.run_hotkey_action(position, active, cx))
    }

    fn suggestions_changed(&mut self, cx: &mut Context<Self>) {
        if self.view == View::Search {
            self.search.update(cx, |search, cx| search.suggestions_changed(cx));
        }
    }

    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.close_menu(cx));
        match page {
            Page::Search => {
                self.view = View::Search;
                self.settings.update(cx, |settings, cx| settings.leave(cx));
            }
            Page::Settings(section) => {
                self.view = View::Settings;
                self.settings.update(cx, |settings, cx| settings.open(section, window, cx));
            }
        }
        window.focus(&self.home_focus(cx), cx);
        cx.notify();
    }

    fn choose_filter(&mut self, filter: Option<usize>, add: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.choose_filter(filter, add, cx));
        self.open_page(Page::Search, window, cx);
    }

    pub(crate) fn settings_changed(&mut self, changes: Changes, window: &mut Window, cx: &mut Context<Self>) {
        if changes.full_text {
            self.search.update(cx, |search, cx| search.apply_placeholder(cx));
        }
        if changes.theme {
            self.settings.update(cx, |settings, cx| settings.sync_theme_editor(cx));
        }
        if changes.zoom {
            self.rescale(window, cx);
        }
        if changes.filters {
            self.search.update(cx, |search, cx| search.filters_changed(cx));
        }
        if changes.suggestions || changes.recent {
            self.suggestions_changed(cx);
        }
        self.settings.update(cx, |settings, cx| settings.refind(window, cx));
        cx.notify();
    }

    fn rescale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let zoomed = ui::fit_rem(window, cx);
        let ratio = ui::pixel_ratio(window);
        if self.search.update(cx, |search, _| search.rescale(ratio)) || zoomed {
            self.restyle(window, cx);
        }
    }

    fn restyle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ui::fit_rem(window, cx);
        let theme = cx.global::<Theme>().clone();
        let ratio = ui::pixel_ratio(window);
        self.search.update(cx, |search, cx| search.restyle(ratio, &theme, cx));
        self.settings.update(cx, |settings, cx| settings.restyle(&theme, cx));
        let (frame, zoom) = (theme.windows.app.clone(), ui::zoom(cx));
        cx.spawn(async move |_, _| platform::style_app_window(frame, zoom, view::CAPTION_AREAS)).detach();
        cx.notify();
    }

    fn apply_placeholder(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.search.update(cx, |search, cx| search.apply_placeholder(cx));
        self.settings.update(cx, |settings, cx| settings.apply_placeholder(window, cx));
    }
}

fn reload_themes<T: 'static>(cx: &mut Context<T>) {
    cx.spawn(async move |_, cx| {
        let catalog = cx.background_spawn(async move { Catalog::load(settings::theme_files()) }).await;
        cx.update(|cx| {
            if cx.try_global::<Catalog>() != Some(&catalog) {
                cx.set_global(catalog);
            }
        });
    })
    .detach();
}
