use super::FiltersRenumbered;
use super::SettingsPage;
use super::form::Item;
use super::row::Grip;
use super::{ROW, ROW_PADDING, change};
use crate::assets;
use crate::settings::{List, Settings, after_move, after_removal};
use crate::theme::{Theme, pixels, rem};
use crate::ui::{
    DETAIL_LINE, DETAIL_SIZE, Edge, GLYPH, INDENT, LABEL_GAP, NAME_LINE, NAME_SIZE, drop_marker, glyph, icon_button,
    idle_icon_button, sample,
};
use gpui::{Context, DispatchPhase, IntoElement, MouseUpEvent, Pixels, SharedString, Window, canvas, div, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

const LIFTED_OPACITY: f32 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app_window) enum Renumber {
    Moved { from: usize, to: usize },
    Removed(usize),
}

impl Renumber {
    pub(in crate::app_window) fn apply(self, position: usize) -> Option<usize> {
        match self {
            Self::Moved { from, to } => Some(after_move(position, from, to)),
            Self::Removed(removed) => after_removal(position, removed),
        }
    }
}

struct Lifted(Item);

struct Ghost {
    label: SharedString,
    hint: SharedString,
    width: Pixels,
}

impl Render for Ghost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let colors = theme.tooltip;
        let indent = pixels(INDENT, window);
        let hint = (!self.hint.is_empty()).then(|| {
            div()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(theme.settings.hint)
                .child(self.hint.clone())
        });
        div()
            .relative()
            .left(-indent)
            .w(self.width + indent * 2.)
            .min_h(rem(ROW))
            .px(rem(INDENT))
            .py(rem(ROW_PADDING))
            .flex()
            .items_center()
            .gap(rem(LABEL_GAP))
            .font_family(theme.font.clone())
            .text_size(rem(NAME_SIZE))
            .line_height(rem(NAME_LINE))
            .text_color(colors.text)
            .child(sample(theme.radius.large, colors.background, colors.frame))
            .child(glyph(assets::GRIP, GLYPH, theme.icon_button.hover_glyph))
            .child(div().flex_1().min_w_0().flex().flex_col().child(self.label.clone()).children(hint))
    }
}

impl Item {
    fn position(self) -> usize {
        match self {
            Self::Filter(position)
            | Self::Action(position)
            | Self::Command(position)
            | Self::Web(position)
            | Self::Favorite(position)
            | Self::MenuTool(position) => position,
        }
    }

    fn at(self, position: usize) -> Self {
        match self {
            Self::Filter(_) => Self::Filter(position),
            Self::Action(_) => Self::Action(position),
            Self::Command(_) => Self::Command(position),
            Self::Web(_) => Self::Web(position),
            Self::Favorite(_) => Self::Favorite(position),
            Self::MenuTool(_) => Self::MenuTool(position),
        }
    }

    pub(super) fn ordered(self) -> bool {
        match self {
            Self::Filter(_) | Self::Action(_) | Self::Favorite(_) | Self::MenuTool(_) => true,
            Self::Command(_) | Self::Web(_) => false,
        }
    }

    pub(super) fn list(self) -> List {
        match self {
            Self::Filter(_) => List::Filters,
            Self::Action(_) => List::Actions,
            Self::Command(_) => List::Commands,
            Self::Web(_) => List::Web,
            Self::Favorite(_) => List::Favorites,
            Self::MenuTool(_) => List::MenuTools,
        }
    }

    fn same_list(self, other: Self) -> bool {
        self.list() == other.list()
    }
}

impl SettingsPage {
    pub(super) fn steps(&self, item: Item, theme: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let position = item.position();
        let count = cx.global::<Settings>().count(item.list());
        let step = |id: &'static str, path: &'static str, target: Option<usize>| match target {
            Some(to) => icon_button((id, position), path, theme)
                .on_click(cx.listener(move |this, _, _, cx| this.move_item(item, to, cx))),
            None => idle_icon_button((id, position), path, theme),
        };
        div()
            .flex_none()
            .w(rem(GLYPH * 2.0 + LABEL_GAP))
            .flex()
            .items_center()
            .gap(rem(LABEL_GAP))
            .child(step("move-up", assets::ARROW_UP, position.checked_sub(1)))
            .child(step("move-down", assets::ARROW_DOWN, Some(position + 1).filter(|&to| to < count)))
    }

    pub(super) fn grip(
        &self,
        item: Item,
        (label, hint): (SharedString, SharedString),
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Grip {
        let lifted = self.form.lifted.filter(|lifted| lifted.same_list(item) && cx.has_active_drag());
        let mark = lifted.filter(|&lifted| lifted != item).map(|lifted| {
            let edge = if item.position() < lifted.position() { Edge::Top } else { Edge::Bottom };
            let this = cx.weak_entity();
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .opacity(0.)
                .drag_over::<Lifted>(|style, _, _, _| style.opacity(1.))
                .on_drop(move |lifted: &Lifted, _, cx| {
                    let _ = this.update(cx, |this, cx| this.move_item(lifted.0, item.position(), cx));
                })
                .child(drop_marker(edge, theme.settings.label))
        });
        let width = Rc::new(Cell::new(Pixels::ZERO));
        let measured = width.clone();
        let this = cx.weak_entity();
        Box::new(move |header| {
            header
                .when(lifted == Some(item), |header| header.opacity(LIFTED_OPACITY))
                .child(
                    canvas(move |bounds, _, _| measured.set(bounds.size.width), |_, _, _, _| {}).absolute().size_full(),
                )
                .on_drag(Lifted(item), move |_, _, _, cx| {
                    let _ = this.update(cx, |this, cx| this.lift(item, cx));
                    cx.new(|_| Ghost { label: label.clone(), hint: hint.clone(), width: width.get() })
                })
                .children(mark)
        })
    }

    pub(super) fn release_on_mouse_up(&self, cx: &Context<Self>) -> Option<impl IntoElement + use<>> {
        self.form.lifted?;
        let this = cx.weak_entity();
        let release = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let this = this.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = this.update(cx, |this, cx| this.release(cx));
                    }
                });
            },
        );
        Some(release.absolute().size_full())
    }

    fn lift(&mut self, item: Item, cx: &mut Context<Self>) {
        self.form.lifted = Some(item);
        self.menus.close();
        self.stop_recording(cx);
        self.stop_editing(cx);
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if self.form.lifted.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn move_item(&mut self, item: Item, to: usize, cx: &mut Context<Self>) {
        let (list, from) = (item.list(), item.position());
        let mut moved = false;
        change(cx, |settings| moved = settings.move_item(list, from, to));
        if !moved {
            return;
        }
        self.renumber(list, Renumber::Moved { from, to }, cx);
        self.stop_recording(cx);
        cx.notify();
    }

    pub(super) fn remove_item(&mut self, item: Item, cx: &mut Context<Self>) {
        let (list, removed) = (item.list(), item.position());
        let mut gone = false;
        change(cx, |settings| gone = settings.remove_item(list, removed));
        if !gone {
            return;
        }
        self.renumber(list, Renumber::Removed(removed), cx);
        self.stop_recording(cx);
        self.rebuild_fields(cx);
    }

    fn renumber(&mut self, list: List, renumber: Renumber, cx: &mut Context<Self>) {
        if let Some(editing) = self.form.editing.filter(|editing| editing.list() == list) {
            self.form.editing = renumber.apply(editing.position()).map(|position| editing.at(position));
        }
        if list == List::Filters {
            cx.emit(FiltersRenumbered(renumber));
        }
    }
}
