mod card;
mod folders;
mod mode;
mod navigation;
mod running;
mod suggest;
mod tools;
mod view;

pub(crate) use card::Card;
use folders::FolderMenu;
use mode::{Appear, Mode, Side};
use running::{Running, with_running};

use crate::i18n::Language;
use crate::index::{Counts, Facets, Focus, Hit, Index, Results, Status, Token};
use crate::input::{Changed, TextInput};
use crate::platform::{self, Anchor, CardLayout, DialogKey, Host, Picks, SearchEvent};
use crate::search::Query;
use crate::settings::{ActionSpec, History, Opening, Repeat, Settings};
use crate::theme::{Chrome, Theme};
use crate::ui::{
    self, Arrival, Command, Confirm, Dismiss, Entry, Extra, Icons, Listing, PickNext, PickPrevious, ResultList, Reveal,
    SelectNext, SelectPrevious, Trail,
};
use futures::FutureExt;
use futures::future::Shared;
use gpui::{
    App, AppContext, BorrowAppContext, ClickEvent, Context, Entity, EventEmitter, Focusable, Pixels, Point,
    Subscription, Task, Window, actions,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONTEXT: &str = "Launcher";
const RESULT_LIMIT: usize = 50;
const RECENT_LIMIT: usize = view::MAX_ROWS;
const TOGGLE_GRACE: Duration = Duration::from_millis(500);
const APPS_INTERVAL: Duration = Duration::from_secs(60);

actions!(launcher, [Forget, QuickSwitch, ShowFavorites, ShowRecent]);

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = launcher, no_json)]
struct OpenRow {
    position: usize,
}

pub(crate) fn bind_keys(actions: &[ActionSpec], cx: &mut App) {
    ui::bind_keys(CONTEXT, actions, cx);
    let fixed = [
        ui::binding("shift-delete", Forget, CONTEXT, cx),
        ui::binding("ctrl-h", Forget, CONTEXT, cx),
        ui::binding("ctrl-g", QuickSwitch, CONTEXT, cx),
        ui::binding("ctrl-b", ShowFavorites, CONTEXT, cx),
        ui::binding("ctrl-r", ShowRecent, CONTEXT, cx),
        ui::binding("backspace", ui::Back, CONTEXT, cx),
    ];
    let rows = (0..view::SHORTCUTS)
        .filter_map(|position| ui::binding(&format!("ctrl-{}", position + 1), OpenRow { position }, CONTEXT, cx));
    let bindings: Vec<_> = fixed.into_iter().flatten().chain(rows).collect();
    cx.bind_keys(bindings);
}

pub(crate) enum Expand {
    Search(String),
    Settings,
}

pub(crate) struct Launcher {
    input: Entity<TextInput>,
    session: Option<Session>,
    placed: Mode,
    joined: bool,
    results: ResultList,
    active: bool,
    filling: Option<Task<()>>,
    arrival: Arrival,
    focus: Shared<Task<Focus>>,
    running: Shared<Task<Running>>,
    index: Arc<Index>,
    dismissed: Option<Instant>,
    card: Option<Entity<Card>>,
    menu: Option<tools::Menu>,
    recent: Option<Vec<Hit>>,
    apps_listed: Option<Instant>,
    searched: String,
    _subscriptions: Vec<Subscription>,
}

struct Session {
    mode: Mode,
    reveal: Option<Appear>,
    card_layout: Option<CardLayout>,
    prepared: Option<String>,
    came_from: Option<Host>,
    offered: Option<(Host, Vec<Hit>)>,
    trail: Trail,
}

impl Session {
    fn new(mode: Mode, came_from: Option<Host>) -> Self {
        Self {
            mode,
            reveal: None,
            card_layout: None,
            prepared: None,
            came_from,
            offered: None,
            trail: Trail::default(),
        }
    }
}

impl EventEmitter<Expand> for Launcher {}

