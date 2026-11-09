use super::icons::{self, IconKey, Icons};
use super::scroll::Reveal;
use super::{CTRL, Command, HINT_GAP, Key, glyph, hint_chip, perform};
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::index::{ContentStatus, Hit, Status};
use crate::platform::{self, IconSource};
use crate::search::Listed;
use crate::settings::{BuiltinAction, Settings};
use crate::theme::{Theme, rem};
use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, MouseButton, MouseMoveEvent, Pixels, Point, ScrollStrategy,
    UniformListScrollHandle, Window, div, img, prelude::*,
};
use std::collections::BTreeSet;

const PERCENT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Listing {
    Recent,
    Folders,
    Searching,
    Found(usize),
    AtLeast(usize),
    Contents(usize),
    ContentOff,
    ContentShort,
    Actions,
}

impl Listing {
    pub(crate) fn of(listed: Listed, total: usize) -> Self {
        match listed {
            Listed::Matches => Self::Found(total),
            Listed::AtLeast => Self::AtLeast(total),
            Listed::Contents => Self::Contents(total),
            Listed::ContentOff => Self::ContentOff,
            Listed::ContentShort => Self::ContentShort,
        }
    }

    pub(crate) fn listed(self) -> Listed {
        match self {
            Self::Contents(_) => Listed::Contents,
            Self::AtLeast(_) => Listed::AtLeast,
            Self::ContentOff => Listed::ContentOff,
            Self::ContentShort => Listed::ContentShort,
            Self::Recent | Self::Folders | Self::Searching | Self::Found(_) | Self::Actions => Listed::Matches,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    First,
    Last,
    HalfPageUp,
    HalfPageDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Command(usize),
    Web(usize),
    Action { index: usize, dir: bool, builtin: Option<BuiltinAction> },
    Shell { index: usize, submenu: bool },
}

impl Tool {
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Self::Command(_) => assets::COMMAND,
            Self::Shell { .. } => assets::CONTEXT_MENU,
            Self::Web(_) => assets::GLOBE,
            Self::Action { builtin, .. } => match builtin {
                Some(BuiltinAction::Open) => assets::OPEN,
                Some(BuiltinAction::OpenWith) => assets::OPEN_WITH,
                Some(BuiltinAction::Reveal) => assets::REVEAL,
                Some(BuiltinAction::RunAsAdmin) => assets::ADMIN,
                Some(BuiltinAction::Copy) => assets::COPY,
                Some(BuiltinAction::Cut) => assets::CUT,
                Some(BuiltinAction::CopyPath) => assets::COPY_PATH,
                Some(BuiltinAction::CopyName) => assets::COPY_NAME,
                Some(BuiltinAction::Delete) => assets::DELETE,
                Some(BuiltinAction::Properties) => assets::PROPERTIES,
                Some(BuiltinAction::ContextMenu) => assets::CONTEXT_MENU,
                None => assets::ACTION,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Extra {
    pub(crate) tool: Tool,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) argument: String,
}

#[derive(Clone, Copy)]
pub(crate) enum Entry<'a> {
    File(&'a Hit),
    Extra(&'a Extra),
    Heading(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    Fresh,
    Rerun,
}

enum Row {
    File(String),
    Extra(Extra),
}

#[derive(Clone)]
struct Heading {
    at: usize,
    title: String,
}

pub(crate) struct Shown {
    hits: Vec<Hit>,
    before: Vec<Extra>,
    after: Vec<Extra>,
    heading: Option<Heading>,
    listing: Listing,
    selected: usize,
    picked: BTreeSet<usize>,
    anchor: usize,
    offset: Point<Pixels>,
}

pub(crate) struct ResultList {
    hits: Vec<Hit>,
    before: Vec<Extra>,
    after: Vec<Extra>,
    heading: Option<Heading>,
    listing: Listing,
    selected: usize,
    picked: BTreeSet<usize>,
    anchor: usize,
    reselect: Option<String>,
    notice: Option<String>,
    reveal: Option<Reveal>,
    pointer: Option<Point<Pixels>>,
    pressed: Option<(usize, Point<Pixels>)>,
    scroll: UniformListScrollHandle,
    ratio: f32,
}

impl ResultList {
    pub(crate) fn new(ratio: f32) -> Self {
        Self {
            hits: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            heading: None,
            listing: Listing::Recent,
            selected: 0,
            picked: BTreeSet::new(),
            anchor: 0,
            reselect: None,
            notice: None,
            reveal: Some(Reveal::WholeRows),
            pointer: None,
            pressed: None,
            scroll: UniformListScrollHandle::new(),
            ratio,
        }
    }

    pub(crate) fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub(crate) fn len(&self) -> usize {
        self.before.len() + self.hits.len() + self.after.len() + usize::from(self.heading.is_some())
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn entry(&self, position: usize) -> Option<Entry<'_>> {
        let position = match &self.heading {
            Some(heading) if heading.at == position => return Some(Entry::Heading(&heading.title)),
            Some(heading) if heading.at < position => position - 1,
            Some(_) | None => position,
        };
        let after = position.checked_sub(self.before.len()).map(|position| position.checked_sub(self.hits.len()));
        match after {
            None => self.before.get(position).map(Entry::Extra),
            Some(None) => self.hits.get(position - self.before.len()).map(Entry::File),
            Some(Some(position)) => self.after.get(position).map(Entry::Extra),
        }
    }

    pub(crate) fn selected_entry(&self) -> Option<Entry<'_>> {
        self.entry(self.selected)
    }

    pub(crate) fn hit(&self, position: usize) -> Option<&Hit> {
        match self.entry(position)? {
            Entry::File(hit) => Some(hit),
            Entry::Extra(_) | Entry::Heading(_) => None,
        }
    }

    pub(crate) fn listing(&self) -> Listing {
        self.listing
    }

    pub(crate) fn scroll(&self) -> &UniformListScrollHandle {
        &self.scroll
    }

    pub(crate) fn glide(&self, row: f32) -> impl IntoElement + use<> {
        super::glide(
            "results",
            self.scroll.0.borrow().base_handle.clone(),
            self.reveal.map(|how| (self.selected, row, how)),
        )
    }

    pub(crate) fn is_selected(&self, position: usize) -> bool {
        position == self.selected
    }

    pub(crate) fn selected_position(&self) -> usize {
        self.selected
    }

    pub(crate) fn selected(&self) -> Option<&Hit> {
        self.hit(self.selected)
    }

    pub(crate) fn around_selected(&self) -> Vec<Hit> {
        let positions = [self.selected.checked_add(1), self.selected.checked_sub(1)];
        positions.into_iter().flatten().filter_map(|position| self.hit(position)).cloned().collect()
    }

    pub(crate) fn start_search(&mut self) {
        if matches!(self.listing, Listing::Recent | Listing::Folders) {
            self.listing = Listing::Searching;
        }
    }

    #[cfg(test)]
    pub(crate) fn present(&mut self, hits: Vec<Hit>, listing: Listing) {
        self.present_with(Vec::new(), hits, Vec::new(), listing);
    }

    pub(crate) fn present_with(&mut self, before: Vec<Extra>, hits: Vec<Hit>, after: Vec<Extra>, listing: Listing) {
        self.before = before;
        self.after = after;
        self.hits = hits;
        self.heading = None;
        self.listing = listing;
        self.selected = 0;
        self.picked.clear();
        self.anchor = 0;
        self.notice = None;
        self.pressed = None;
        let row = self.reselect.take();
        let found = self.hits.iter().position(|hit| row.as_ref().is_some_and(|row| hit.path.eq_ignore_ascii_case(row)));
        match found {
            Some(position) => {
                self.select(self.before.len() + position);
                self.scroll.scroll_to_item(self.selected, ScrollStrategy::Center);
            }
            None => self.scroll.scroll_to_item(0, ScrollStrategy::Top),
        }
    }

    pub(crate) fn land(
        &mut self,
        arrival: Arrival,
        before: Vec<Extra>,
        hits: Vec<Hit>,
        after: Vec<Extra>,
        listing: Listing,
    ) {
        match arrival {
            Arrival::Fresh => self.present_with(before, hits, after, listing),
            Arrival::Rerun => self.rerun_with(before, hits, after, listing),
        }
    }

    fn rerun_with(&mut self, before: Vec<Extra>, hits: Vec<Hit>, after: Vec<Extra>, listing: Listing) {
        let selected = self.row(self.selected);
        let anchor = self.row(self.anchor);
        let picked: Vec<Row> = self.picked.iter().filter_map(|&position| self.row(position)).collect();
        let pressed = self.pressed.and_then(|(position, at)| Some((self.row(position)?, at)));
        (self.before, self.hits, self.after, self.heading, self.listing) = (before, hits, after, None, listing);
        let last = self.len().saturating_sub(1);
        self.selected = selected.and_then(|row| self.find(&row)).unwrap_or(self.selected.min(last));
        self.picked = picked.iter().filter_map(|row| self.find(row)).collect();
        self.anchor = anchor.and_then(|row| self.find(&row)).unwrap_or(self.selected);
        self.pressed = pressed.and_then(|(row, at)| Some((self.find(&row)?, at)));
    }

    fn row(&self, position: usize) -> Option<Row> {
        match self.entry(position)? {
            Entry::File(hit) => Some(Row::File(hit.path.clone())),
            Entry::Extra(extra) => Some(Row::Extra(extra.clone())),
            Entry::Heading(_) => None,
        }
    }

    fn find(&self, row: &Row) -> Option<usize> {
        match row {
            Row::File(path) => {
                self.hits.iter().position(|hit| hit.path == *path).map(|position| self.before.len() + position)
            }
            Row::Extra(extra) => self.before.iter().position(|before| before == extra).or_else(|| {
                let position = self.after.iter().position(|after| after == extra)?;
                Some(self.before.len() + self.hits.len() + position)
            }),
        }
    }

    pub(crate) fn shows(&self, before: &[Extra], hits: &[Hit], after: &[Extra], listing: Listing) -> bool {
        self.listing == listing
            && self.heading.is_none()
            && self.hits == hits
            && self.before == before
            && self.after == after
    }

    pub(crate) fn wanted(&self) -> Option<&str> {
        self.reselect.as_deref()
    }

    pub(crate) fn reselect(&mut self, row: Option<String>) {
        self.reselect = row;
    }

    pub(crate) fn announce(&mut self, notice: String) {
        self.notice = Some(notice);
    }

    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(crate) fn head(&mut self, at: usize, title: String) {
        self.heading = Some(Heading { at, title });
        if !self.selectable(self.selected) {
            self.step(1);
        }
    }

    fn selectable(&self, position: usize) -> bool {
        position < self.len() && self.heading.as_ref().is_none_or(|heading| heading.at != position)
    }

    pub(crate) fn shown(&self) -> Shown {
        Shown {
            hits: self.hits.clone(),
            before: self.before.clone(),
            after: self.after.clone(),
            heading: self.heading.clone(),
            listing: self.listing,
            selected: self.selected,
            picked: self.picked.clone(),
            anchor: self.anchor,
            offset: self.scroll.0.borrow().base_handle.offset(),
        }
    }

    pub(crate) fn show_again(&mut self, shown: Shown) {
        let Shown { hits, before, after, heading, listing, selected, picked, anchor, offset } = shown;
        (self.hits, self.before, self.after, self.heading) = (hits, before, after, heading);
        (self.listing, self.selected, self.reveal) = (listing, selected, None);
        (self.picked, self.anchor) = (picked, anchor);
        let mut scroll = self.scroll.0.borrow_mut();
        scroll.deferred_scroll_to_item = None;
        scroll.base_handle.set_offset(offset);
    }

    pub(crate) fn select(&mut self, position: usize) {
        let Some(last) = self.len().checked_sub(1) else { return };
        self.selected = position.min(last);
        self.reveal = Some(Reveal::WholeRows);
        if self.picked.is_empty() {
            self.anchor = self.selected;
        }
    }

    pub(crate) fn select_next(&mut self) {
        self.step(1);
    }

    pub(crate) fn select_previous(&mut self) {
        self.step(-1);
    }

    pub(crate) fn jump(&mut self, motion: Motion) {
        let Some(last) = self.len().checked_sub(1) else { return };
        let half = self.half_page() as isize;
        let (target, toward) = match motion {
            Motion::First => (0, 1),
            Motion::Last => (last, -1),
            Motion::HalfPageUp => (self.selected.saturating_sub(half.unsigned_abs()), -1),
            Motion::HalfPageDown => (self.selected.saturating_add_signed(half).min(last), 1),
        };
        self.select(target);
        if !self.selectable(target) {
            self.step(toward);
        }
    }

    fn half_page(&self) -> usize {
        let shown = self.scroll.0.borrow().last_item_size.map_or(0.0, |size| {
            let row = f32::from(size.contents.height) / self.len().max(1) as f32;
            if row > 0.0 { f32::from(size.item.height) / row } else { 0.0 }
        });
        ((shown / 2.0) as usize).max(1)
    }

    fn step(&mut self, by: isize) {
        let next = self.selected.saturating_add_signed(by);
        let target = if self.selectable(next) { next } else { next.saturating_add_signed(by) };
        if self.selectable(target) {
            self.select(target);
        }
    }

    pub(crate) fn point_at(&mut self, position: usize, at: Point<Pixels>, origin: Point<Pixels>) -> bool {
        let list = self.scroll.0.borrow().base_handle.bounds();
        if list.size.height > Pixels::ZERO && at.y >= list.bottom() {
            return false;
        }
        let pointer = origin + at;
        let moved = self.pointer.replace(pointer).is_some_and(|last| last != pointer);
        let changed = moved && position != self.selected && self.selectable(position);
        if changed {
            self.selected = position;
            self.reveal = Some(Reveal::Nearest);
        }
        changed
    }

    pub(crate) fn press(&mut self, position: usize, at: Point<Pixels>) {
        self.pressed = self.hit(position).map(|_| (position, at));
    }

    pub(crate) fn drag<T: 'static>(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<T>) {
        let Some((position, from)) = self.pressed else { return };
        if event.pressed_button != Some(MouseButton::Left) {
            self.pressed = None;
            return;
        }
        let (wide, tall) = platform::drag_distance();
        let moved = (event.position - from).map(|length| f32::from(length).abs() * window.scale_factor());
        if moved.x < wide as f32 && moved.y < tall as f32 {
            return;
        }
        self.pressed = None;
        let paths: Vec<String> = if self.picked.contains(&position) {
            self.picked_hits().map(|hit| hit.path.clone()).collect()
        } else {
            self.hit(position).map(|hit| hit.path.clone()).into_iter().collect()
        };
        if !paths.is_empty() {
            perform(Command::Drag(paths), cx);
        }
    }

    pub(crate) fn is_picked(&self, position: usize) -> bool {
        self.picked.contains(&position)
    }

    pub(crate) fn picked_count(&self) -> usize {
        self.picked.len()
    }

    fn picked_hits(&self) -> impl Iterator<Item = &Hit> {
        self.picked.iter().filter_map(|&position| self.hit(position))
    }

    pub(crate) fn targets(&self) -> Vec<Hit> {
        if self.picked.is_empty() {
            self.selected().cloned().into_iter().collect()
        } else {
            self.picked_hits().cloned().collect()
        }
    }

    pub(crate) fn targets_at(&mut self, position: usize) -> Vec<Hit> {
        if !self.picked.contains(&position) {
            self.unpick();
        }
        self.select(position);
        self.targets()
    }

    pub(crate) fn toggle_pick(&mut self, position: usize) {
        if self.hit(position).is_none() {
            return;
        }
        if !self.picked.remove(&position) {
            self.picked.insert(position);
        }
        self.select(position);
        self.anchor = position;
    }

    pub(crate) fn pick_click(&mut self, position: usize, event: &ClickEvent) -> bool {
        let modifiers = event.modifiers();
        if modifiers.control {
            self.toggle_pick(position);
        } else if modifiers.shift {
            self.pick_to(position);
        } else {
            return false;
        }
        true
    }

    pub(crate) fn pick_to(&mut self, position: usize) {
        if self.hit(position).is_none() {
            return;
        }
        let anchor = self.anchor;
        let range = anchor.min(position)..=anchor.max(position);
        self.picked = range.filter(|&row| self.hit(row).is_some()).collect();
        self.select(position);
    }

    pub(crate) fn pick_step(&mut self, by: isize) {
        let from = self.selected;
        self.step(by);
        if self.picked.is_empty() {
            self.anchor = from;
        }
        self.pick_to(self.selected);
    }

    pub(crate) fn unpick(&mut self) -> bool {
        self.anchor = self.selected;
        let had = !self.picked.is_empty();
        self.picked.clear();
        had
    }

    pub(crate) fn forget_pointer(&mut self) {
        self.pointer = None;
    }

    pub(crate) fn remove(&mut self, position: usize) -> Option<Hit> {
        let index = position.checked_sub(self.before.len()).filter(|&index| index < self.hits.len())?;
        let hit = self.hits.remove(index);
        self.picked.clear();
        self.selected = self.selected.min(self.len().saturating_sub(1));
        self.anchor = self.selected;
        Some(hit)
    }

    pub(crate) fn ratio(&self) -> f32 {
        self.ratio
    }

    pub(crate) fn rescale(&mut self, ratio: f32) -> bool {
        let changed = ratio != self.ratio;
        self.ratio = ratio;
        changed
    }

    pub(crate) fn start_icons(&self, side: f32, recent: Vec<(String, bool)>, cx: &mut App) {
        icons::start(recent, self.pixels(side), cx);
    }

    pub(crate) fn request_icons(&self, side: f32, cx: &mut App) {
        let pixels = self.pixels(side);
        icons::request(self.hits.iter().map(|hit| icon_key(hit, pixels)).collect(), cx);
    }

    pub(crate) fn icon(&self, hit: &Hit, side: f32, theme: &Theme, cx: &App) -> AnyElement {
        let replace_small = cx.global::<Settings>().replace_small_icons;
        match cx.global::<Icons>().get(&icon_key(hit, self.pixels(side)), replace_small) {
            Some(image) => img(image).size(rem(side)).flex_none().into_any_element(),
            None => {
                glyph(if hit.dir { assets::FOLDER } else { assets::FILE }, side, theme.results.icon).into_any_element()
            }
        }
    }

    pub(crate) fn extra_icon(extra: &Extra, side: f32, theme: &Theme) -> AnyElement {
        glyph(extra.tool.icon(), side, theme.results.icon).into_any_element()
    }

    pub(crate) fn status(&self, status: Status, language: Language) -> String {
        let strings = language.strings();
        if let Some(notice) = &self.notice {
            return notice.clone();
        }
        if !self.picked.is_empty() {
            return language.picked(self.picked.len());
        }
        match (self.listing, status.scanning) {
            (Listing::Recent, _) if !self.hits.is_empty() => strings.recent.to_owned(),
            (Listing::Folders, _) if !self.hits.is_empty() => strings.open_folders.to_owned(),
            (Listing::Actions, _) => strings.actions_status.to_owned(),
            (Listing::ContentOff, _) => strings.full_text_off_status.to_owned(),
            (Listing::ContentShort, _) => strings.full_text_short.to_owned(),
            (Listing::Found(total), false) if total > 0 => language.matches(total),
            (Listing::AtLeast(total), false) if total > 0 => language.at_least(total),
            (Listing::Contents(total), false) if total > 0 => language.items(total),
            (
                Listing::Recent
                | Listing::Folders
                | Listing::Searching
                | Listing::Found(_)
                | Listing::AtLeast(_)
                | Listing::Contents(_),
                _,
            ) => index_status(status, language),
        }
    }

    pub(crate) fn empty_message(&self, language: Language) -> &'static str {
        match self.listing {
            Listing::Found(_) | Listing::AtLeast(_) | Listing::Actions => language.strings().no_results,
            Listing::Contents(_) => language.strings().empty_folder,
            Listing::ContentOff => language.strings().full_text_off_status,
            Listing::ContentShort => language.strings().full_text_short,
            Listing::Recent | Listing::Folders | Listing::Searching => "",
        }
    }

    fn pixels(&self, side: f32) -> u32 {
        (side * self.ratio).round() as u32
    }
}

fn index_status(status: Status, language: Language) -> String {
    let strings = language.strings();
    let names = match (status.scanning, status.details) {
        (true, _) => format!("{} {}", strings.indexing, language.items(status.entries)),
        (false, false) => format!("{}, {}", language.items(status.entries), strings.reading_details),
        (false, true) => language.items(status.entries),
    };
    match status.content {
        ContentStatus::Building { done, total, .. } if total > 0 => {
            format!("{names}, {}", language.content_progress(done * PERCENT / total))
        }
        ContentStatus::Building { .. } | ContentStatus::Off | ContentStatus::Ready { .. } => names,
    }
}

pub(crate) fn result_hints(strings: &Strings, theme: &Theme, window: &Window) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rem(HINT_GAP))
        .child(hint_chip(&[Key::Enter], strings.open, theme, window))
        .child(hint_chip(&[CTRL, Key::Enter], strings.reveal, theme, window))
        .child(hint_chip(&[CTRL, Key::Text("O")], strings.actions_status, theme, window))
}

