mod api;
mod menu;
mod transition;
mod updates;

use transition::{Side, Transition};
use updates::{Check, Updater};

use crate::app_window::{self, AppWindow, Page};
use crate::index::{Drive, Index, Ranking};
use crate::input;
use crate::launcher::{self, Card, Expand, Launcher};
use crate::platform::{self, Binding, Event, Host, HostKind, Hotkey, SearchEvent, Watching};
use crate::search::Filter;
use crate::settings::{ActionSpec, History, Scope, Settings, Shortcut};
use crate::theme::{Appearance, Catalog, Theme};
use crate::ui;
use crate::ui::Subject;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    App, AppContext, Context, Entity, Global, Subscription, Task, WindowBackgroundAppearance, WindowHandle,
    WindowOptions,
};
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::{Duration, Instant};

const COUNT_DELAY: Duration = Duration::from_millis(500);
const REFRESH_INTERVAL: Duration = Duration::from_millis(250);
const SAVE_DELAY: Duration = Duration::from_secs(1);
const PLACEMENT_DELAY: Duration = Duration::from_millis(16);

struct Running {
    _shell: Entity<Shell>,
}

impl Global for Running {}

pub(crate) fn start(index: Arc<Index>, events: UnboundedReceiver<Event>, cx: &mut App) {
    ui::publish_status(&index, cx);
    app_window::probe(cx);
    apply_theme(cx);
    apply_language(cx);
    ui::start_menus(cx);
    let mut card = None;
    let card_opened = cx.open_window(ui::popup_options(cx), |window, cx| {
        platform::attach_card(window, cx);
        let entity = cx.new(|cx| Card::new(window, cx));
        card = Some(entity.clone());
        entity
    });
    if let Err(error) = card_opened {
        log::warn!("Couldn't open the preview window: {error:#}");
    }
    let overlay = WindowOptions { window_background: WindowBackgroundAppearance::Transparent, ..ui::popup_options(cx) };
    let transition = cx
        .open_window(overlay, |window, cx| {
            platform::attach_transition(window, cx);
            cx.new(|_| Transition::default())
        })
        .inspect_err(|error| log::warn!("Couldn't open the transition window: {error:#}"))
        .ok();
    let searched = index.clone();
    let opened = cx.open_window(ui::popup_options(cx), |window, cx| {
        platform::attach(window, cx);
        let launcher = cx.new(|cx| Launcher::new(searched, card, window, cx));
        let closing = launcher.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Err(error) = closing.update(cx, |launcher, cx| launcher.close(cx)) {
                log::warn!("The search window is gone: {error:#}");
            }
            false
        });
        launcher
    });
    match opened {
        Ok(launcher) => {
            let shell = cx.new(|cx| Shell::new(index, launcher, transition, events, cx));
            cx.set_global(Running { _shell: shell });
        }
        Err(error) => {
            log::error!("Couldn't open the search window: {error:#}");
            cx.quit();
        }
    }
}

struct Shell {
    index: Arc<Index>,
    launcher: WindowHandle<Launcher>,
    transition: Option<WindowHandle<Transition>>,
    app_window: Option<WindowHandle<AppWindow>>,
    saved: Settings,
    filters: Arc<RwLock<Vec<Filter>>>,
    api: Option<api::Server>,
    counting: Option<Task<()>>,
    refreshing: Option<Task<()>>,
    refreshed: Option<Instant>,
    settings_save: Option<Task<()>>,
    history_save: Option<Task<()>>,
    updater: Updater,
    _subscriptions: Vec<Subscription>,
    _events: Task<()>,
}

