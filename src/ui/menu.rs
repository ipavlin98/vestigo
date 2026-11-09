mod folders;
mod panel;

use super::icons::{picture, readable};
use super::{
    Back, Confirm, Dismiss, FILL_GAP, INDENT, Jump, Motion, SelectNext, SelectPrevious, ShowActions, binding,
    popup_options, zoom,
};
use crate::platform::{self, MenuAt, MenuSpot, PointerMove};
use crate::theme::{Chrome, Theme};
pub(crate) use folders::{Go, favorite_entries, folder_entries, nothing, recent_folders};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, Global, RenderImage, SharedString, Subscription, Task, WindowHandle,
};
use panel::Panel;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONTEXT: &str = "Menu";
const ROOT: usize = 0;
const WARM_PANELS: usize = 4;
const MAX_DEPTH: usize = 8;
const HOVER_DWELL: Duration = Duration::from_millis(40);
const AIM_DELAY: Duration = Duration::from_millis(200);
const LEAVE_DELAY: Duration = Duration::from_millis(300);
const PAGE: usize = 8;
const MENU_EDGE: f32 = FILL_GAP + INDENT;

pub(crate) type Choose = Rc<dyn Fn(&mut App)>;
type Expand = Rc<dyn Fn(&mut App) -> Task<Vec<MenuEntry>>>;

#[derive(Clone)]
pub(crate) enum MenuIcon {
    Blank,
    Glyph(&'static str),
    Path { path: String, dir: bool },
    Picture(Arc<RenderImage>),
}

#[derive(Clone)]
pub(crate) struct MenuItem {
    label: SharedString,
    icon: MenuIcon,
    hint: Option<SharedString>,
    choose: Option<Choose>,
    submenu: Option<Expand>,
}

impl MenuItem {
    pub(crate) fn new(label: impl Into<SharedString>, icon: MenuIcon) -> Self {
        Self { label: label.into(), icon, hint: None, choose: None, submenu: None }
    }

    pub(crate) fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub(crate) fn on_choose(mut self, choose: impl Fn(&mut App) + 'static) -> Self {
        self.choose = Some(Rc::new(choose));
        self
    }

    pub(crate) fn submenu(mut self, expand: impl Fn(&mut App) -> Task<Vec<MenuEntry>> + 'static) -> Self {
        self.submenu = Some(Rc::new(expand));
        self
    }

    fn enabled(&self) -> bool {
        self.choose.is_some() || self.submenu.is_some()
    }

