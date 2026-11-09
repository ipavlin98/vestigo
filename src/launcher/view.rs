mod band;
mod folders;
mod row;

pub(super) use folders::{bar_chrome, drop_chrome, folder_menu_at};

use super::{Launcher, Mode, OpenRow, Side};
use crate::assets;
use crate::i18n::Language;
use crate::index::{Hit, Status};
use crate::platform::Layout;
use crate::settings::Settings;
use crate::theme::{Chrome, Rounding, Theme, pixels, rem};
use crate::ui::{Browser, Look, Mark};
use crate::ui::{
    DETAIL_LINE, DETAIL_SIZE, Edge, FOOTER, HINT_GAP, ICON_BUTTON, INDENT, INSET, Key, Listing, NAME_LINE, NAME_SIZE,
    TITLE_LINE, TITLE_SIZE, TOP_BAR, Tip, TogglePreview, action_hints, divider, faded, glyph, hint_slot, icon_button,
    icon_tip, icon_toggle, result_hints, window_shape,
};
use gpui::{
    AnyElement, Bounds, Context, IntoElement, Pixels, Render, SharedString, Window, div, img, point, prelude::*, px,
    size, uniform_list,
};

pub(super) const MAX_ROWS: usize = 8;
pub(super) const SHORTCUTS: usize = 9;

struct Metrics {
    width: f32,
    input: f32,
    query: f32,
    query_line: f32,
    row: f32,
    action_row: f32,
    action_icon: f32,
    inset: f32,
    indent: f32,
    trailing: f32,
    rounding: Rounding,
    icon: f32,
    gap: f32,
    footer: f32,
}

impl Metrics {
    fn gutter(&self) -> f32 {
        self.inset + self.indent
    }
}

const LAUNCHER: Metrics = Metrics {
    width: 680.0,
    input: TOP_BAR,
    query: TITLE_SIZE,
    query_line: TITLE_LINE,
    row: 56.0,
    action_row: 40.0,
    action_icon: 20.0,
    inset: INSET,
    indent: INDENT,
    trailing: 12.0,
    rounding: Rounding::Large,
    icon: 32.0,
    gap: 12.0,
    footer: FOOTER,
};
const DOCKED: Metrics = Metrics {
    width: 560.0,
    input: 40.0,
    query: NAME_SIZE,
    query_line: NAME_LINE,
    row: 36.0,
    action_row: 36.0,
    action_icon: 16.0,
    inset: 4.0,
    indent: INDENT,
    trailing: 8.0,
    rounding: Rounding::Small,
    icon: 16.0,
    gap: 8.0,
    footer: 32.0,
};

fn metrics(mode: Mode) -> &'static Metrics {
    match mode {
        Mode::Launcher => &LAUNCHER,
        Mode::Dialog(..) | Mode::Explorer(_) => &DOCKED,
    }
}

pub(super) fn row_height(mode: Mode, listing: Listing) -> f32 {
    let metrics = metrics(mode);
    match listing {
        Listing::Actions => metrics.action_row,
        Listing::Recent
        | Listing::Folders
        | Listing::Searching
        | Listing::Found(_)
        | Listing::AtLeast(_)
        | Listing::Contents(_)
        | Listing::ContentOff
        | Listing::ContentShort => metrics.row,
    }
}

fn extra_icon_side(mode: Mode, listing: Listing) -> f32 {
    let metrics = metrics(mode);
    match listing {
        Listing::Actions => metrics.action_icon,
        Listing::Recent
        | Listing::Folders
        | Listing::Searching
        | Listing::Found(_)
        | Listing::AtLeast(_)
        | Listing::Contents(_)
        | Listing::ContentOff
        | Listing::ContentShort => metrics.icon,
    }
}

pub(super) fn layout(mode: Mode, rows: usize, row: f32, footer: bool, chrome: Chrome, zoom: f32) -> Layout {
    let (panel_top, panel_height) = panel_span(mode, rows, row, footer);
    let metrics = metrics(mode);
    Layout {
        width: metrics.width,
        height: panel_height_of(mode, MAX_ROWS, metrics.row, true),
        panel_top,
        panel_height,
        grip: if footer && mode == Mode::Launcher { metrics.footer } else { 0.0 },
        grip_end: metrics.gutter() + ICON_BUTTON,
        zoom,
        chrome,
    }
}

fn panel_span(mode: Mode, rows: usize, row: f32, footer: bool) -> (f32, f32) {
    let height = panel_height_of(mode, rows, row, footer);
    match growth(mode) {
        Growth::Down => (0.0, height),
        Growth::Up => (panel_height_of(mode, MAX_ROWS, metrics(mode).row, true) - height, height),
    }
}

fn panel_area(mode: Mode, rows: usize, row: f32, footer: bool, window: &Window) -> Bounds<Pixels> {
    let (top, height) = panel_span(mode, rows, row, footer);
    let width = pixels(metrics(mode).width, window);
    Bounds::new(point(px(0.), pixels(top, window)), size(width, pixels(height, window)))
}