pub(crate) fn action_hints(strings: &Strings, theme: &Theme, window: &Window) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rem(HINT_GAP))
        .child(hint_chip(&[Key::Enter], strings.run, theme, window))
        .child(hint_chip(&[Key::Text("Esc")], strings.back, theme, window))
}

pub(super) fn icon_key(hit: &Hit, pixels: u32) -> IconKey {
    IconKey { source: IconSource::for_path(&hit.path, hit.dir), pixels }
}

#[cfg(test)]
mod tests {
    use super::{Arrival, Entry, Extra, Listing, Motion, ResultList, Tool};
    use crate::index::Hit;

    const PATH: &str = "C:\\Документы\\отчёт.txt";

    fn open_action() -> Extra {
        Extra {
            tool: Tool::Action { index: 0, dir: false, builtin: None },
            title: "Open".to_owned(),
            detail: String::new(),
            argument: PATH.to_owned(),
        }
    }

    #[test]
    fn actions_stay_as_they_are_until_the_search_after_them_lands() {
        let mut list = ResultList::new(1.0);
        list.present_with(vec![open_action()], Vec::new(), Vec::new(), Listing::Actions);
        list.start_search();
        assert_eq!(list.listing(), Listing::Actions);
        list.present(vec![Hit::new(PATH.to_owned(), false)], Listing::Found(1));
        assert_eq!(list.listing(), Listing::Found(1));
    }