    fn activation(&self) -> Activation {
        match (&self.choose, &self.submenu) {
            (Some(choose), _) => Activation::Choose(choose.clone()),
            (None, Some(_)) => Activation::Open,
            (None, None) => Activation::Nothing,
        }
    }
}

enum Activation {
    Choose(Choose),
    Open,
    Nothing,
}

#[derive(Clone)]
pub(crate) enum MenuEntry {
    Item(MenuItem),
    Heading { label: SharedString, button: Option<(&'static str, Choose)> },
    Separator,
}

impl MenuEntry {
    fn selectable(&self) -> bool {
        match self {
            Self::Item(item) => item.enabled(),
            Self::Heading { .. } | Self::Separator => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Driver {
    Pointer,
    Keys,
}

struct Level {
    entries: Vec<MenuEntry>,
    selected: Option<usize>,
    parent: Option<usize>,
    panel: usize,
}

impl Level {
    fn item(&self, row: usize) -> Option<&MenuItem> {
        match self.entries.get(row)? {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Heading { .. } | MenuEntry::Separator => None,
        }
    }

    fn next(&self, by: isize) -> Option<usize> {
        let count = self.entries.len() as isize;
        let start = self.selected.map_or(if by > 0 { -1 } else { count }, |row| row as isize);
        (1..=count)
            .map(|step| (start + by * step).rem_euclid(count) as usize)
            .find(|&row| self.entries.get(row).is_some_and(MenuEntry::selectable))
    }

    fn toward(&self, by: isize, limit: usize) -> Option<usize> {
        let count = self.entries.len() as isize;
        let mut at = self.selected.map_or(if by > 0 { -1 } else { count }, |row| row as isize);
        let (mut found, mut moved) = (None, 0);
        while moved < limit {
            at += by;
            let Some(entry) = usize::try_from(at).ok().and_then(|row| self.entries.get(row)) else { break };
            if entry.selectable() {
                found = Some(at as usize);
                moved += 1;
            }
        }
        found.or(self.selected)
    }

    fn starting_with(&self, letter: char) -> Option<usize> {
        let count = self.entries.len();
        let start = self.selected.map_or(0, |row| row + 1);
        let wanted =
            |label: &str| label.chars().next().is_some_and(|first| first.to_lowercase().eq(letter.to_lowercase()));
        (0..count)
            .map(|step| (start + step) % count)
            .find(|&row| self.item(row).is_some_and(|item| item.enabled() && wanted(&item.label)))
    }
}

pub(crate) struct Menu {
    panels: Vec<WindowHandle<Panel>>,
    levels: Vec<Level>,
    active: usize,
    reveal: usize,
    pending: Option<Task<()>>,
    leaving: Option<(usize, Task<()>)>,
    root: MenuAt,
    chrome: Chrome,
    driver: Driver,
}

struct Toggled;

impl EventEmitter<Toggled> for Menu {}

struct Shared(Entity<Menu>);

impl Global for Shared {}

pub(crate) fn start(cx: &mut App) {
    let menu = cx.new(|_| Menu {
        panels: Vec::new(),
        levels: Vec::new(),
        active: ROOT,
        reveal: 0,
        pending: None,
        leaving: None,
        root: MenuAt::Cursor,
        chrome: Chrome::default(),
        driver: Driver::Pointer,
    });
    cx.set_global(Shared(menu.clone()));
    for index in 0..WARM_PANELS {
        let Some(handle) = open_panel(menu.clone(), index, cx) else { break };
        menu.update(cx, |menu, _| menu.panels.push(handle));
    }
}

fn open_panel(menu: Entity<Menu>, index: usize, cx: &mut App) -> Option<WindowHandle<Panel>> {
    let options = popup_options(cx);
    let opened = cx.open_window(options, |window, cx| {
        platform::attach_menu(window, index, index == ROOT, cx);
        cx.new(|cx| Panel::new(menu, index, window, cx))
    });
    opened.inspect_err(|error| log::warn!("Couldn't open a menu window: {error:#}")).ok()
}

pub(crate) fn bind_keys(cx: &mut App) {
    let bindings = [
        binding("down", SelectNext, CONTEXT, cx),
        binding("tab", SelectNext, CONTEXT, cx),
        binding("up", SelectPrevious, CONTEXT, cx),
        binding("shift-tab", SelectPrevious, CONTEXT, cx),
        binding("right", ShowActions, CONTEXT, cx),
        binding("left", Back, CONTEXT, cx),
        binding("home", Jump { to: Motion::First }, CONTEXT, cx),
        binding("end", Jump { to: Motion::Last }, CONTEXT, cx),
        binding("pageup", Jump { to: Motion::HalfPageUp }, CONTEXT, cx),
        binding("pagedown", Jump { to: Motion::HalfPageDown }, CONTEXT, cx),
        binding("enter", Confirm, CONTEXT, cx),
        binding("space", Confirm, CONTEXT, cx),
        binding("escape", Dismiss, CONTEXT, cx),
    ];
    cx.bind_keys(bindings.into_iter().flatten());
}

pub(crate) fn open(entries: Vec<MenuEntry>, cx: &mut App) {
    let chrome = cx.global::<Theme>().windows.launcher.clone();
    open_at(entries, MenuAt::Cursor, chrome, cx);
}

pub(crate) fn open_at(entries: Vec<MenuEntry>, at: MenuAt, chrome: Chrome, cx: &mut App) {
    let Some(menu) = cx.try_global::<Shared>().map(|shared| shared.0.clone()) else { return };
    menu.update(cx, |menu, cx| menu.open_root(entries, at, chrome, cx));
}

pub(crate) fn shown_at(at: MenuAt, cx: &App) -> bool {
    cx.try_global::<Shared>().is_some_and(|shared| {
        let menu = shared.0.read(cx);
        !menu.levels.is_empty() && menu.root == at
    })
}

pub(crate) fn close_search(cx: &mut App) {
    let Some(menu) = cx.try_global::<Shared>().map(|shared| shared.0.clone()) else { return };
    menu.update(cx, |menu, cx| match menu.root {
        MenuAt::Search { .. } => menu.close(true, None, cx),
        MenuAt::Cursor | MenuAt::Beside { .. } => {}
    });
}

pub(crate) fn follow<T: 'static>(
    cx: &mut Context<T>,
    toggled: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> Option<Subscription> {
    let menu = cx.try_global::<Shared>()?.0.clone();
    Some(cx.subscribe(&menu, move |this, _, _: &Toggled, cx| toggled(this, cx)))
}

pub(crate) fn open_shell(path: String, cx: &mut App) {
    let entries = shell_entries(path, Vec::new(), cx);
    cx.spawn(async move |cx| {
        let entries = entries.await;
        if !entries.is_empty() {
            cx.update(|cx| open(entries, cx));
        }
    })
    .detach();
}

struct ShellRow {
    title: String,
    picture: Option<Arc<RenderImage>>,
    submenu: bool,
    trail: Vec<u32>,
    separated: bool,
}

pub(crate) fn shell_entries(path: String, trail: Vec<u32>, cx: &mut App) -> Task<Vec<MenuEntry>> {
    let items = platform::load_shell_menu(path.clone(), trail);
    let background = cx.global::<Theme>().launcher.background.l;
    let rows = cx.background_spawn(async move {
        let items = items.await.unwrap_or_default();
        items
            .into_iter()
            .map(|item| ShellRow {
                picture: item.icon.and_then(|mut icon| {
                    readable(&mut icon.bgra, background);
                    picture(icon.width, icon.height, icon.bgra)
                }),
                title: item.title,
                submenu: item.submenu,
                trail: item.trail,
                separated: item.separated,
            })
            .collect::<Vec<_>>()
    });
    cx.spawn(async move |_| {
        let mut entries = Vec::new();
        for row in rows.await {
            if row.separated {
                entries.push(MenuEntry::Separator);
            }
            let item = MenuItem::new(row.title, row.picture.map_or(MenuIcon::Blank, MenuIcon::Picture));
            let (path, trail) = (path.clone(), row.trail);
            entries.push(MenuEntry::Item(if row.submenu {
                item.submenu(move |cx| shell_entries(path.clone(), trail.clone(), cx))
            } else {
                item.on_choose(move |_| platform::run_shell_item(path.clone(), trail.clone()))
            }));
        }
        entries
    })
}

impl Menu {
    fn free_panel(&self) -> Option<usize> {
        let used = |panel: usize| self.levels.iter().any(|level| level.panel == panel);
        (ROOT + 1..self.panels.len()).find(|&panel| !used(panel))
    }

    fn level_on(&self, panel: usize) -> Option<(usize, &Level)> {
        self.levels.iter().enumerate().find(|(_, level)| level.panel == panel)
    }

    fn open_root(&mut self, entries: Vec<MenuEntry>, at: MenuAt, chrome: Chrome, cx: &mut Context<Self>) {
        if self.panels.is_empty() {
            return;
        }
        self.root = at;
        self.chrome = chrome;
        cx.emit(Toggled);
        let shown: Vec<usize> = self.levels.drain(..).map(|level| level.panel).filter(|&panel| panel != ROOT).collect();
        self.pending = None;
        self.leaving = None;
        self.driver = Driver::Pointer;
        self.active = ROOT;
        self.levels.push(Level { entries, selected: None, parent: None, panel: ROOT });
        self.show(ROOT, None, shown, cx);
    }

    fn show(&mut self, depth: usize, replaced: Option<usize>, hidden: Vec<usize>, cx: &mut Context<Self>) {
        let Some(level) = self.levels.get(depth) else { return };
        let Some(handle) = self.panels.get(level.panel).copied() else { return };
        let entries = level.entries.clone();
        let Ok((width, heights)) = handle.update(cx, |panel, window, cx| panel.prepare(&entries, window, cx)) else {
            return;
        };
        let (at, chrome) = match depth.checked_sub(1).and_then(|up| self.levels.get(up)) {
            Some(parent) => {
                let above = self.panels.get(parent.panel).copied();
                let scrolled = above.and_then(|above| above.update(cx, |panel, window, _| panel.scrolled(window)).ok());
                let row = panel::row_top(&parent.entries, level.parent.unwrap_or_default());
                (
                    MenuAt::Beside { panel: parent.panel, top: row - scrolled.unwrap_or_default() - FILL_GAP },
                    cx.global::<Theme>().windows.launcher.clone(),
                )
            }
            None => (self.root, self.chrome.clone()),
        };
        let spot = MenuSpot { width, heights, zoom: zoom(cx), chrome, at };
        let panel = level.panel;
        cx.notify();
        platform::note_placing();
        cx.spawn(async move |this, cx| {
            let shown = platform::place_menu(panel, spot, replaced);
            platform::hide_menus(&hidden);
            if !shown {
                let _ = this.update(cx, |menu, cx| menu.close(false, None, cx));
            }
        })
        .detach();
    }

    fn collapse(&mut self, keep: usize, cx: &mut Context<Self>) {
        self.pending = None;
        if self.levels.len() <= keep {
            return;
        }
        let hidden: Vec<usize> = self.levels.drain(keep..).map(|level| level.panel).collect();
        self.active = self.active.min(keep.saturating_sub(1));
        cx.notify();
        cx.spawn(async move |_, _| platform::hide_menus(&hidden)).detach();
    }

    fn close(&mut self, restore: bool, choose: Option<Choose>, cx: &mut Context<Self>) {
        self.pending = None;
        self.leaving = None;
        if self.levels.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |menu, cx| {
                menu.levels.clear();
                cx.emit(Toggled);
            });
            cx.spawn(async move |cx| {
                platform::close_menu(restore);
                if let Some(choose) = choose {
                    cx.update(|cx| choose(cx));
                }
            })
            .detach();
        })
        .detach();
    }

    fn point(&mut self, depth: usize, row: usize, cx: &mut Context<Self>) {
        let aiming = match platform::pointer_move(self.levels.get(depth + 1).map(|below| below.panel)) {
            PointerMove::Still => return,
            PointerMove::Free => false,
            PointerMove::Aiming => true,
        };
        let Some(level) = self.levels.get(depth) else { return };
        let parent = level.parent;
        let selectable = level.entries.get(row).is_some_and(MenuEntry::selectable);
        let opens = level.item(row).is_some_and(|item| item.submenu.is_some());
        if let Some(parent) = parent
            && let Some(up) = depth.checked_sub(1).and_then(|up| self.levels.get_mut(up))
            && up.selected != Some(parent)
        {
            up.selected = Some(parent);
            self.pending = None;
            cx.notify();
        }
        self.active = depth;
        if !selectable {
            self.rest(depth, cx);
            return;
        }
        self.driver = Driver::Pointer;
        self.leaving = None;
        let Some(level) = self.levels.get_mut(depth) else { return };
        if level.selected == Some(row) {
            if self.pending.is_some() || !opens {
                return;
            }
        } else {
            level.selected = Some(row);
            cx.notify();
        }
        if self.levels.get(depth + 1).is_some_and(|below| below.parent == Some(row)) {
            self.pending = None;
            return;
        }
        self.expand(depth, row, false, if aiming { AIM_DELAY } else { HOVER_DWELL }, cx);
    }

    fn rest(&mut self, depth: usize, cx: &mut Context<Self>) {
        if self.driver == Driver::Keys {
            return;
        }
        let Some(level) = self.levels.get(depth) else { return };
        if level.selected.is_none() && self.levels.len() <= depth + 1 {
            self.leaving = None;
            return;
        }
        if self.leaving.as_ref().is_some_and(|(pending, _)| *pending == depth) {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LEAVE_DELAY).await;
            let _ = this.update(cx, |menu, cx| menu.settle(depth, cx));
        });
        self.leaving = Some((depth, task));
    }

