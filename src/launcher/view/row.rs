use super::{SHORTCUTS, extra_icon_side, metrics, row_height};
use crate::assets;
use crate::index::{Hit, Snippet};
use crate::launcher::tools::find;
use crate::launcher::{Forget, Launcher, Mode};
use crate::settings::{Settings, Shortcut};
use crate::theme::{Theme, rem};
use crate::ui::{
    DETAIL_LINE, DETAIL_SIZE, Entry, Extra, Listing, Mark, NAME_LINE, ResultList, Tip, TipAnchor, Tool, Under, faded,
    glyph, highlighted_name, icon_button, icon_tip, note_click, path_label, slot_width, snippet_text,
};
use gpui::{
    AnyElement, Bounds, ClickEvent, Context, Div, FontWeight, HighlightStyle, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, SharedString, Stateful, StyledText, Window, div, prelude::*,
};
use std::ops::Range;

const DOCKED_NAME: f32 = 160.0;
const SHORTCUT_SIZE: f32 = 13.0;

enum Detail<'a> {
    Snippet(&'a Snippet),
    Path(&'a [Range<usize>]),
    Plain,
}

struct Parts<'a> {
    icon: AnyElement,
    title: &'a str,
    name: AnyElement,
    detail: &'a str,
    shape: Detail<'a>,
    tail: Tail,
}

enum Tail {
    Nothing,
    Shortcut(Shortcut),
    Submenu,
}

impl Launcher {
    pub(super) fn row(
        &self,
        mode: Mode,
        position: usize,
        (area, mark): (Bounds<Pixels>, &Mark),
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let parts = match self.results.entry(position)? {
            Entry::Heading(title) => return Some(heading(mode, self.results.listing(), title, theme)),
            Entry::File(hit) => self.file_parts(mode, hit, theme, cx),
            Entry::Extra(extra) => self.extra_parts(mode, extra, theme, cx),
        };
        let metrics = metrics(mode);
        let colors = theme.results;
        let selected = self.results.is_selected(position);
        let picked = self.results.is_picked(position);
        let base = if picked && !selected { colors.picked } else { theme.launcher.background };
        let under = Under::marked(base, colors.selected, mark);
        let anchor = TipAnchor::default();
        let name_tip = Tip::new(parts.title.to_owned()).inside(area).along(anchor.clone());
        let name = faded("name", parts.name, name_tip, NAME_LINE, under.clone());
        let detail = detail(parts.detail, &parts.shape, area, &anchor, &under, theme);
        let label = label(mode, name, detail);
        let trailing = self.trailing(position, parts.tail, area, theme, window, cx);
        let slot = slot_width(format!("Ctrl+{SHORTCUTS}").into(), SHORTCUT_SIZE, theme, window);
        Some(
            self.pointer(div().id(position), position, cx)
                .w_full()
                .h(rem(row_height(mode, self.results.listing())))
                .px(rem(metrics.inset))
                .block_mouse_except_scroll()
                .child(
                    div()
                        .relative()
                        .size_full()
                        .pl(rem(metrics.indent))
                        .pr(rem(metrics.trailing))
                        .flex()
                        .items_center()
                        .gap(rem(metrics.gap))
                        .rounded(rem(theme.radius.of(metrics.rounding)))
                        .cursor_pointer()
                        .when(picked && !selected, |row| row.bg(base))
                        .when(selected, |row| row.child(mark.spot()))
                        .child(anchor.track())
                        .child(parts.icon)
                        .child(label)
                        .child(div().flex_none().min_w(slot).flex().justify_end().children(trailing)),
                )
                .into_any_element(),
        )
    }