    #[test]
    fn results_left_for_the_actions_come_back_as_they_were() {
        let mut list = ResultList::new(1.0);
        let hits = vec![Hit::new("C:\\a.txt".to_owned(), false), Hit::new(PATH.to_owned(), false)];
        list.present(hits, Listing::Found(2));
        list.select(1);
        let shown = list.shown();
        list.present_with(vec![open_action()], Vec::new(), Vec::new(), Listing::Actions);
        list.show_again(shown);
        assert_eq!(list.listing(), Listing::Found(2));
        assert_eq!(list.len(), 2);
        assert_eq!(list.selected().map(|hit| hit.path.as_str()), Some(PATH));
    }

    #[test]
    fn a_heading_takes_a_row_but_is_never_selected() {
        let mut list = ResultList::new(1.0);
        let second = Extra { title: "Send to".to_owned(), ..open_action() };
        list.present_with(vec![open_action(), second], Vec::new(), Vec::new(), Listing::Actions);
        list.head(1, "Context menu".to_owned());
        assert_eq!(list.len(), 3);
        assert!(matches!(list.entry(1), Some(Entry::Heading("Context menu"))));
        assert!(matches!(list.entry(2), Some(Entry::Extra(extra)) if extra.title == "Send to"));
        list.select_next();
        assert_eq!(list.selected_position(), 2);
        list.select_next();
        assert_eq!(list.selected_position(), 2);
        list.select_previous();
        assert_eq!(list.selected_position(), 0);
        list.present_with(vec![open_action()], Vec::new(), Vec::new(), Listing::Actions);
        list.head(0, "Context menu".to_owned());
        assert_eq!(list.selected_position(), 1);
        list.select_previous();
        assert_eq!(list.selected_position(), 1);
    }