    fn settle(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.leaving = None;
        let Some(level) = self.levels.get_mut(depth) else { return };
        if !platform::pointer_on_panel(level.panel) {
            return;
        }
        level.selected = None;
        self.collapse(depth + 1, cx);
        self.active = depth;
        cx.notify();
    }

    fn activate(&mut self, depth: usize, row: usize, cx: &mut Context<Self>) {
        let Some(item) = self.levels.get(depth).and_then(|level| level.item(row)) else { return };
        match item.activation() {
            Activation::Choose(choose) => self.close(true, Some(choose), cx),
            Activation::Open => {
                if let Some(level) = self.levels.get_mut(depth) {
                    level.selected = Some(row);
                }
                self.active = depth;
                cx.notify();
                self.expand(depth, row, true, Duration::ZERO, cx);
            }
            Activation::Nothing => {}
        }
    }

    fn expand(&mut self, depth: usize, row: usize, keyboard: bool, delay: Duration, cx: &mut Context<Self>) {
        if self.levels.get(depth + 1).is_some_and(|below| below.parent == Some(row)) {
            self.pending = None;
            if keyboard {
                self.enter(depth + 1, cx);
            }
            return;
        }
        let expand = self.levels.get(depth).and_then(|level| level.item(row)).and_then(|item| item.submenu.clone());
        let entries = expand.filter(|_| depth + 1 < MAX_DEPTH).map(|expand| expand(cx));
        let started = Instant::now();
        self.pending = Some(cx.spawn(async move |this, cx| {
            let entries = match entries {
                Some(entries) => entries.await,
                None => Vec::new(),
            };
            if let Some(rest) = delay.checked_sub(started.elapsed()).filter(|rest| !rest.is_zero()) {
                cx.background_executor().timer(rest).await;
            }
            let _ = this.update(cx, |menu, cx| menu.attach(depth, row, entries, keyboard, cx));
        }));
    }