impl Launcher {
    pub(crate) fn new(
        index: Arc<Index>,
        card: Option<Entity<Card>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(TextInput::new);
        let mut subscriptions = vec![
            ui::paint_changes(cx),
            ui::paint_changes_of(&input, cx),
            cx.observe_self(|this, cx| {
                this.sync_card(cx);
                this.prepare_menu();
            }),
            cx.subscribe(&input, |this, _, _: &Changed, cx| this.input_changed(cx)),
            cx.observe_window_activation(window, |this, window, cx| this.activation_changed(window, cx)),
            cx.observe_window_bounds(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Theme>(window, |this, window, cx| this.restyle(window, cx)),
            cx.observe_global_in::<Settings>(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global::<Language>(|this, cx| this.apply_placeholder(cx)),
            cx.observe_global::<Settings>(|this, cx| this.apply_placeholder(cx)),
            cx.observe_global::<History>(|this, cx| {
                if this.session.is_none() {
                    this.filling = Some(this.refresh_recent(cx));
                }
            }),
            cx.observe_global::<Status>(|_, cx| cx.notify()),
            cx.observe_global::<Icons>(|_, cx| cx.notify()),
        ];
        subscriptions.extend(ui::follow_menu(cx, |this, cx| {
            cx.notify();
            if this.joins_menu(cx) != this.joined {
                this.place(None, cx);
            }
        }));
        subscriptions.extend(card.as_ref().map(|card| cx.observe(card, |this, _, cx| this.sync_card(cx))));
        ui::fit_rem(window, cx);
        let mut launcher = Self {
            input,
            session: None,
            placed: Mode::Launcher,
            joined: false,
            results: ResultList::new(ui::pixel_ratio(window)),
            active: false,
            filling: None,
            arrival: Arrival::Fresh,
            focus: Task::ready(Focus::Programs).shared(),
            running: Task::ready(Running::default()).shared(),
            index,
            dismissed: None,
            card,
            menu: None,
            recent: None,
            apps_listed: None,
            searched: String::new(),
            _subscriptions: subscriptions,
        };
        let recent = cx.global::<History>().visits().into_iter().map(|visit| (visit.path, visit.dir)).collect();
        launcher.results.start_icons(view::icon_side(Mode::Launcher), recent, cx);
        launcher.list_apps(cx);
        launcher.find_running(cx);
        launcher.filling = Some(launcher.refresh_recent(cx));
        cx.background_spawn(async { platform::recent_items() }).detach();
        launcher.apply_placeholder(cx);
        launcher.restyle(window, cx);
        launcher
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.session.is_some() {
            self.run_query(cx);
        }
    }

    pub(crate) fn handle(&mut self, event: SearchEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SearchEvent::Toggle if self.mode() == Some(Mode::Launcher) => self.close(cx),
            SearchEvent::Summon if self.mode() == Some(Mode::Launcher) => {
                match cx.global::<Settings>().hotkeys.repeat {
                    Repeat::Expand => self.expand(cx),
                    Repeat::Close => self.close(cx),
                }
            }
            SearchEvent::Toggle | SearchEvent::Summon
                if self.dismissed.is_some_and(|at| at.elapsed() < TOGGLE_GRACE) => {}
            SearchEvent::Toggle | SearchEvent::Summon => {
                let opening = cx.global::<Settings>().opening;
                if opening == Opening::Empty {
                    self.clear_query(cx);
                }
                self.open(Mode::Launcher, Appear::Focused, window, cx);
                self.input.update(cx, |input, cx| match opening {
                    Opening::Selected => input.select_everything(cx),
                    Opening::Kept => input.caret_to_end(cx),
                    Opening::Empty => {}
                });
            }
            SearchEvent::Typed { host, text, first } => {
                if !first && self.host() == Some(host) {
                    self.input.update(cx, |input, cx| input.append(&text, cx));
                } else {
                    self.open(Mode::docked(host, cx), Appear::Focused, window, cx);
                    self.input.update(cx, |input, cx| input.set_text(&text, cx));
                }
            }
            SearchEvent::Attach { host, .. } if self.host() == Some(host) => {
                self.redock(cx);
                self.place(None, cx);
            }
            SearchEvent::Attach { host, explorer, picks } => {
                self.clear_query(cx);
                let appear = match picks {
                    Picks::Files | Picks::Folders => Appear::Focused,
                    Picks::Saves => Appear::Unfocused,
                };
                let shown = self.mode();
                self.session = Some(Session::new(Mode::docked(host, cx), explorer));
                self.reveal_open(shown, appear, window, cx);
            }
            SearchEvent::DialogKey { host, key } if self.host() == Some(host) => match key {
                DialogKey::Switch => self.switch_dialog(cx),
                DialogKey::Row(position) => self.confirm(position, false, cx),
            },
            SearchEvent::DialogKey { .. } => {}
            SearchEvent::Detach if self.host().is_some() => self.close(cx),
            SearchEvent::HostMoved if self.host().is_some() => {
                ui::close_search_menu(cx);
                if self.redock(cx) {
                    self.place(None, cx);
                }
            }
            SearchEvent::Detach | SearchEvent::HostMoved => {}
            SearchEvent::Moved(position) => {
                cx.update_global::<Settings, _>(|settings, _| settings.position = Some(position));
                self.place(None, cx);
            }
            SearchEvent::ResetPosition => {
                cx.update_global::<Settings, _>(|settings, _| settings.position = None);
                if self.mode() == Some(Mode::Launcher) {
                    self.place_at(Anchor::Center, None, cx);
                }
            }
        }
    }

    fn mode(&self) -> Option<Mode> {
        self.session.as_ref().map(|session| session.mode)
    }

    /// The look of the shown search window: corners, opacity, blur and shadow of its mode.
    pub(crate) fn chrome(&self, cx: &App) -> Option<Chrome> {
        self.mode().map(|mode| mode.chrome(cx.global::<Theme>()))
    }

    fn open(&mut self, mode: Mode, appear: Appear, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self.mode();
        match &mut self.session {
            Some(session) => session.mode = mode,
            None => self.session = Some(Session::new(mode, None)),
        }
        self.reveal_open(shown, appear, window, cx);
    }

    fn reveal_open(&mut self, shown: Option<Mode>, appear: Appear, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut() else { return };
        session.reveal = Some(appear);
        if shown.is_some_and(|shown| shown != session.mode) {
            cx.spawn(async move |_, _| platform::hide()).detach();
        }
        self.restore_menu(cx);
        self.find_focus(cx);
        let under = cx.global::<Theme>().launcher.background;
        self.input.update(cx, |input, cx| input.set_under(under, cx));
        ui::publish_status(&self.index, cx);
        self.apply_placeholder(cx);
        if appear.activates() {
            window.focus(&self.input.focus_handle(cx), cx);
            self.input.update(cx, |input, cx| input.set_blinking(true, cx));
        }
        self.query_changed(cx);
    }

    fn find_focus(&mut self, cx: &mut Context<Self>) {
        let task = match self.host() {
            None => Task::ready(Focus::Programs),
            Some(host) => {
                cx.background_spawn(async move { platform::host_folder(host).map_or(Focus::Anything, Focus::Folder) })
            }
        };
        self.focus = task.shared();
        self.find_running(cx);
    }

    fn list_apps(&mut self, cx: &mut Context<Self>) {
        if self.apps_listed.is_some_and(|at| at.elapsed() < APPS_INTERVAL) {
            return;
        }
        self.apps_listed = Some(Instant::now());
        let index = self.index.clone();
        cx.background_spawn(async move { index.list_apps(platform::store_apps()) }).detach();
    }

    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        if self.session.take().is_some() {
            ui::close_search_menu(cx);
            self.filling = Some(self.refresh_recent(cx));
            self.restore_menu(cx);
            self.input.update(cx, |input, cx| input.set_blinking(false, cx));
            cx.spawn(async move |_, _| platform::hide()).detach();
        }
    }

