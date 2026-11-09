use super::{AppWindow, Page, Section, View};
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::platform::Caption;
use crate::search::{Builtin, Filter};
use crate::settings::Settings;
use crate::theme::{Corners, Theme, rem};
use crate::ui::{
    DETAIL_LINE, DETAIL_SIZE, Dismiss, Edge, FILL_GAP, GLYPH, GUTTER, HINT_GAP, INDENT, INSET, NAME_LINE, NAME_SIZE,
    TITLE_LINE, TITLE_SIZE, TOP_BAR, baseline_shift, divider, glyph, note_click, window_shape,
};
use gpui::{
    AnyElement, ClickEvent, Context, Div, ElementId, Hsla, IntoElement, MouseButton, Render, SharedString, Stateful,
    Window, div, prelude::*,
};

pub(super) const WIDTH: f32 = 1008.0;
pub(super) const HEIGHT: f32 = 692.0;
const TITLE_BAR: f32 = 40.0;
const CAPTION: f32 = 48.0;
const SIDEBAR: f32 = 208.0;
pub(super) const CAPTION_AREAS: Caption = Caption { height: TITLE_BAR, button: CAPTION };
const NAV_ROW: f32 = 32.0;
pub(in crate::app_window) const BOTTOM_BAR: f32 = NAV_ROW + 2.0 * INSET;

impl Render for AppWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        let maximized = window.is_maximized();
        let corners = if maximized { Corners::default() } else { theme.windows.app.corners };
        let page: AnyElement = match self.view {
            View::Search => self.search.clone().into_any_element(),
            View::Settings => self.settings.clone().into_any_element(),
        };
        window_shape(div(), corners)
            .id("app-window")
            .key_context(super::CONTEXT)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| window.focus(&this.home_focus(cx), cx)))
            .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.hide(cx)))
            .size_full()
            .flex()
            .font_family(theme.font.clone())
            .text_size(rem(NAME_SIZE))
            .line_height(rem(NAME_LINE))
            .text_color(theme.results.name)
            .child(self.sidebar(theme, language, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.app.background)
                    .child(caption_bar(maximized, theme))
                    .child(div().flex_1().min_h_0().flex().child(page)),
            )
    }
}

fn caption_bar(maximized: bool, theme: &Theme) -> impl IntoElement {
    let colors = theme.caption;
    let caption = |id: &'static str, icon: &'static str, hover: Hsla| {
        div()
            .id(id)
            .w(rem(CAPTION))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .block_mouse_except_scroll()
            .hover(|style| style.bg(hover))
            .child(glyph(icon, GLYPH, colors.glyph))
    };
    let restore = if maximized { assets::RESTORE } else { assets::MAXIMIZE };
    div()
        .h(rem(TITLE_BAR))
        .flex_none()
        .flex()
        .child(div().flex_1().h_full())
        .child(caption("minimize", assets::MINIMIZE, colors.hover))
        .child(caption("maximize", restore, colors.hover))
        .child(caption("close", assets::CLOSE, colors.close_hover))
}

pub(in crate::app_window) fn top_bar(content: impl IntoElement, theme: &Theme, window: &Window) -> Div {
    let shift = baseline_shift(NAME_SIZE, TITLE_SIZE, theme, window);
    div()
        .h(rem(TOP_BAR))
        .flex_none()
        .flex()
        .items_center()
        .px(rem(GUTTER))
        .text_size(rem(TITLE_SIZE))
        .line_height(rem(TITLE_LINE))
        .text_color(theme.app.title)
        .child(div().relative().top(shift).flex_1().min_w_0().overflow_hidden().child(content))
}