    fn pointer(&self, row: Stateful<Div>, position: usize, cx: &Context<Self>) -> Stateful<Div> {
        row.on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
            this.point_at(position, event.position, window.bounds().origin, cx);
            this.results.drag(event, window, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, _| {
                note_click(event);
                this.results.press(position, event.position);
            }),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| this.click(position, event, cx)))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _, _, cx| {
                let targets = this.results.targets_at(position);
                this.show_actions(position, targets, cx);
            }),
        )
    }

    fn file_parts<'a>(&self, mode: Mode, hit: &'a Hit, theme: &Theme, cx: &Context<Self>) -> Parts<'a> {
        let plain = cx.global::<Settings>().hide_shortcut_extensions;
        Parts {
            icon: self.results.icon(hit, metrics(mode).icon, theme, cx),
            title: hit.label(plain),
            name: highlighted_name(hit, plain, theme).into_any_element(),
            detail: hit.folder(),
            shape: hit.snippet.as_ref().map_or(Detail::Path(&hit.marks), Detail::Snippet),
            tail: Tail::Nothing,
        }
    }

    fn extra_parts<'a>(&self, mode: Mode, extra: &'a Extra, theme: &Theme, cx: &Context<Self>) -> Parts<'a> {
        let style = HighlightStyle { color: Some(theme.results.matched), ..Default::default() };
        let matched = self
            .menu
            .as_ref()
            .and_then(|_| find(&extra.title, self.input.read(cx).text()))
            .filter(|range| !range.is_empty());
        let name = StyledText::new(SharedString::from(extra.title.clone()))
            .with_highlights(matched.map(|range| (range, style)))
            .into_any_element();
        let tail = match extra.tool {
            Tool::Action { index, .. } => {
                let shortcut = cx.global::<Settings>().actions.get(index).and_then(|action| action.shortcut.clone());
                shortcut.map_or(Tail::Nothing, Tail::Shortcut)
            }
            Tool::Shell { submenu: true, .. } => Tail::Submenu,
            Tool::Command(_) | Tool::Web(_) | Tool::Shell { .. } => Tail::Nothing,
        };
        let side = extra_icon_side(mode, self.results.listing());
        let icon = match extra.tool {
            Tool::Shell { index, .. } => self.shell_icon(index, side, theme),
            Tool::Action { .. } | Tool::Command(_) | Tool::Web(_) => ResultList::extra_icon(extra, side, theme),
        };
        Parts { icon, title: &extra.title, name, detail: &extra.detail, shape: Detail::Plain, tail }
    }

    fn trailing(
        &self,
        position: usize,
        tail: Tail,
        area: Bounds<Pixels>,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let colors = theme.results;
        let listing = self.results.listing();
        if self.results.is_selected(position) && listing == Listing::Recent {
            let tip = Tip::keys(&Forget, window, cx).map(|tip| tip.inside(area));
            let forget = icon_tip(icon_button(("forget", position), assets::CLOSE, theme), tip).on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.forget(position, cx);
                },
            ));
            return Some(forget.into_any_element());
        }
        if self.results.is_picked(position) {
            return Some(glyph(assets::CHECK, DETAIL_LINE, colors.shortcut).into_any_element());
        }
        let hint = match (tail, listing) {
            (Tail::Submenu, _) => {
                return Some(glyph(assets::CHEVRON_RIGHT, DETAIL_LINE, colors.shortcut).into_any_element());
            }
            (Tail::Shortcut(shortcut), _) => Some(shortcut.to_string()),
            (Tail::Nothing, Listing::Actions) => None,
            (Tail::Nothing, _) => (position < SHORTCUTS).then(|| format!("Ctrl+{}", position + 1)),
        };
        hint.map(|hint| {
            div()
                .flex_none()
                .text_size(rem(SHORTCUT_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(colors.shortcut)
                .child(hint)
                .into_any_element()
        })
    }
}

fn detail(
    text: &str,
    shape: &Detail<'_>,
    area: Bounds<Pixels>,
    anchor: &TipAnchor,
    under: &Under,
    theme: &Theme,
) -> Option<Stateful<Div>> {
    if text.is_empty() {
        return None;
    }
    let tip = |text: String| Tip::new(text).inside(area).along(anchor.clone());
    let plain = SharedString::from(text.to_owned());
    let line = match shape {
        Detail::Snippet(snippet) => {
            faded("detail", snippet_text(snippet, theme), tip(snippet.text.clone()), DETAIL_LINE, under.clone())
        }
        Detail::Path(marks) => path_label("detail", plain, marks.to_vec(), tip(text.to_owned()), DETAIL_LINE),
        Detail::Plain => faded("detail", plain, tip(text.to_owned()), DETAIL_LINE, under.clone()),
    };
    Some(line.text_size(rem(DETAIL_SIZE)).text_color(theme.results.path))
}

fn label(mode: Mode, name: Stateful<Div>, detail: Option<Stateful<Div>>) -> Div {
    match (mode, detail) {
        (Mode::Launcher, detail) => div().flex_1().min_w_0().flex().flex_col().child(name).children(detail),
        (Mode::Dialog(..) | Mode::Explorer(_), Some(detail)) => div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(rem(metrics(mode).gap))
            .child(name.flex_none().w(rem(DOCKED_NAME)))
            .child(detail.flex_1().min_w_0()),
        (Mode::Dialog(..) | Mode::Explorer(_), None) => {
            div().flex_1().min_w_0().flex().items_center().child(name.flex_1().min_w_0())
        }
    }
}

fn heading(mode: Mode, listing: Listing, title: &str, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .h(rem(row_height(mode, listing)))
        .px(rem(metrics(mode).gutter()))
        .flex()
        .items_center()
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.results.heading)
        .child(SharedString::from(title.to_owned()))
        .into_any_element()
}