    fn expand(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().to_owned();
        cx.emit(Expand::Search(query));
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        cx.emit(Expand::Settings);
    }

    fn host(&self) -> Option<Host> {
        self.mode().and_then(Mode::host)
    }

    fn redock(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(host) = self.host() else { return false };
        let mode = Mode::docked(host, cx);
        let Some(session) = self.session.as_mut().filter(|session| session.mode != mode) else { return false };
        session.mode = mode;
        cx.notify();
        true
    }

    fn place(&mut self, show: Option<Appear>, cx: &mut Context<Self>) {
        let Some(mode) = self.mode() else { return };
        let anchor = match (mode, show) {
            (Mode::Launcher, None) => Anchor::Current,
            (Mode::Launcher, Some(_)) | (Mode::Dialog(..) | Mode::Explorer(_), _) => {
                mode.anchor(cx.global::<Settings>())
            }
        };
        self.place_at(anchor, show, cx);
    }

    fn place_at(&mut self, anchor: Anchor, show: Option<Appear>, cx: &mut Context<Self>) {
        let Some(mode) = self.mode() else { return };
        self.results.forget_pointer();
        self.joined = self.joins_menu(cx);
        let row = view::row_height(mode, self.results.listing());
        let footer = self.shows_footer(mode, cx);
        let chrome = view::bar_chrome(mode, self.joined, cx.global::<Theme>());
        let layout = view::layout(mode, self.rows(mode), row, footer, chrome, ui::zoom(cx));
        self.placed = mode;
        platform::note_placing();
        cx.spawn(async move |this, cx| {
            platform::place(anchor, layout);
            if let Some(appear) = show
                && !platform::show(appear.activates(), mode.entrance())
                && mode.host().is_none()
            {
                log::warn!("Windows kept the search window in the background, hiding it");
                let _ = this.update(cx, |this, cx| this.close(cx));
            }
        })
        .detach();
    }

