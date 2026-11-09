use super::super::icons::{self, IconKey, Icons};
use super::super::{
    Back, CONTROL, Confirm, DETAIL_LINE, DETAIL_SIZE, Dismiss, Edge, FILL_GAP, GLYPH, INDENT, INSET, Jump, LABEL_GAP,
    Look as Fill, Mark, NAME_LINE, NAME_SIZE, SelectNext, SelectPrevious, ShowActions, Tip, Under, divider,
    faded_unless_fits, fit_rem, glide_rows, glyph, icon_button, paint_changes, pixel_ratio, scroll_to_span, slot_width,
    window_shape,
};
use super::{CONTEXT, MENU_EDGE, Menu, MenuEntry, MenuIcon, MenuItem, ROOT};
use crate::assets;
use crate::platform::{IconSource, MenuAt};
use crate::settings::Settings;
use crate::theme::{BASE_REM, GRID, Theme, pixels, rem};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Render, ScrollHandle,
    SharedString, Subscription, Window, div, img, point, prelude::*, px,
};

const MIN_WIDTH: f32 = 224.0;
const MAX_WIDTH: f32 = 384.0;
const MAX_HEIGHT: f32 = 640.0;
const SEPARATOR: f32 = INSET;
const ROWS_ID: &str = "menu-rows";
const REVEAL_ID: &str = "menu-reveal";
const AREA_ID: &str = "menu-area";

pub(in crate::ui) struct Panel {
    menu: Entity<Menu>,
    index: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
    ratio: f32,
    fits: bool,
    _subscriptions: Vec<Subscription>,
}

impl Panel {
    pub(super) fn new(menu: Entity<Menu>, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![
            cx.observe(&menu, |_, _, cx| cx.notify()),
            cx.observe_global::<Icons>(|_, cx| cx.notify()),
            cx.observe_window_bounds(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Theme>(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Settings>(window, |this, window, cx| this.rescale(window, cx)),
            paint_changes(cx),
        ];
        if index == ROOT {
            subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.menu.update(cx, |menu, cx| menu.close(false, None, cx));
                }
            }));
        }
        fit_rem(window, cx);
        Self {
            menu,
            index,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            ratio: pixel_ratio(window),
            fits: true,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn prepare(
        &mut self,
        entries: &[MenuEntry],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (f32, Vec<f32>) {
        fit_rem(window, cx);
        self.ratio = pixel_ratio(window);
        self.scroll.set_offset(point(px(0.), px(0.)));
        if self.index == ROOT {
            window.focus(&self.focus, cx);
        }
        request_icons(entries, self.pixels(), cx);
        let (width, fits) = measure(entries, cx.global::<Theme>(), window);
        self.fits = fits;
        (width, heights(entries))
    }

    fn rescale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        fit_rem(window, cx);
        let ratio = pixel_ratio(window);
        if ratio != self.ratio {
            self.ratio = ratio;
            let menu = self.menu.read(cx);
            let entries = menu.level_on(self.index).map(|(_, level)| level.entries.clone()).unwrap_or_default();
            request_icons(&entries, self.pixels(), cx);
        }
        cx.notify();
    }

    pub(super) fn scrolled(&self, window: &Window) -> f32 {
        -f32::from(self.scroll.offset().y) * BASE_REM / f32::from(window.rem_size())
    }

    fn pixels(&self) -> u32 {
        (GLYPH * self.ratio).round() as u32
    }
}

fn request_icons(entries: &[MenuEntry], pixels: u32, cx: &mut Context<Panel>) {
    let keys = entries
        .iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(MenuItem { icon: MenuIcon::Path { path, dir }, .. }) => {
                Some(IconKey { source: IconSource::for_path(path, *dir), pixels })
            }
            MenuEntry::Item(_) | MenuEntry::Heading { .. } | MenuEntry::Separator => None,
        })
        .collect();
    icons::request(keys, cx);
}

fn height(entry: &MenuEntry) -> f32 {
    match entry {
        MenuEntry::Item(_) | MenuEntry::Heading { .. } => CONTROL,
        MenuEntry::Separator => SEPARATOR,
    }
}

pub(super) fn row_top(entries: &[MenuEntry], row: usize) -> f32 {
    FILL_GAP + entries.iter().take(row).map(|entry| height(entry) + FILL_GAP).sum::<f32>()
}

pub(super) fn row_span(entries: &[MenuEntry], row: usize) -> (f32, f32) {
    let top = row_top(entries, row);
    let bottom = top + entries.get(row).map_or(0., height) + FILL_GAP;
    let first = !entries.iter().take(row).any(MenuEntry::selectable);
    (if first { 0. } else { top - FILL_GAP }, bottom)
}

fn has_icons(entries: &[MenuEntry]) -> bool {
    entries.iter().any(|entry| matches!(entry, MenuEntry::Item(item) if !matches!(item.icon, MenuIcon::Blank)))
}