pub(super) fn docked_rows_within(space: f32) -> usize {
    (1..=MAX_ROWS).rev().find(|&rows| stacked_height(&DOCKED, rows, DOCKED.row, true) <= space).unwrap_or(0)
}

fn panel_height_of(mode: Mode, rows: usize, row: f32, footer: bool) -> f32 {
    stacked_height(metrics(mode), rows, row, footer)
}

fn stacked_height(metrics: &Metrics, rows: usize, row: f32, footer: bool) -> f32 {
    let list = if rows == 0 { 0.0 } else { rows as f32 * row + 2.0 * metrics.inset };
    metrics.input + list + if footer { metrics.footer } else { 0.0 }
}

pub(super) fn panel_height() -> f32 {
    panel_height_of(Mode::Launcher, MAX_ROWS, LAUNCHER.row, true)
}

pub(super) fn icon_side(mode: Mode) -> f32 {
    metrics(mode).icon
}

#[derive(Clone, Copy)]
enum Growth {
    Down,
    Up,
}

fn growth(mode: Mode) -> Growth {
    match mode {
        Mode::Launcher | Mode::Dialog(_, Side::Below, _) => Growth::Down,
        Mode::Dialog(_, Side::Above, _) | Mode::Explorer(_) => Growth::Up,
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        let mode = self.placed;
        let rows = self.rows(mode);
        let row = row_height(mode, self.results.listing());
        let footer = self.shows_footer(mode, cx);
        let area = panel_area(mode, rows, row, footer, window);
        let panel = window_shape(div(), bar_chrome(mode, self.joined, theme).corners)
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme.launcher.background)
            .font_family(theme.font.clone())
            .text_size(rem(NAME_SIZE))
            .line_height(rem(NAME_LINE))
            .text_color(theme.results.name);
        let list = (rows > 0).then(|| self.list(mode, rows, area, theme, language, cx));
        let status = *cx.global::<Status>();
        let footer = footer.then(|| match mode {
            Mode::Launcher => self.footer(area, theme, language, status, window, cx).into_any_element(),
            Mode::Dialog(..) | Mode::Explorer(_) => self.path_band(mode, area, theme, language, status),
        });
        let beyond = div().flex_1().bg(theme.launcher.background);

        let root = div()
            .id("launcher")
            .key_context(super::CONTEXT)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::pick_next))
            .on_action(cx.listener(Self::pick_previous))
            .on_action(cx.listener(Self::confirm_selected))
            .on_action(cx.listener(Self::reveal_selected))
            .on_action(cx.listener(Self::show_actions_selected))
            .on_action(cx.listener(Self::enter_folder))
            .on_action(cx.listener(Self::leave_folder))
            .on_action(cx.listener(Self::enter_containing))
            .on_action(cx.listener(Self::jump))
            .on_action(cx.listener(Self::set_mark))
            .on_action(cx.listener(Self::go_to_mark))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::run_action))
            .on_action(cx.listener(Self::forget_selected))
            .on_action(cx.listener(Self::quick_switch))
            .on_action(cx.listener(Self::show_favorites))
            .on_action(cx.listener(Self::show_recent))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::open_row))
            .size_full()
            .flex()
            .flex_col();
        match growth(mode) {
            Growth::Down => root
                .child(
                    panel
                        .child(self.query_row(mode, area, theme, language, window, cx))
                        .children(list)
                        .children(footer),
                )
                .child(beyond),
            Growth::Up => root.child(beyond).child(
                panel.children(footer).children(list).child(self.query_row(mode, area, theme, language, window, cx)),
            ),
        }
    }
}

impl Launcher {
    fn open_row(&mut self, row: &OpenRow, _: &mut Window, cx: &mut Context<Self>) {
        self.confirm(row.position, false, cx);
    }

