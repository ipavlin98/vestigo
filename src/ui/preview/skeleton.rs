use super::super::tokens::DETAIL_LINE;
use super::load::{ICON, Kind};
use crate::platform;
use crate::theme::{Theme, rem};
use gpui::{
    Animation, AnimationExt, AnyElement, Div, ElementId, IntoElement, div, ease_out_quint, prelude::*,
    pulsating_between,
};
use std::time::Duration;

const BAR: f32 = 8.0;
const TEXT_BARS: [f32; 9] = [288.0, 256.0, 272.0, 192.0, 288.0, 240.0, 264.0, 208.0, 128.0];
pub(super) const SIZE_BAR: f32 = 64.0;
pub(super) const DATE_BAR: f32 = 128.0;
const APPEAR: Duration = Duration::from_millis(320);
const BREATH: Duration = Duration::from_millis(1600);
const ARRIVAL: Duration = Duration::from_millis(160);
const LOW_OPACITY: f32 = 0.5;
const HIGH_OPACITY: f32 = 1.0;

pub(super) fn visual(kind: Kind, theme: &Theme) -> AnyElement {
    let block = div().flex_none().rounded(rem(theme.radius.large)).bg(theme.results.loading);
    let shape = match kind {
        Kind::Image => block.size_full(),
        Kind::Text => div().size_full().flex().flex_col().children(TEXT_BARS.map(|width| line(width, theme))),
        Kind::Other => block.size(rem(ICON)),
    };
    breathing(shape, "preview-skeleton")
}

pub(super) fn value(width: f32, row: usize, theme: &Theme) -> AnyElement {
    breathing(line(width, theme), ("preview-skeleton-fact", row))
}

pub(super) fn arriving(element: Div, id: impl Into<ElementId>) -> AnyElement {
    element
        .with_animation(id, Animation::new(ARRIVAL).with_easing(ease_out_quint()), |element, opacity| {
            element.opacity(opacity)
        })
        .into_any_element()
}

fn line(width: f32, theme: &Theme) -> Div {
    div().flex_none().h(rem(DETAIL_LINE)).flex().items_center().child(bar(width, theme))
}

fn bar(width: f32, theme: &Theme) -> Div {
    div().flex_none().w(rem(width)).max_w_full().h(rem(BAR)).rounded(rem(theme.radius.small)).bg(theme.results.loading)
}

fn breathing(element: Div, id: impl Into<ElementId>) -> AnyElement {
    let rest = (LOW_OPACITY + HIGH_OPACITY) / 2.0;
    let mut animations = vec![Animation::new(APPEAR).with_easing(move |delta| rest * delta * delta)];
    if !platform::motion_reduced() {
        animations.push(Animation::new(BREATH).repeat().with_easing(pulsating_between(LOW_OPACITY, HIGH_OPACITY)));
    }
    element.with_animations(id, animations, |element, _, opacity| element.opacity(opacity)).into_any_element()
}