pub(super) fn heights(entries: &[MenuEntry]) -> Vec<f32> {
    let last = entries.len().saturating_sub(1);
    let ends = entries.iter().enumerate().scan(FILL_GAP, |bottom, (row, entry)| {
        *bottom += height(entry) + FILL_GAP;
        Some((*bottom, row == last || matches!(entry, MenuEntry::Item(_))))
    });
    ends.filter(|&(bottom, can_end)| can_end && bottom <= MAX_HEIGHT).map(|(bottom, _)| bottom).collect()
}

pub(super) fn rest_offsets(entries: &[MenuEntry]) -> Vec<f32> {
    let tops = entries.iter().scan(0., |top: &mut f32, entry| {
        let at = *top;
        *top += height(entry) + FILL_GAP;
        Some((at, !matches!(entry, MenuEntry::Separator)))
    });
    tops.filter(|&(_, can_lead)| can_lead).map(|(at, _)| at).collect()
}

fn rests(entries: &[MenuEntry], window: &Window) -> Vec<gpui::Pixels> {
    rest_offsets(entries).into_iter().map(|at| pixels(at, window)).collect()
}

fn measure(entries: &[MenuEntry], theme: &Theme, window: &Window) -> (f32, bool) {
    let base = |text: &SharedString, size: f32| {
        f32::from(slot_width(text.clone(), size, theme, window)) * BASE_REM / f32::from(window.rem_size())
    };
    let icon = if has_icons(entries) { GLYPH + LABEL_GAP } else { 0. };
    let widest = entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Item(item) => {
                icon + base(&item.label, NAME_SIZE)
                    + item.hint.as_ref().map_or(0., |hint| LABEL_GAP + base(hint, DETAIL_SIZE))
                    + if item.submenu.is_some() { LABEL_GAP + GLYPH } else { 0. }
            }
            MenuEntry::Heading { label, button } => {
                base(label, DETAIL_SIZE) + if button.is_some() { LABEL_GAP + GLYPH } else { 0. }
            }
            MenuEntry::Separator => 0.,
        })
        .fold(0., f32::max);
    let width = 2. * MENU_EDGE + widest;
    let width = ((width / GRID).ceil() * GRID).max(MIN_WIDTH);
    (width.min(MAX_WIDTH), width <= MAX_WIDTH)
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let menu = self.menu.read(cx);
        let corners = if self.index == ROOT { menu.chrome.corners } else { theme.windows.launcher.corners };
        let seam = match menu.root {
            MenuAt::Search { upward: false, .. } if self.index == ROOT => Some(Edge::Top),
            MenuAt::Search { upward: true, .. } if self.index == ROOT => Some(Edge::Bottom),
            MenuAt::Search { .. } | MenuAt::Cursor | MenuAt::Beside { .. } => None,
        };
        let panel = window_shape(div(), corners)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.launcher.background)
            .font_family(theme.font.clone())
            .text_size(rem(NAME_SIZE))
            .line_height(rem(NAME_LINE))
            .text_color(theme.results.name)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(on(&self.menu, |menu, _: &SelectNext, cx| menu.step(1, cx)))
            .on_action(on(&self.menu, |menu, _: &SelectPrevious, cx| menu.step(-1, cx)))
            .on_action(on(&self.menu, |menu, _: &ShowActions, cx| menu.open_selected(cx)))
            .on_action(on(&self.menu, |menu, _: &Back, cx| menu.back(cx)))
            .on_action(on(&self.menu, |menu, _: &Confirm, cx| menu.confirm(cx)))
            .on_action(on(&self.menu, |menu, _: &Dismiss, cx| menu.dismiss(cx)))
            .on_action(on(&self.menu, |menu, jump: &Jump, cx| menu.jump(jump.to, cx)))
            .on_key_down({
                let menu = self.menu.clone();
                move |event: &KeyDownEvent, _, cx| {
                    if let Some(letter) = typed_letter(event) {
                        menu.update(cx, |menu, cx| menu.seek(letter, cx));
                    }
                }
            });
        let Some((depth, level)) = menu.level_on(self.index) else { return panel };
        let area = Bounds::new(point(px(0.), px(0.)), window.viewport_size());
        let pixels = self.pixels();
        let replace_small = cx.global::<Settings>().replace_small_icons;
        let icons = has_icons(&level.entries);
        let mark = Mark::default();
        let fill = Fill { radius: theme.radius.small, color: theme.results.selected };
        let indicator = mark.indicator(fill);
        let rows = level.entries.iter().enumerate().map(|(row, entry)| match entry {
            MenuEntry::Item(item) => {
                let look =
                    Look { selected: level.selected == Some(row), icons, pixels, replace_small, area, fits: self.fits };
                item_row(&self.menu, (depth, row), item, look, &mark, theme, cx)
            }
            MenuEntry::Heading { label, button } => heading(label.clone(), button.clone(), theme),
            MenuEntry::Separator => separator(theme),
        });
        let reveal = level.selected.filter(|_| depth == menu.active).map(|row| row_span(&level.entries, row));
        panel
            .child(
                div()
                    .id(AREA_ID)
                    .relative()
                    .size_full()
                    .on_mouse_move(on_level(&self.menu, self.index, Menu::rest))
                    .child(glide_rows(ROWS_ID, self.scroll.clone(), rests(&level.entries, window)))
                    .child(scroll_to_span(REVEAL_ID, self.scroll.clone(), reveal, menu.reveal))
                    .child(indicator)
                    .child(
                        div()
                            .id(ROWS_ID)
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .p(rem(FILL_GAP))
                            .flex()
                            .flex_col()
                            .gap(rem(FILL_GAP))
                            .children(rows),
                    ),
            )
            .children(seam.map(|edge| divider(edge, theme.launcher.divider)))
    }
}