    fn shell_icon(&self, index: usize, side: f32, theme: &Theme) -> AnyElement {
        let Some((image, width)) = self.shell_row(index).and_then(|row| row.icon.clone()) else {
            return glyph(assets::MENU_ITEM, side, theme.results.icon).into_any_element();
        };
        let picture = img(image).size(rem(width as f32 / self.results.ratio())).flex_none();
        div().size(rem(side)).flex_none().flex().items_center().justify_center().child(picture).into_any_element()
    }
    fn query_row(
        &self,
        mode: Mode,
        area: Bounds<Pixels>,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let metrics = metrics(mode);
        let hint = self.results.selected().and_then(|hit| action_label(language, mode, hit));
        let previews = cx.global::<Settings>().preview.launcher;
        let preview = (mode == Mode::Launcher).then(|| {
            icon_tip(
                icon_toggle("preview", assets::PREVIEW, previews, theme),
                Tip::keys(&TogglePreview, window, cx).map(|tip| tip.inside(area).on_left()),
            )
            .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(&TogglePreview, window, cx)))
        });
        let scope = self.menu.as_ref().map(|menu| {
            let title = menu.title().map_or(SharedString::from(language.strings().actions_status), |title| {
                SharedString::from(title.to_owned())
            });
            div()
                .relative()
                .flex_none()
                .h(rem(metrics.query_line))
                .pr(rem(metrics.gap))
                .text_size(rem(metrics.query))
                .line_height(rem(metrics.query_line))
                .text_color(theme.input.scope)
                .child(title)
                .child(divider(Edge::Right, theme.launcher.divider))
        });
        div()
            .h(rem(metrics.input))
            .flex_none()
            .flex()
            .items_center()
            .gap(rem(metrics.gap))
            .px(rem(metrics.gutter()))
            .children(scope)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_size(rem(metrics.query))
                    .line_height(rem(metrics.query_line))
                    .child(self.input.clone()),
            )
            .children(hint.map(|label| hint_slot(&[Key::Enter], label, &action_labels(language), theme, window)))
            .children(matches!(mode, Mode::Dialog(..)).then(|| self.folder_buttons(mode, theme, cx)))
            .children(preview)
    }

    fn list(
        &self,
        mode: Mode,
        rows: usize,
        area: Bounds<Pixels>,
        theme: &Theme,
        language: Language,
        cx: &Context<Self>,
    ) -> AnyElement {
        let metrics = metrics(mode);
        let row = row_height(mode, self.results.listing());
        let container =
            div().flex_none().relative().h(rem(rows as f32 * row + 2.0 * metrics.inset)).py(rem(metrics.inset));
        let divider = match growth(mode) {
            Growth::Down => divider(Edge::Top, theme.launcher.divider),
            Growth::Up => divider(Edge::Bottom, theme.launcher.divider),
        };
        if self.results.is_empty() {
            let message = self.results.empty_message(language);
            return container
                .child(
                    div()
                        .h(rem(row))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.results.empty)
                        .child(message),
                )
                .child(divider)
                .into_any_element();
        }
        let mark = Mark::default();
        let look = Look { radius: theme.radius.of(metrics.rounding), color: theme.results.selected };
        let indicator = mark.indicator(look);
        let results = div().relative().size_full().child(self.results.glide(row)).child(indicator).child(
            uniform_list(
                "results",
                self.results.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                    let theme = cx.global::<Theme>();
                    range
                        .filter_map(|position| this.row(mode, position, (area, &mark), theme, window, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(self.results.scroll())
            .size_full(),
        );
        container.child(results).child(divider).into_any_element()
    }

    fn footer(
        &self,
        area: Bounds<Pixels>,
        theme: &Theme,
        language: Language,
        status: Status,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let status = self.results.status(status, language);
        let settings = icon_button("settings", assets::SETTINGS, theme)
            .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)));
        div()
            .relative()
            .h(rem(LAUNCHER.footer))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(rem(HINT_GAP))
            .px(rem(LAUNCHER.gutter()))
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(theme.status.text)
            .child(divider(Edge::Top, theme.launcher.divider))
            .child(
                faded("status", status.clone(), Tip::new(status).inside(area), DETAIL_LINE, theme.launcher.background)
                    .flex_1()
                    .min_w_0(),
            )
            .children(match self.results.listing() {
                Listing::Actions => Some(action_hints(language.strings(), theme, window).into_any_element()),
                Listing::Recent
                | Listing::Folders
                | Listing::Searching
                | Listing::Found(_)
                | Listing::AtLeast(_)
                | Listing::Contents(_)
                | Listing::ContentOff
                | Listing::ContentShort => (!self.results.hits().is_empty())
                    .then(|| result_hints(language.strings(), theme, window).into_any_element()),
            })
            .child(settings)
    }
}

fn action_labels(language: Language) -> [&'static str; 3] {
    let strings = language.strings();
    [strings.go_to, strings.open, strings.choose]
}

fn action_label(language: Language, mode: Mode, hit: &Hit) -> Option<&'static str> {
    let strings = language.strings();
    match (mode, hit.dir) {
        (Mode::Launcher, _) => None,
        (Mode::Dialog(..) | Mode::Explorer(_), true) => Some(strings.go_to),
        (Mode::Explorer(_), false) => Some(strings.open),
        (Mode::Dialog(..), false) => Some(strings.choose),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_docked_list_keeps_only_the_rows_that_fit_beside_the_dialog() {
        assert_eq!(docked_rows_within(368.0), 8);
        assert_eq!(docked_rows_within(1000.0), 8);
        assert_eq!(docked_rows_within(367.0), 7);
        assert_eq!(docked_rows_within(116.0), 1);
        assert_eq!(docked_rows_within(115.0), 0);
    }

    #[test]
    fn layout_lands_on_whole_pixels_at_common_scales() {
        for metrics in [&LAUNCHER, &DOCKED] {
            let lengths = [
                metrics.width,
                metrics.input,
                metrics.query_line,
                metrics.row,
                metrics.action_row,
                metrics.action_icon,
                metrics.inset,
                metrics.indent,
                metrics.trailing,
                metrics.icon,
                metrics.gap,
                metrics.footer,
            ];
            crate::ui::assert_whole_pixels(&lengths);
        }
    }
}
