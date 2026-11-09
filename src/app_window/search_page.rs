mod actions;
mod navigation;
mod table;
mod view;

pub(super) use view::ICON;

use super::menus::Menus;
use super::settings_page::{Menu, Renumber};
use crate::index::{Counts, Facets, Focus, Hit, Index, Results, Token};
use crate::input::{Changed, TextInput};
use crate::platform;
use crate::search::{Filter, Period, Query, Rule};
use crate::settings::{History, Settings};
use crate::theme::Theme;
use crate::ui::{
    self, Arrival, Command, Confirm, Dismiss, Listing, PickNext, PickPrevious, Preview, ResultList, Reveal, SelectNext,
    SelectPrevious, Sources, TogglePreview, Trail,
};
use gpui::{
    App, AppContext, BorrowAppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    Point, Subscription, Task, Window,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use table::{Column, DetailMap, Sort, arrange};

pub(super) const RESULT_LIMIT: usize = 400;

pub(super) struct Presented;

pub(super) struct SearchPage {
    input: Entity<TextInput>,
    filters: BTreeSet<usize>,
    date: Option<Period>,
    counts: Option<Counts>,
    preview: Preview,
    results: ResultList,
    details: DetailMap,
    sort: Option<Sort>,
    menus: Menus<Menu>,
    search: Option<Task<()>>,
    arrival: Arrival,
    trail: Trail,
    index: Arc<Index>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Presented> for SearchPage {}

impl Focusable for SearchPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl SearchPage {
    pub(super) fn new(index: Arc<Index>, ratio: f32, cx: &mut Context<Self>) -> Self {
        let input = cx.new(TextInput::new);
        let subscriptions = vec![
            ui::paint_changes(cx),
            ui::paint_changes_of(&input, cx),
            cx.subscribe(&input, |this, _, _: &Changed, cx| this.query_changed(cx)),
        ];
        Self {
            input,
            filters: BTreeSet::new(),
            date: None,
            counts: None,
            preview: Preview::default(),
            results: ResultList::new(ratio),
            details: DetailMap::default(),
            sort: None,
            menus: Menus::default(),
            search: None,
            arrival: Arrival::Fresh,
            trail: Trail::default(),
            index,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn filters(&self) -> &BTreeSet<usize> {
        &self.filters
    }

    pub(super) fn counts(&self) -> Option<&Counts> {
        self.counts.as_ref()
    }

    pub(super) fn show(&mut self, query: Option<&str>, select: bool, cx: &mut Context<Self>) {
        match query {
            Some(query) => self.input.update(cx, |input, cx| input.set_text(query, cx)),
            None if select => self.input.update(cx, |input, cx| input.select_everything(cx)),
            None => {}
        }
        self.query_changed(cx);
    }

    pub(super) fn set_blinking(&self, active: bool, cx: &mut App) {
        self.input.update(cx, |input, cx| input.set_blinking(active, cx));
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menus.close() {
            cx.notify();
        }
    }

    pub(super) fn suggestions_changed(&mut self, cx: &mut Context<Self>) {
        if self.results.listing() == Listing::Recent {
            self.query_changed(cx);
        }
        cx.notify();
    }

    pub(super) fn filters_changed(&mut self, cx: &mut Context<Self>) {
        let filters = &cx.global::<Settings>().filters;
        let chosen = self.filters.len();
        self.filters.retain(|&position| filters.get(position).is_some_and(|filter| filter.enabled));
        if self.filters.len() != chosen {
            self.query_changed(cx);
        }
    }

    pub(super) fn renumber_filter(&mut self, renumber: Renumber, cx: &mut Context<Self>) {
        let chosen = self.filters.len();
        self.filters = self.filters.iter().filter_map(|&position| renumber.apply(position)).collect();
        if self.filters.len() != chosen {
            self.query_changed(cx);
        }
    }

    pub(super) fn choose_filter(&mut self, filter: Option<usize>, add: bool, cx: &mut Context<Self>) {
        let mut filters = self.filters.clone();
        match (filter, add) {
            (None, _) => filters.clear(),
            (Some(position), false) => filters = BTreeSet::from([position]),
            (Some(position), true) => {
                if !filters.remove(&position) {
                    filters.insert(position);
                }
            }
        }
        if filters != self.filters {
            self.filters = filters;
            self.query_changed(cx);
        }
    }

    fn choose_date(&mut self, date: Option<Period>, cx: &mut Context<Self>) {
        if self.date != date {
            self.date = date;
            self.query_changed(cx);
        }
    }

    fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some(sort) if sort.column == column && !sort.descending => Some(Sort { column, descending: true }),
            Some(sort) if sort.column == column => None,
            Some(_) | None => Some(Sort { column, descending: false }),
        };
        self.query_changed(cx);
    }

    pub(super) fn query_changed(&mut self, cx: &mut Context<Self>) {
        self.arrival = Arrival::Fresh;
        self.rerun(cx);
    }

    pub(super) fn rerun(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().to_owned();
        let sort = self.sort;
        let (facets, scope) = self.facets(cx);
        let scope = (facets.chosen != 0 || facets.date.is_some()).then_some(scope);
        let Some(mut query) = Query::parse_with(&text, &ui::grammar(cx), scope) else {
            let sources = Sources::of(cx);
            self.search = Some(cx.spawn(async move |this, cx| {
                let (hits, details) = cx
                    .background_spawn(async move {
                        let hits = sources.map(|sources| sources.collect(RESULT_LIMIT)).unwrap_or_default();
                        arrange(hits, sort)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.counts = None;
                    this.present(hits, details, Listing::Recent, cx)
                });
            }));
            return;
        };
        if let Some(row) = self.results.wanted() {
            query.keep(row);
        }
        self.results.start_search();
        let index = self.index.clone();
        let boosts = cx.global::<History>().boosts();
        let work = move |token: Token| async move {
            let (Results { hits, total, listed }, counts) =
                index.search_until(&query, &boosts, &Focus::Programs, RESULT_LIMIT, &facets, &token)?;
            let (hits, details) = arrange(hits, sort);
            Some((hits, details, Listing::of(listed, total), counts))
        };
        let found = |this: &mut Self, (hits, details, listing, counts), cx: &mut Context<Self>| {
            this.counts = Some(counts);
            this.present(hits, details, listing, cx)
        };
        self.search = Some(ui::cancellable(cx, work, found));
    }

    fn facets(&self, cx: &App) -> (Facets, Rule) {
        let filters = &cx.global::<Settings>().filters;
        let enabled: Vec<(usize, &Filter)> = filters.iter().enumerate().filter(|(_, filter)| filter.enabled).collect();
        let chosen: Vec<usize> = enabled
            .iter()
            .enumerate()
            .filter(|(_, (known, _))| self.filters.contains(known))
            .map(|(kind, _)| kind)
            .collect();
        let mut scopes = chosen.iter().filter_map(|&kind| enabled.get(kind)).map(|(_, filter)| filter.rule().scope());
        let scope = match scopes.next() {
            Some(first) if scopes.all(|other| other == first) => first,
            Some(_) | None => Rule::default(),
        };
        let clock = platform::clock();
        let facets = Facets {
            kinds: enabled.iter().map(|(_, filter)| filter.rule()).collect(),
            dates: Period::ALL.into_iter().map(|period| period.range(clock)).collect(),
            chosen: Facets::mask(chosen),
            date: self.date.and_then(|date| Period::ALL.into_iter().position(|period| period == date)),
        };
        (facets, scope)
    }

    fn present(&mut self, hits: Vec<Hit>, details: DetailMap, listing: Listing, cx: &mut Context<Self>) {
        self.results.land(self.arrival, Vec::new(), hits, Vec::new(), listing);
        self.arrival = Arrival::Rerun;
        self.details = details;
        self.sync_preview(cx);
        self.request_icons(cx);
        cx.emit(Presented);
        cx.notify();
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.results.select_next();
        self.sync_preview(cx);
        cx.notify();
    }

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.results.select_previous();
        self.sync_preview(cx);
        cx.notify();
    }

    fn pick_next(&mut self, _: &PickNext, _: &mut Window, cx: &mut Context<Self>) {
        self.results.pick_step(1);
        self.sync_preview(cx);
        cx.notify();
    }

    fn pick_previous(&mut self, _: &PickPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.results.pick_step(-1);
        self.sync_preview(cx);
        cx.notify();
    }

    fn click(&mut self, position: usize, event: &ClickEvent, cx: &mut Context<Self>) {
        if self.results.pick_click(position, event) {
            self.sync_preview(cx);
            cx.notify();
        } else {
            self.open_at(position, cx);
        }
    }

    fn point_at(&mut self, position: usize, at: Point<Pixels>, origin: Point<Pixels>, cx: &mut Context<Self>) {
        if self.results.point_at(position, at, origin) {
            self.sync_preview(cx);
            cx.notify();
        }
    }

    fn toggle_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        cx.update_global::<Settings, _>(|settings, _| settings.preview.window = !settings.preview.window);
        self.sync_preview(cx);
        cx.notify();
    }

    fn shows_preview(cx: &App) -> bool {
        cx.global::<Settings>().preview.window
    }

    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let shows = Self::shows_preview(cx);
        let hit = self.results.selected().filter(|_| shows).cloned();
        let around = if shows { self.results.around_selected() } else { Vec::new() };
        self.preview.show(hit.as_ref(), around, self.results.ratio(), |this: &mut Self| &mut this.preview, cx);
    }

    fn open_at(&mut self, position: usize, cx: &mut Context<Self>) {
        self.results.select(position);
        self.launch(self.results.selected().cloned(), Command::Open, cx);
    }

    fn confirm_selected(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        self.launch(self.results.selected().cloned(), Command::Open, cx);
    }

    fn reveal_selected(&mut self, _: &Reveal, _: &mut Window, cx: &mut Context<Self>) {
        self.launch(self.results.selected().cloned(), Command::Reveal, cx);
    }

    fn launch(&mut self, hit: Option<Hit>, command: fn(String) -> Command, cx: &mut Context<Self>) {
        let Some(hit) = hit else { return };
        cx.update_global::<History, _>(|history, _| history.record(&hit.path, hit.dir));
        ui::perform(command(hit.path), cx);
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if self.menus.close() || self.results.unpick() {
            cx.notify();
        } else if !self.input.read(cx).text().is_empty() {
            self.input.update(cx, |input, cx| input.set_text("", cx));
        } else {
            cx.propagate();
        }
    }

    pub(super) fn rescale(&mut self, ratio: f32) -> bool {
        self.results.rescale(ratio)
    }

    pub(super) fn restyle(&mut self, ratio: f32, theme: &Theme, cx: &mut Context<Self>) {
        let under = theme.app.background;
        self.input.update(cx, |input, cx| input.set_under(under, cx));
        self.results.rescale(ratio);
        self.request_icons(cx);
        self.sync_preview(cx);
        cx.notify();
    }

    fn request_icons(&mut self, cx: &mut Context<Self>) {
        self.results.request_icons(ICON, cx);
    }

    pub(super) fn apply_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = ui::search_placeholder(cx);
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder);
            cx.notify();
        });
    }
}