    #[test]
    fn jumps_reach_the_ends_and_skip_a_heading() {
        let mut list = ResultList::new(1.0);
        let hits = (0..4).map(|row| Hit::new(format!("C:\\{row}.txt"), false)).collect();
        list.present(hits, Listing::Found(4));
        list.head(0, "Recent".to_owned());
        list.jump(Motion::Last);
        assert_eq!(list.selected_position(), 4);
        list.jump(Motion::HalfPageUp);
        assert_eq!(list.selected_position(), 3);
        list.jump(Motion::First);
        assert_eq!(list.selected_position(), 1);
        list.jump(Motion::HalfPageDown);
        assert_eq!(list.selected_position(), 2);
    }

    #[test]
    fn a_row_asked_for_before_a_search_is_selected_when_it_lands() {
        let mut list = ResultList::new(1.0);
        list.reselect(Some("c:\\WORK\\b.txt".to_owned()));
        let hits = vec![Hit::new("C:\\work\\a.txt".to_owned(), false), Hit::new("C:\\work\\b.txt".to_owned(), false)];
        list.present(hits.clone(), Listing::Contents(2));
        assert_eq!(list.selected_position(), 1);
        list.present(hits, Listing::Contents(2));
        assert_eq!(list.selected_position(), 0);
    }