    fn attach(&mut self, depth: usize, row: usize, entries: Vec<MenuEntry>, keyboard: bool, cx: &mut Context<Self>) {
        if self.levels.get(depth).and_then(|level| level.selected) != Some(row) {
            return;
        }
        if !keyboard && self.leaving.as_ref().is_some_and(|(pending, _)| *pending == depth) {
            return;
        }
        if entries.is_empty() {
            self.collapse(depth + 1, cx);
            return;
        }
        let Some(panel) = self.free_panel() else {
            self.grow(depth, row, entries, keyboard, cx);
            return;
        };
        let replaced = self.levels.get(depth + 1).map(|level| level.panel);
        let hidden: Vec<usize> = self.levels.iter().skip(depth + 2).map(|level| level.panel).collect();
        self.levels.truncate(depth + 1);
        self.levels.push(Level { entries, selected: None, parent: Some(row), panel });
        self.active = self.active.min(depth);
        if keyboard {
            self.enter(depth + 1, cx);
        }
        self.show(depth + 1, replaced, hidden, cx);
    }

    fn grow(&mut self, depth: usize, row: usize, entries: Vec<MenuEntry>, keyboard: bool, cx: &mut Context<Self>) {
        let index = self.panels.len();
        self.pending = Some(cx.spawn(async move |this, cx| {
            let Some(handle) = cx.update(|cx| open_panel(this.upgrade()?, index, cx)) else { return };
            let _ = this.update(cx, |menu, cx| {
                if menu.panels.len() == index {
                    menu.panels.push(handle);
                    menu.attach(depth, row, entries, keyboard, cx);
                }
            });
        }));
    }