impl Shell {
    fn new(
        index: Arc<Index>,
        launcher: WindowHandle<Launcher>,
        transition: Option<WindowHandle<Transition>>,
        mut events: UnboundedReceiver<Event>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![
            cx.observe_global::<Settings>(|this, cx| this.settings_changed(cx)),
            cx.observe_global::<History>(|this, cx| this.history_save = Some(later(cx, Self::save_history))),
            cx.on_app_quit(|this, cx| {
                this.save_settings(cx);
                this.save_history(cx);
                this.index.close_content();
                this.index.save();
                std::future::ready(())
            }),
            cx.observe_global::<Catalog>(|_, cx| apply_theme(cx)),
            ui::hide_tips_on_keys(cx),
            cx.on_keyboard_layout_change(|cx| {
                let actions = cx.global::<Settings>().actions.clone();
                bind_keys(&actions, cx);
            }),
        ];
        match launcher.update(cx, |_, window, _| window.observe_window_appearance(|_, cx| apply_theme(cx))) {
            Ok(subscription) => subscriptions.push(subscription),
            Err(error) => log::warn!("Couldn't follow the system theme: {error:#}"),
        }
        match launcher.update(cx, |_, _, cx| cx.entity()) {
            Ok(entity) => {
                subscriptions.push(cx.subscribe(&entity, |this, _, expand: &Expand, cx| this.expand(expand, cx)))
            }
            Err(error) => log::warn!("Couldn't connect the search window: {error:#}"),
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.prepare_app_window(cx));
        })
        .detach();
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |this, cx| this.handle(event, cx)).is_err() {
                    break;
                }
            }
        });
        let saved = cx.global::<Settings>().clone();
        let mut shell = Self {
            index,
            launcher,
            transition,
            app_window: None,
            filters: Arc::new(RwLock::new(saved.filters.clone())),
            saved,
            api: None,
            counting: None,
            refreshing: None,
            refreshed: None,
            settings_save: None,
            history_save: None,
            updater: Updater::default(),
            _subscriptions: subscriptions,
            _events: events,
        };
        shell.sync_api();
        shell.schedule_updates(shell.saved.check_updates, cx);
        shell
    }

    fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Search(event) => {
                if let Err(error) = self.launcher.update(cx, |launcher, window, cx| launcher.handle(event, window, cx))
                {
                    log::warn!("The search window is gone: {error:#}");
                }
            }
            Event::Hotkey(hotkey, host) => self.hotkey(hotkey, host, cx),
            Event::FolderMenu(clicked) => menu::open(Some(clicked.host), clicked.folder, clicked.hidden_shown, cx),
            Event::IndexChanged => self.index_changed(cx),
            Event::IndexCounted => self.index_counted(cx),
            Event::Rebuild => self.index.rebuild(),
            Event::DeleteContentCache => self.index.delete_content(),
            Event::OpenWindow => self.open_app_window(Page::Search, None, cx),
            Event::OpenSettings => self.open_app_window(Page::SETTINGS, None, cx),
            Event::CheckUpdates => self.check_updates(Check::Manual, cx),
            Event::InstallUpdate => self.install_update(cx),
            Event::ShowUpdate => self.show_update(cx),
            Event::Quit => cx.quit(),
        }
    }

    fn hotkey(&mut self, hotkey: Hotkey, host: Option<Host>, cx: &mut Context<Self>) {
        match hotkey {
            Hotkey::Search => self.handle(Event::Search(SearchEvent::Summon), cx),
            Hotkey::Window => self.open_app_window(Page::Search, None, cx),
            Hotkey::Menu => menu::show(host, cx),
            Hotkey::Action(position) => {
                if self.run_in_windows(position, cx) {
                    return;
                }
                let Some(action) = cx.global::<Settings>().actions.get(position).cloned() else { return };
                cx.spawn(async move |this, cx| {
                    let subject = cx.background_spawn(async move { selection(host) }).await;
                    if !subject.paths.is_empty() {
                        let _ = this.update(cx, |_, cx| ui::perform_action(&action, subject, cx));
                    }
                })
                .detach();
            }
            Hotkey::Command(position) => {
                let Some(command) = cx.global::<Settings>().commands.get(position).cloned() else { return };
                cx.spawn(async move |this, cx| {
                    let folder = cx.background_spawn(async move { platform::current_folder(host) }).await;
                    let subject = Subject { folder, ..Subject::default() };
                    let _ = this.update(cx, |_, cx| ui::perform_command(&command, subject, host, cx));
                })
                .detach();
            }
        }
    }

    fn run_in_windows(&mut self, position: usize, cx: &mut Context<Self>) -> bool {
        let launched = self.launcher.update(cx, |launcher, _, cx| launcher.run_hotkey_action(position, cx));
        if launched.unwrap_or(false) {
            return true;
        }
        self.app_window.is_some_and(|handle| {
            handle
                .update(cx, |app_window, window, cx| {
                    app_window.run_hotkey_action(position, window.is_window_active(), cx)
                })
                .unwrap_or(false)
        })
    }

    fn sync_api(&mut self) {
        let api = &self.saved.api;
        if self.api.as_ref().map(api::Server::port) == api.enabled.then_some(api.port) {
            return;
        }
        self.api = None;
        if api.enabled {
            self.api = api::Server::start(api.port, self.index.clone(), self.filters.clone());
        }
    }

    fn index_changed(&mut self, cx: &mut Context<Self>) {
        if self.refreshing.is_some() {
            return;
        }
        let wait = self.refreshed.map_or(Duration::ZERO, |at| REFRESH_INTERVAL.saturating_sub(at.elapsed()));
        if wait.is_zero() {
            self.refresh(cx);
            return;
        }
        self.refreshing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.refreshing = None;
                this.refresh(cx);
            });
        }));
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refreshed = Some(Instant::now());
        ui::publish_status(&self.index, cx);
        self.refresh_launcher(cx);
        if let Some(handle) = self.app_window {
            let _ = handle.update(cx, |app_window, _, cx| app_window.index_changed(cx));
        }
    }

    fn index_counted(&mut self, cx: &mut Context<Self>) {
        if self.counting.is_some() {
            return;
        }
        self.counting = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COUNT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.counting = None;
                ui::publish_status(&this.index, cx);
            });
        }));
    }

    fn refresh_launcher(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.launcher.update(cx, |launcher, _, cx| launcher.refresh(cx)) {
            log::warn!("The search window is gone: {error:#}");
        }
    }

    /// Opens the Vestigo window from the search window: the bar grows into the window when the
    /// user can't see the window (hidden, minimized or covered by other windows) and neither is
    /// blurred, otherwise the bar just hides and the window comes forward.
    fn expand(&mut self, expand: &Expand, cx: &mut Context<Self>) {
        let (page, query) = match expand {
            Expand::Search(query) => (Page::Search, Some(query.clone())),
            Expand::Settings => (Page::SETTINGS, None),
        };
        let launcher = self.launcher;
        let leave = move |cx: &mut App| {
            if let Err(error) = launcher.update(cx, |launcher, _, cx| launcher.close(cx)) {
                log::warn!("The search window is gone: {error:#}");
            }
        };
        let looks = launcher.update(cx, |launcher, window, cx| (ui::pixel_ratio(window), launcher.chrome(cx)));
        let (bar_unit, bar) = looks.unwrap_or((1.0, None));
        let bar = bar.unwrap_or_default();
        let frosted = bar.frosted() || cx.global::<Theme>().windows.app.frosted();
        let Some(transition) =
            self.transition.filter(|_| !frosted && !platform::motion_reduced() && !platform::app_window_in_sight())
        else {
            leave(cx);
            self.open_app_window(page, query.as_deref(), cx);
            return;
        };
        platform::hold_app_window();
        self.open_app_window(page, query.as_deref(), cx);
        let window_unit =
            self.app_window.and_then(|handle| handle.update(cx, |_, window, _| ui::pixel_ratio(window)).ok());
        let theme = cx.global::<Theme>();
        let looks = (
            (theme.launcher.background, bar, bar_unit),
            (theme.app.background, theme.windows.app.clone(), window_unit.unwrap_or(bar_unit)),
        );
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(PLACEMENT_DELAY).await;
            let shapes = platform::panel_silhouette().zip(platform::app_window_silhouette());
            let area = shapes.and_then(|(from, to)| {
                let side = |silhouette, (color, chrome, unit)| Side::new(silhouette, color, chrome, unit);
                let (from, to) = (side(from, looks.0), side(to, looks.1));
                transition.update(cx, |transition, _, cx| transition.start(from, to, leave, cx)).ok()
            });
            if !area.is_some_and(platform::begin_transition) {
                let _ = transition.update(cx, |transition, _, cx| transition.finish(cx));
                platform::end_transition();
                cx.update(leave);
            }
        })
        .detach();
    }

    fn prepare_app_window(&mut self, cx: &mut Context<Self>) {
        if self.app_window.is_none() {
            self.app_window = app_window::open(self.index.clone(), cx);
        }
    }

    fn open_app_window(&mut self, page: Page, query: Option<&str>, cx: &mut Context<Self>) {
        let show = |handle: WindowHandle<AppWindow>, cx: &mut Context<Self>| {
            handle.update(cx, |app_window, window, cx| app_window.show(page, query, window, cx)).is_ok()
        };
        if self.app_window.is_some_and(|handle| show(handle, cx)) {
            return;
        }
        self.app_window = None;
        self.prepare_app_window(cx);
        if !self.app_window.is_some_and(|handle| show(handle, cx)) {
            log::warn!("Couldn't show the Vestigo window");
        }
    }

    fn save_settings(&mut self, _: &mut Context<Self>) {
        if self.settings_save.take().is_some() {
            self.saved.save();
        }
    }

    fn save_history(&mut self, cx: &mut Context<Self>) {
        if self.history_save.take().is_some() {
            cx.global::<History>().save();
        }
    }

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        if *cx.global::<Settings>() == self.saved {
            return;
        }
        let settings = cx.global::<Settings>().clone();
        let changes = settings.changes(&self.saved);
        let previous = std::mem::replace(&mut self.saved, settings.clone());
        self.settings_save = Some(later(cx, Self::save_settings));
        if watching(&settings) != watching(&previous) {
            platform::configure(watching(&settings));
        }
        if changes.theme {
            apply_theme(cx);
        }
        if changes.language {
            apply_language(cx);
        }
        if changes.indexed {
            self.index.restart(drives(&settings), settings.index.folders.clone());
        }
        if changes.rewalk {
            self.index.rewalk_every(settings.index.rewalk.every());
        }
        if changes.full_text {
            self.index.full_text(settings.index.full_text);
        }
        if changes.ranking {
            self.index.rank(ranking(&settings));
            self.index_changed(cx);
        }
        if changes.filters {
            *self.filters.write() = settings.filters.clone();
        }
        if changes.api {
            self.sync_api();
        }
        if changes.actions {
            bind_keys(&settings.actions, cx);
        }
        if changes.recent {
            self.refresh_launcher(cx);
        }
        if changes.updates {
            self.schedule_updates(settings.check_updates, cx);
        }
        if let Some(handle) = self.app_window {
            let _ = handle.update(cx, |app_window, window, cx| app_window.settings_changed(changes, window, cx));
        }
    }
}