    fn files(count: usize) -> Vec<Hit> {
        (0..count).map(|row| Hit::new(format!("C:\\{row}.txt"), false)).collect()
    }

    fn targets(list: &ResultList) -> Vec<String> {
        list.targets().into_iter().map(|hit| hit.path).collect()
    }

    #[test]
    fn picking_rows_makes_them_the_targets_and_a_new_listing_drops_them() {
        let mut list = ResultList::new(1.0);
        list.present(files(5), Listing::Found(5));
        list.select(1);
        assert_eq!(targets(&list), ["C:\\1.txt"]);
        list.toggle_pick(3);
        list.toggle_pick(0);
        assert_eq!(targets(&list), ["C:\\0.txt", "C:\\3.txt"]);
        list.toggle_pick(3);
        assert_eq!(targets(&list), ["C:\\0.txt"]);
        list.present(files(5), Listing::Found(5));
        assert_eq!(targets(&list), ["C:\\0.txt"]);
        assert!(!list.is_picked(0));
    }

    #[test]
    fn a_range_runs_from_the_row_chosen_by_keys_not_from_the_pointer() {
        let mut list = ResultList::new(1.0);
        list.present(files(6), Listing::Found(6));
        list.select_next();
        list.pick_to(4);
        assert_eq!(targets(&list), ["C:\\1.txt", "C:\\2.txt", "C:\\3.txt", "C:\\4.txt"]);
        list.pick_to(0);
        assert_eq!(targets(&list), ["C:\\0.txt", "C:\\1.txt"]);
    }