impl AppWindow {
    fn sidebar(&self, theme: &Theme, language: Language, cx: &Context<Self>) -> impl IntoElement {
        let strings = language.strings();
        let kinds: Vec<_> = match self.view {
            View::Search => {
                let filters = &cx.global::<Settings>().filters;
                let search = self.search.read(cx);
                let (chosen, counts) = (search.filters().clone(), search.counts().cloned());
                let counts = counts.as_ref();
                let count = |count: Option<usize>| count.map(|count| language.number(count));
                let all = (None, SharedString::from(strings.kind_all), count(counts.map(|counts| counts.any_kind)));
                let choices = std::iter::once(all).chain(
                    filters.iter().enumerate().filter(|(_, filter)| filter.enabled).enumerate().map(
                        |(kind, (position, filter))| {
                            let found = counts.and_then(|counts| counts.kinds.get(kind).copied());
                            (Some(position), filter_label(filter, strings), count(found))
                        },
                    ),
                );
                choices
                    .map(|(filter, label, count)| {
                        let current = filter.map_or(chosen.is_empty(), |position| chosen.contains(&position));
                        let mark = if current { Mark::Current } else { Mark::Idle };
                        nav_item(("filter", filter.map_or(0, |position| position + 1)), label, count, mark, theme)
                            .on_mouse_down(MouseButton::Left, |event, _, _| note_click(event))
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                this.choose_filter(filter, event.modifiers().control, window, cx)
                            }))
                    })
                    .collect()
            }
            View::Settings => {
                let settings = self.settings.read(cx);
                let current = settings.section();
                let found: Vec<usize> = Section::ALL.into_iter().map(|section| settings.found(section)).collect();
                Section::ALL
                    .into_iter()
                    .zip(found)
                    .map(|(section, found)| {
                        let mark = match (section == current, found > 0) {
                            (true, _) => Mark::Current,
                            (false, true) => Mark::Found,
                            (false, false) => Mark::Idle,
                        };
                        let count = (found > 0).then(|| language.number(found));
                        nav_item(("section", section as usize), section.label(strings), count, mark, theme).on_click(
                            cx.listener(move |this, _, window, cx| this.open_page(Page::Settings(section), window, cx)),
                        )
                    })
                    .collect()
            }
        };
        let settings = match self.view {
            View::Search => nav_item("settings", strings.settings, None, Mark::Idle, theme)
                .on_click(cx.listener(|this, _, window, cx| this.open_page(Page::SETTINGS, window, cx))),
            View::Settings => nav_item("search", strings.back_to_search, None, Mark::Idle, theme)
                .on_click(cx.listener(|this, _, window, cx| this.open_page(Page::Search, window, cx))),
        };
        div()
            .relative()
            .w(rem(SIDEBAR))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.sidebar.background)
            .child(
                div()
                    .id("title")
                    .h(rem(TITLE_BAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(rem(GUTTER))
                    .text_size(rem(DETAIL_SIZE))
                    .line_height(rem(DETAIL_LINE))
                    .text_color(theme.sidebar.title)
                    .child(super::TITLE),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(rem(FILL_GAP))
                    .px(rem(INSET))
                    .pt(rem((TOP_BAR - NAV_ROW) / 2.0))
                    .children(kinds),
            )
            .child(div().flex_none().px(rem(INSET)).pb(rem(INSET)).child(settings))
            .child(divider(Edge::Right, theme.sidebar.divider))
    }
}

pub(in crate::app_window) fn status_bar(theme: &Theme) -> Div {
    div()
        .h(rem(BOTTOM_BAR))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .gap(rem(HINT_GAP))
        .px(rem(GUTTER))
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .text_color(theme.status.text)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Idle,
    Current,
    Found,
}

fn nav_item(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    count: Option<String>,
    mark: Mark,
    theme: &Theme,
) -> Stateful<Div> {
    let colors = theme.nav;
    let current = mark == Mark::Current;
    let text = match mark {
        Mark::Idle => colors.text,
        Mark::Current => colors.current_text,
        Mark::Found => colors.found,
    };
    div()
        .id(id)
        .h(rem(NAV_ROW))
        .flex_none()
        .flex()
        .items_center()
        .gap(rem(INDENT))
        .px(rem(INDENT))
        .rounded(rem(theme.radius.large))
        .cursor_pointer()
        .block_mouse_except_scroll()
        .text_color(text)
        .when(current, |item| item.bg(colors.current))
        .when(!current, |item| item.hover(|style| style.bg(colors.hover).text_color(colors.hover_text)))
        .child(div().flex_1().min_w_0().overflow_hidden().child(label.into()))
        .children(count.map(|count| div().flex_none().text_color(colors.count).child(count)))
}

pub(in crate::app_window) fn filter_label(filter: &Filter, strings: &Strings) -> SharedString {
    let builtin = match filter.builtin {
        Some(Builtin::Folders) => strings.kind_folders,
        Some(Builtin::Files) => strings.kind_files,
        Some(Builtin::Apps) => strings.kind_apps,
        Some(Builtin::Documents) => strings.kind_documents,
        Some(Builtin::Images) => strings.kind_images,
        Some(Builtin::Audio) => strings.kind_audio,
        Some(Builtin::Video) => strings.kind_video,
        Some(Builtin::Archives) => strings.kind_archives,
        None if filter.name.trim().is_empty() => return filter.keyword.clone().into(),
        None => return filter.name.clone().into(),
    };
    builtin.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_lands_on_whole_pixels_at_common_scales() {
        crate::ui::assert_whole_pixels(&[
            WIDTH,
            HEIGHT,
            TITLE_BAR,
            CAPTION,
            SIDEBAR,
            NAV_ROW,
            BOTTOM_BAR,
            (TOP_BAR - NAV_ROW) / 2.0,
        ]);
    }
}