fn typed_letter(event: &KeyDownEvent) -> Option<char> {
    let keystroke = &event.keystroke;
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    let mut chars = keystroke.key_char.as_deref()?.chars();
    let letter = chars.next().filter(|letter| !letter.is_whitespace() && !letter.is_control())?;
    chars.next().is_none().then_some(letter)
}

struct Look {
    selected: bool,
    icons: bool,
    pixels: u32,
    replace_small: bool,
    area: Bounds<gpui::Pixels>,
    fits: bool,
}

fn on<A: gpui::Action>(
    menu: &Entity<Menu>,
    job: impl Fn(&mut Menu, &A, &mut Context<Menu>) + 'static,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    let menu = menu.clone();
    move |action, _, cx| menu.update(cx, |menu, cx| job(menu, action, cx))
}

fn on_level<E>(
    menu: &Entity<Menu>,
    index: usize,
    job: fn(&mut Menu, usize, &mut Context<Menu>),
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    let menu = menu.clone();
    move |_, _, cx| {
        menu.update(cx, |menu, cx| {
            if let Some((depth, _)) = menu.level_on(index) {
                job(menu, depth, cx);
            }
        })
    }
}

fn item_row(
    menu: &Entity<Menu>,
    (depth, row): (usize, usize),
    item: &MenuItem,
    look: Look,
    mark: &Mark,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let colors = theme.results;
    let enabled = item.enabled();
    let under = Under::marked(theme.launcher.background, colors.selected, mark);
    let label = item.label.clone();
    let tip = Tip::new(label.clone()).inside(look.area);
    div()
        .id(("menu-item", row))
        .flex_none()
        .h(rem(CONTROL))
        .px(rem(INDENT))
        .flex()
        .items_center()
        .gap(rem(LABEL_GAP))
        .rounded(rem(theme.radius.small))
        .block_mouse_except_scroll()
        .relative()
        .when(look.selected, |element| element.child(mark.spot()))
        .when(enabled, |element| element.cursor_pointer())
        .when(!enabled, |element| element.text_color(colors.empty))
        .on_mouse_move({
            let menu = menu.clone();
            move |_, _, cx| menu.update(cx, |menu, cx| menu.point(depth, row, cx))
        })
        .on_click({
            let menu = menu.clone();
            move |_, _, cx| menu.update(cx, |menu, cx| menu.activate(depth, row, cx))
        })
        .when(look.icons, |element| element.child(icon(&item.icon, &look, theme, cx)))
        .child(faded_unless_fits(("menu-label", row), label, tip, NAME_LINE, under, look.fits).flex_1().min_w_0())
        .children(item.hint.clone().map(|hint| {
            div()
                .flex_none()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(colors.shortcut)
                .child(hint)
        }))
        .children(item.submenu.as_ref().map(|_| glyph(assets::CHEVRON_RIGHT, GLYPH, colors.icon)))
        .into_any_element()
}

fn icon(icon: &MenuIcon, look: &Look, theme: &Theme, cx: &App) -> AnyElement {
    let color = theme.results.icon;
    match icon {
        MenuIcon::Blank => div().flex_none().size(rem(GLYPH)).into_any_element(),
        MenuIcon::Glyph(path) => glyph(path, GLYPH, color).into_any_element(),
        MenuIcon::Path { path, dir } => {
            let key = IconKey { source: IconSource::for_path(path, *dir), pixels: look.pixels };
            match cx.global::<Icons>().get(&key, look.replace_small) {
                Some(image) => img(image).size(rem(GLYPH)).flex_none().into_any_element(),
                None => glyph(if *dir { assets::FOLDER } else { assets::FILE }, GLYPH, color).into_any_element(),
            }
        }
        MenuIcon::Picture(image) => img(image.clone()).size(rem(GLYPH)).flex_none().into_any_element(),
    }
}

fn heading(label: SharedString, button: Option<(&'static str, super::Choose)>, theme: &Theme) -> AnyElement {
    div()
        .flex_none()
        .h(rem(CONTROL))
        .px(rem(INDENT))
        .flex()
        .items_center()
        .justify_between()
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.results.heading)
        .child(label)
        .children(button.map(|(path, choose)| {
            icon_button("menu-heading-button", path, theme).on_click(move |_, _, cx| {
                let choose = choose.clone();
                cx.defer(move |cx| choose(cx));
            })
        }))
        .into_any_element()
}

fn separator(theme: &Theme) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .h(rem(SEPARATOR))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(rem(FILL_GAP))
                .h(rem(FILL_GAP))
                .child(divider(Edge::Top, theme.launcher.divider)),
        )
        .into_any_element()
}