    #[test]
    fn stepping_with_shift_grows_and_shrinks_the_range() {
        let mut list = ResultList::new(1.0);
        list.present(files(4), Listing::Found(4));
        list.select(1);
        list.pick_step(1);
        list.pick_step(1);
        assert_eq!(targets(&list), ["C:\\1.txt", "C:\\2.txt", "C:\\3.txt"]);
        list.pick_step(-1);
        assert_eq!(targets(&list), ["C:\\1.txt", "C:\\2.txt"]);
        assert!(list.unpick());
        assert!(!list.unpick());
        assert_eq!(targets(&list), ["C:\\2.txt"]);
    }

    #[test]
    fn acting_on_a_row_outside_the_picked_ones_drops_them() {
        let mut list = ResultList::new(1.0);
        list.present(files(4), Listing::Found(4));
        list.toggle_pick(0);
        list.toggle_pick(1);
        assert_eq!(list.targets_at(1).len(), 2);
        assert_eq!(list.targets_at(3).into_iter().map(|hit| hit.path).collect::<Vec<_>>(), ["C:\\3.txt"]);
        assert!(!list.is_picked(0));
    }

    #[test]
    fn a_rerun_keeps_the_selected_and_picked_rows_where_they_moved() {
        let mut list = ResultList::new(1.0);
        list.present(files(4), Listing::Found(4));
        list.toggle_pick(1);
        list.select(2);
        let mut hits = files(4);
        hits.insert(0, Hit::new(PATH.to_owned(), false));
        list.land(Arrival::Rerun, Vec::new(), hits, Vec::new(), Listing::Found(5));
        assert_eq!(list.selected().map(|hit| hit.path.as_str()), Some("C:\\2.txt"));
        assert_eq!(targets(&list), ["C:\\1.txt"]);
    }

    #[test]
    fn a_rerun_without_the_selected_row_stays_at_its_place() {
        let mut list = ResultList::new(1.0);
        list.present(files(4), Listing::Found(4));
        list.select(3);
        list.land(Arrival::Rerun, Vec::new(), files(2), Vec::new(), Listing::Found(2));
        assert_eq!(list.selected_position(), 1);
        list.land(Arrival::Rerun, Vec::new(), files(3), Vec::new(), Listing::Found(3));
        assert_eq!(list.selected_position(), 1);
        list.land(Arrival::Fresh, Vec::new(), files(3), Vec::new(), Listing::Found(3));
        assert_eq!(list.selected_position(), 0);
    }

    #[test]
    fn suggestions_turn_into_a_running_search() {
        let mut list = ResultList::new(1.0);
        list.present(vec![Hit::new(PATH.to_owned(), false)], Listing::Recent);
        list.start_search();
        assert_eq!(list.listing(), Listing::Searching);
    }
}