    fn enter(&mut self, depth: usize, cx: &mut Context<Self>) {
        let Some(level) = self.levels.get_mut(depth) else { return };
        if level.selected.is_none() {
            level.selected = level.next(1);
        }
        self.active = depth;
        self.reveal += 1;
        cx.notify();
    }

    fn keys(&mut self) {
        self.driver = Driver::Keys;
        self.leaving = None;
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let next = self.levels.get(self.active).and_then(|level| level.next(by));
        self.select(next, cx);
    }

    fn jump(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let (by, limit) = match motion {
            Motion::First => (-1, usize::MAX),
            Motion::Last => (1, usize::MAX),
            Motion::HalfPageUp => (-1, PAGE),
            Motion::HalfPageDown => (1, PAGE),
        };
        let next = self.levels.get(self.active).and_then(|level| level.toward(by, limit));
        self.select(next, cx);
    }

    fn seek(&mut self, letter: char, cx: &mut Context<Self>) {
        let next = self.levels.get(self.active).and_then(|level| level.starting_with(letter));
        self.select(next, cx);
    }

    fn select(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        self.keys();
        let depth = self.active;
        let Some(row) = row else { return };
        let Some(level) = self.levels.get_mut(depth) else { return };
        if level.selected == Some(row) {
            return;
        }
        level.selected = Some(row);
        self.reveal += 1;
        self.collapse(depth + 1, cx);
        cx.notify();
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        self.keys();
        let depth = self.active;
        let Some(row) = self.levels.get(depth).and_then(|level| level.selected) else { return };
        if self.levels.get(depth).and_then(|level| level.item(row)).is_some_and(|item| item.submenu.is_some()) {
            self.expand(depth, row, true, Duration::ZERO, cx);
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        self.keys();
        let depth = self.active;
        if let Some(row) = self.levels.get(depth).and_then(|level| level.selected) {
            self.activate(depth, row, cx);
        }
    }

    fn back(&mut self, cx: &mut Context<Self>) {
        self.keys();
        if self.active > ROOT {
            self.collapse(self.active, cx);
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.active > ROOT {
            self.back(cx);
        } else {
            self.close(true, None, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level(entries: Vec<MenuEntry>, selected: Option<usize>) -> Level {
        Level { entries, selected, parent: None, panel: ROOT }
    }

    fn enabled() -> MenuEntry {
        MenuEntry::Item(MenuItem::new("on", MenuIcon::Blank).on_choose(|_| {}))
    }

    fn disabled() -> MenuEntry {
        MenuEntry::Item(MenuItem::new("off", MenuIcon::Blank))
    }

    #[test]
    fn keys_skip_headings_separators_and_disabled_rows_and_wrap() {
        let heading = MenuEntry::Heading { label: "title".into(), button: None };
        let entries = vec![heading, enabled(), MenuEntry::Separator, disabled(), enabled()];
        assert_eq!(level(entries.clone(), None).next(1), Some(1));
        assert_eq!(level(entries.clone(), Some(1)).next(1), Some(4));
        assert_eq!(level(entries.clone(), Some(4)).next(1), Some(1), "down from the last row wraps to the first");
        assert_eq!(level(entries.clone(), None).next(-1), Some(4));
        assert_eq!(level(entries, Some(1)).next(-1), Some(4), "up from the first row wraps to the last");
    }

    fn named(label: &'static str) -> MenuEntry {
        MenuEntry::Item(MenuItem::new(label, MenuIcon::Blank).on_choose(|_| {}))
    }

    #[test]
    fn a_folder_with_contents_is_chosen_and_a_plain_submenu_opens() {
        let folder = MenuItem::new("Work", MenuIcon::Blank).on_choose(|_| {}).submenu(|_| Task::ready(Vec::new()));
        let listing = MenuItem::new("Recent", MenuIcon::Blank).submenu(|_| Task::ready(Vec::new()));
        assert!(matches!(folder.activation(), Activation::Choose(_)), "a click or Enter goes to the folder itself");
        assert!(matches!(listing.activation(), Activation::Open));
        assert!(matches!(MenuItem::new("off", MenuIcon::Blank).activation(), Activation::Nothing));
    }

    #[test]
    fn home_end_and_pages_stop_at_the_ends_without_wrapping() {
        let heading = MenuEntry::Heading { label: "title".into(), button: None };
        let entries = vec![heading, enabled(), disabled(), enabled(), MenuEntry::Separator, enabled(), enabled()];
        assert_eq!(level(entries.clone(), Some(5)).toward(-1, usize::MAX), Some(1), "Home");
        assert_eq!(level(entries.clone(), None).toward(1, usize::MAX), Some(6), "End with nothing selected");
        assert_eq!(level(entries.clone(), Some(1)).toward(1, 2), Some(5), "a page skips disabled rows");
        assert_eq!(level(entries.clone(), Some(5)).toward(1, 8), Some(6), "a page near the end stops on the last row");
        assert_eq!(level(entries, Some(1)).toward(-1, 8), Some(1), "a page up from the first row stays there");
    }

    #[test]
    fn a_typed_letter_selects_the_next_row_starting_with_it_in_any_case() {
        let entries = vec![named("Документы"), named("desktop"), named("Загрузки"), named("Диски"), disabled()];
        assert_eq!(level(entries.clone(), None).starting_with('д'), Some(0));
        assert_eq!(level(entries.clone(), Some(0)).starting_with('Д'), Some(3));
        assert_eq!(level(entries.clone(), Some(3)).starting_with('д'), Some(0), "wraps to the first match");
        assert_eq!(level(entries.clone(), Some(0)).starting_with('D'), Some(1));
        assert_eq!(level(entries, None).starting_with('o'), None, "disabled rows aren't jumped to");
    }

    #[test]
    fn a_menu_may_end_only_after_a_whole_item_within_the_height_limit() {
        let heading = MenuEntry::Heading { label: "title".into(), button: None };
        let short = vec![heading, enabled(), MenuEntry::Separator, enabled()];
        assert_eq!(panel::heights(&short), [76.0, 124.0], "never right after the heading or the separator");
        assert_eq!(panel::heights(&vec![enabled(); 18]).last(), Some(&616.0), "the 18th row would pass 640");
        assert_eq!(panel::heights(&vec![enabled(); 2]), [40.0, 76.0]);
    }

    #[test]
    fn a_scrolled_menu_rests_with_a_row_on_top_never_a_separator() {
        let heading = MenuEntry::Heading { label: "title".into(), button: None };
        let entries = vec![heading, enabled(), MenuEntry::Separator, enabled()];
        assert_eq!(panel::rest_offsets(&entries), [0.0, 36.0, 84.0]);
    }

    #[test]
    fn a_row_shown_by_keys_keeps_the_menu_padding_and_the_first_one_its_heading() {
        let heading = MenuEntry::Heading { label: "title".into(), button: None };
        let entries = vec![heading, enabled(), enabled(), MenuEntry::Separator, enabled()];
        assert_eq!(panel::row_span(&entries, 1), (0.0, 76.0), "the heading above the first item scrolls in with it");
        assert_eq!(panel::row_span(&entries, 2), (72.0, 112.0));
        assert_eq!(panel::row_span(&entries, 4), (120.0, 160.0));
    }

    #[test]
    fn a_level_without_enabled_rows_selects_nothing() {
        assert_eq!(level(vec![disabled(), MenuEntry::Separator], None).next(1), None);
        assert_eq!(level(Vec::new(), None).next(-1), None);
    }
}