fn later(cx: &mut Context<Shell>, save: fn(&mut Shell, &mut Context<Shell>)) -> Task<()> {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        let _ = this.update(cx, save);
    })
}

pub(crate) fn drives(settings: &Settings) -> Vec<Drive> {
    let drives = platform::fixed_drives(platform::is_elevated());
    drives.into_iter().filter(|drive| !settings.index.skipped_drives.contains(&drive.letter)).collect()
}

pub(crate) fn ranking(settings: &Settings) -> Ranking {
    let index = &settings.index;
    Ranking::new(&index.excluded, &index.high, &index.low, index.hidden_low)
}

pub(crate) fn bind_keys(actions: &[ActionSpec], cx: &mut App) {
    cx.clear_key_bindings();
    input::bind_keys(cx);
    launcher::bind_keys(actions, cx);
    app_window::bind_keys(actions, cx);
    ui::bind_menu_keys(cx);
}

pub(crate) fn watching(settings: &Settings) -> Watching {
    let mut bindings = Vec::new();
    let mut bind = |shortcut: &Option<Shortcut>, hotkey: Hotkey, scope: Scope| {
        let Some(shortcut) = shortcut.clone() else { return };
        match scope {
            Scope::Vestigo => {}
            Scope::Explorer => bindings.push(Binding { shortcut, hotkey, explorer: true }),
            Scope::Anywhere if shortcut.has_modifier() => bindings.push(Binding { shortcut, hotkey, explorer: false }),
            Scope::Anywhere => {}
        }
    };
    let hotkeys = &settings.hotkeys;
    bind(&hotkeys.search, Hotkey::Search, Scope::Anywhere);
    bind(&hotkeys.window, Hotkey::Window, Scope::Anywhere);
    bind(&hotkeys.menu, Hotkey::Menu, Scope::Explorer);
    for (position, action) in settings.actions.iter().enumerate().filter(|(_, action)| action.enabled) {
        bind(&action.shortcut, Hotkey::Action(position), action.scope);
    }
    for (position, command) in settings.commands.iter().enumerate().filter(|(_, command)| command.enabled) {
        bind(&command.shortcut, Hotkey::Command(position), command.scope);
    }
    Watching {
        triggers: settings.triggers,
        bindings,
        fullscreen: hotkeys.fullscreen,
        tray: settings.tray,
        disabled: settings
            .disabled_apps
            .iter()
            .map(|app| app.trim().to_lowercase())
            .filter(|app| !app.is_empty())
            .collect(),
        double_click: settings.menu.double_click,
        middle_click: settings.menu.middle_click,
    }
}

fn selection(host: Option<Host>) -> Subject {
    let Some(host) = host else { return Subject::default() };
    let paths = match host.kind {
        HostKind::Explorer => platform::explorer_selection(host),
        HostKind::Desktop | HostKind::Dialog => Vec::new(),
    };
    Subject { paths, folder: platform::host_folder(host), query: String::new() }
}

fn apply_theme(cx: &mut App) {
    let system = Appearance::of(cx.window_appearance());
    let settings = cx.global::<Settings>();
    let theme = cx.global::<Catalog>().resolve(&settings.theme, system, &settings.theme_edits);
    if cx.try_global() != Some(&theme) {
        cx.set_global(theme);
        cx.refresh_windows();
    }
}

fn apply_language(cx: &mut App) {
    let language = cx.global::<Settings>().language.resolve(platform::system_language());
    cx.set_global(language);
    platform::sync_tray(language);
    cx.refresh_windows();
}
