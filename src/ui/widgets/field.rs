use super::super::tokens::*;
use super::{cap_shift, frame};
use crate::input::{FIELD_CONTEXT, NextField, PreviousField, TextInput};
use crate::settings::Shortcut;
use crate::theme::{Theme, rem};
use gpui::{Div, ElementId, Entity, Pixels, SharedString, Stateful, Window, div, prelude::*};

pub(crate) fn text_field(input: Entity<TextInput>, focused: bool, width: f32, theme: &Theme) -> Div {
    let radius = theme.radius.large;
    let colors = theme.field;
    div()
        .relative()
        .flex_none()
        .w(rem(width))
        .h(rem(CONTROL))
        .px(rem(FILL_GAP + INDENT))
        .flex()
        .items_center()
        .overflow_hidden()
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .key_context(FIELD_CONTEXT)
        .on_action(|_: &NextField, window, cx| window.focus_next(cx))
        .on_action(|_: &PreviousField, window, cx| window.focus_prev(cx))
        .child(div().flex_1().min_w_0().child(input))
        .child(frame(radius, if focused { colors.focus } else { colors.frame }))
}

pub(crate) fn shortcut_field(
    id: impl Into<ElementId>,
    shortcut: Option<&Shortcut>,
    recording: Option<&'static str>,
    empty: &'static str,
    theme: &Theme,
    window: &Window,
) -> Stateful<Div> {
    let radius = theme.radius.large;
    let colors = theme.field;
    let shift = cap_shift(DETAIL_SIZE, theme, window);
    let field = div()
        .id(id)
        .relative()
        .flex_none()
        .w(rem(CONTROL_WIDTH))
        .h(rem(CONTROL))
        .flex()
        .items_center()
        .gap(rem(KEY_GAP))
        .rounded(rem(radius))
        .cursor_pointer()
        .block_mouse_except_scroll()
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .hover(|style| style.bg(theme.button.hover));
    let prompt = |field: Stateful<Div>, text: &'static str| {
        field.px(rem(FILL_GAP + INDENT)).text_color(theme.input.placeholder).child(text)
    };
    let field = match (recording, shortcut) {
        (Some(text), _) => prompt(field, text),
        (None, Some(shortcut)) => {
            let caps = shortcut.parts().into_iter().map(|part| key_cap(part, shift, theme));
            field.p(rem(FILL_GAP)).children(caps)
        }
        (None, None) => prompt(field, empty),
    };
    field.child(frame(radius, if recording.is_some() { colors.focus } else { colors.frame }))
}

fn key_cap(text: impl Into<SharedString>, shift: Pixels, theme: &Theme) -> Div {
    div()
        .relative()
        .h(rem(KEY_CAP))
        .min_w(rem(KEY_CAP))
        .px(rem(KEY_CAP_PADDING))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.results.name)
        .child(div().relative().top(shift).child(text.into()))
        .child(frame(theme.radius.small, theme.keys.frame))
}