    fn activation_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = window.is_window_active();
        self.active = active;
        self.input.update(cx, |input, cx| input.set_blinking(active, cx));
        if active {
            window.focus(&self.input.focus_handle(cx), cx);
            if matches!(self.mode(), Some(Mode::Dialog(..))) {
                self.find_focus(cx);
            }
        }
        match self.mode() {
            Some(Mode::Launcher) if !active => {
                self.dismissed = Some(Instant::now());
                self.close(cx);
            }
            Some(Mode::Explorer(_)) if !active => self.close(cx),
            Some(Mode::Dialog(..)) if self.input.read(cx).text().is_empty() => self.query_changed(cx),
            Some(Mode::Launcher | Mode::Dialog(..) | Mode::Explorer(_)) | None => {}
        }
    }

    fn input_changed(&mut self, cx: &mut Context<Self>) {
        if self.menu.is_some() || self.input.read(cx).text() != self.searched {
            self.query_changed(cx);
        }
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.arrival = Arrival::Fresh;
        self.run_query(cx);
    }

    fn run_query(&mut self, cx: &mut Context<Self>) {
        if self.filter_actions(cx) {
            return;
        }
        let text = self.input.read(cx).text();
        let again = text == self.searched;
        self.searched = text.to_owned();
        let Some(mut query) = Query::parse(text, &ui::grammar(cx)) else {
            self.suggest(cx);
            return;
        };
        if let Some(row) = self.results.wanted() {
            query.keep(row);
        }
        let place = self.host().map(|host| host.kind);
        let (before, after) = ui::extras(text, place, cx.global::<Settings>(), *cx.global::<Language>());
        self.results.start_search();
        let kept = again && self.reveal_kept(&before, &after, cx);
        let index = self.index.clone();
        let boosts = cx.global::<History>().boosts();
        let focus = self.focus.clone();
        let running = self.running.clone();
        let work = move |token: Token| async move {
            let focus = focus.await;
            let boosts = with_running(boosts, &running.await);
            index.search_until(&query, &boosts, &focus, RESULT_LIMIT, &Facets::default(), &token)
        };
        let found = move |this: &mut Self, (results, _): (Results, Counts), cx: &mut Context<Self>| {
            let Results { hits, total, listed } = results;
            let listing = Listing::of(listed, total);
            if !(kept && this.results.shows(&before, &hits, &after, listing)) {
                this.show(before, hits, after, listing, cx);
            }
        };
        self.filling = Some(ui::cancellable(cx, work, found));
    }

    fn reveal_kept(&mut self, before: &[Extra], after: &[Extra], cx: &mut Context<Self>) -> bool {
        let listing = self.results.listing();
        let found = matches!(listing, Listing::Found(_) | Listing::AtLeast(_) | Listing::Contents(_));
        let reopened = self.mode() == Some(Mode::Launcher) && self.placed == Mode::Launcher;
        let revealing = self.session.as_ref().is_some_and(|session| session.reveal.is_some());
        if !revealing || !reopened || !found {
            return false;
        }
        let hits = self.results.hits().to_vec();
        self.show(before.to_vec(), hits, after.to_vec(), listing, cx);
        true
    }

    fn show(
        &mut self,
        before: Vec<Extra>,
        hits: Vec<Hit>,
        after: Vec<Extra>,
        listing: Listing,
        cx: &mut Context<Self>,
    ) {
        let shape = self.shape(cx);
        self.results.land(self.arrival, before, hits, after, listing);
        self.arrival = Arrival::Rerun;
        self.request_icons(cx);
        match self.session.as_mut().and_then(|session| session.reveal.take()) {
            Some(appear) => {
                self.place(Some(appear), cx);
                cx.notify();
                if self.mode() == Some(Mode::Launcher) {
                    self.list_apps(cx);
                }
            }
            None => self.reshape(shape, cx),
        }
    }

    fn shape(&self, cx: &App) -> (usize, f32, bool) {
        let mode = self.mode().unwrap_or(self.placed);
        (self.rows(mode), view::row_height(mode, self.results.listing()), self.shows_footer(mode, cx))
    }

    fn reshape(&mut self, shape: (usize, f32, bool), cx: &mut Context<Self>) {
        if self.shape(cx) != shape {
            self.place(None, cx);
        }
        cx.notify();
    }

    fn shows_footer(&self, mode: Mode, cx: &App) -> bool {
        if self.joined {
            return false;
        }
        let typed = match self.results.listing() {
            Listing::Found(_)
            | Listing::AtLeast(_)
            | Listing::Contents(_)
            | Listing::ContentOff
            | Listing::ContentShort
            | Listing::Actions => true,
            Listing::Recent | Listing::Folders | Listing::Searching => false,
        };
        let settings = cx.global::<Settings>();
        match mode {
            Mode::Launcher => typed || settings.idle_status,
            Mode::Dialog(..) | Mode::Explorer(_) => settings.path_crumbs && !self.results.hits().is_empty(),
        }
    }

    fn rows(&self, mode: Mode) -> usize {
        if self.joined {
            return 0;
        }
        let rows = match (self.results.len(), self.results.listing()) {
            (
                0,
                Listing::Found(_)
                | Listing::AtLeast(_)
                | Listing::Contents(_)
                | Listing::ContentOff
                | Listing::ContentShort
                | Listing::Actions,
            ) => 1,
            (count, _) => count,
        };
        rows.min(mode.max_rows())
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.results.select_next();
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.results.select_previous();
        cx.notify();
    }

    fn point_at(&mut self, position: usize, at: Point<Pixels>, origin: Point<Pixels>, cx: &mut Context<Self>) {
        if self.results.point_at(position, at, origin) {
            cx.notify();
        }
    }

    fn pick_next(&mut self, _: &PickNext, _: &mut Window, cx: &mut Context<Self>) {
        self.results.pick_step(1);
        cx.notify();
    }

    fn pick_previous(&mut self, _: &PickPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.results.pick_step(-1);
        cx.notify();
    }

    fn click(&mut self, position: usize, event: &ClickEvent, cx: &mut Context<Self>) {
        if self.results.pick_click(position, event) {
            cx.notify();
        } else {
            self.confirm(position, false, cx);
        }
    }

    fn confirm_selected(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm(self.results.selected_position(), false, cx);
    }

    fn reveal_selected(&mut self, _: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm(self.results.selected_position(), true, cx);
    }

    fn confirm(&mut self, position: usize, reveal: bool, cx: &mut Context<Self>) {
        match self.results.entry(position) {
            Some(Entry::File(hit)) => {
                let hit = hit.clone();
                self.launch(hit, reveal, cx);
            }
            Some(Entry::Extra(extra)) => {
                let extra = extra.clone();
                self.run_tool(extra, cx);
            }
            Some(Entry::Heading(_)) | None => {}
        }
    }

    fn launch(&mut self, hit: Hit, reveal: bool, cx: &mut Context<Self>) {
        let Some(mode) = self.mode() else { return };
        cx.update_global::<History, _>(|history, _| history.record(&hit.path, hit.dir));
        let running = self.running_app(&hit.path);
        let command = mode.command(hit, reveal, running);
        self.finish(&command, cx);
        ui::perform(command, cx);
    }

    fn finish(&mut self, command: &Command, cx: &mut Context<Self>) {
        match command {
            Command::Choose { .. } | Command::Focus(_) => self.clear_query(cx),
            Command::Drag(_) => {}
            Command::Open(_)
            | Command::Switch { .. }
            | Command::Reveal(_)
            | Command::RunAsAdmin(_)
            | Command::Properties(_)
            | Command::Navigate(..)
            | Command::OpenWith(_)
            | Command::PutFiles { .. }
            | Command::Recycle(_)
            | Command::Run(_)
            | Command::Create { .. } => self.close(cx),
        }
    }

    fn forget_selected(&mut self, _: &Forget, _: &mut Window, cx: &mut Context<Self>) {
        self.forget(self.results.selected_position(), cx);
    }

    fn forget(&mut self, position: usize, cx: &mut Context<Self>) {
        if self.results.listing() != Listing::Recent {
            return;
        }
        let shape = self.shape(cx);
        let Some(hit) = self.results.remove(position) else { return };
        cx.update_global::<History, _>(|history, _| history.forget(&hit.path));
        cx.update_global::<Settings, _>(|settings, _| {
            let hidden = &mut settings.suggestions.hidden;
            if !hidden.iter().any(|known| known.eq_ignore_ascii_case(&hit.path)) {
                hidden.push(hit.path);
            }
        });
        self.reshape(shape, cx);
    }

    fn quick_switch(&mut self, _: &QuickSwitch, _: &mut Window, cx: &mut Context<Self>) {
        self.switch_dialog(cx);
    }

    fn switch_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(Mode::Dialog(host, ..)) = self.mode() else { return };
        cx.spawn(async move |this, cx| {
            let folder = cx.background_spawn(async move { platform::explorer_folders().into_iter().next() }).await;
            let Some(path) = folder else { return };
            let _ = this.update(cx, |this, cx| {
                let command = Command::Choose { host, path, keep_name: true };
                this.finish(&command, cx);
                ui::perform(command, cx);
            });
        })
        .detach();
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.step_back(cx) {
            return;
        }
        if self.results.unpick() {
            cx.notify();
            return;
        }
        match self.mode() {
            Some(Mode::Explorer(host)) => {
                self.close(cx);
                ui::perform(Command::Focus(host), cx);
            }
            Some(Mode::Dialog(host, ..)) => {
                self.clear_query(cx);
                ui::perform(Command::Focus(host), cx);
            }
            Some(Mode::Launcher) | None => self.close(cx),
        }
    }

    fn rescale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let zoomed = ui::fit_rem(window, cx);
        if self.results.rescale(ui::pixel_ratio(window)) || zoomed {
            self.restyle(window, cx);
        }
    }

    fn restyle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ui::fit_rem(window, cx);
        let under = cx.global::<Theme>().launcher.background;
        self.input.update(cx, |input, cx| input.set_under(under, cx));
        self.results.rescale(ui::pixel_ratio(window));
        self.request_icons(cx);
        self.place(None, cx);
        cx.notify();
    }

    fn request_icons(&mut self, cx: &mut Context<Self>) {
        let side = view::icon_side(self.mode().unwrap_or(Mode::Launcher));
        self.results.request_icons(side, cx);
    }

    fn apply_placeholder(&mut self, cx: &mut Context<Self>) {
        let strings = cx.global::<Language>().strings();
        let placeholder = match (&self.menu, self.mode()) {
            (Some(_), _) => strings.placeholder_actions,
            (None, Some(Mode::Dialog(..) | Mode::Explorer(_))) => strings.placeholder_docked,
            (None, Some(Mode::Launcher) | None) => ui::search_placeholder(cx),
        };
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder);
            cx.notify();
        });
    }
}
