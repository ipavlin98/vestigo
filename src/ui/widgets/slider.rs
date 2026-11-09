use super::super::tokens::*;
use super::hairline;
use crate::theme::{Theme, pixels, rem, snap};
use gpui::{
    App, AppContext, BorderStyle, Bounds, DragMoveEvent, ElementId, EmptyView, MouseButton, Pixels, Point,
    SharedString, Stateful, Window, canvas, div, fill, point, prelude::*, px, quad, size,
};
use std::cell::Cell;
use std::rc::Rc;

const TRACK_THICKNESS: f32 = 4.0;
const VALUE_WIDTH: f32 = 40.0;

#[derive(Clone)]
struct Scrub(ElementId);

pub(crate) struct Slider {
    pub(crate) id: ElementId,
    pub(crate) fraction: f32,
    pub(crate) label: SharedString,
}

pub(crate) fn slider(
    slider: Slider,
    change: impl Fn(f32, &mut Window, &mut App) + 'static,
    theme: &Theme,
) -> impl IntoElement {
    let Slider { id, fraction, label } = slider;
    let colors = theme.slider;
    let track = div().id(id.clone()).relative().flex_1().h(rem(CONTROL)).cursor_pointer().block_mouse_except_scroll();
    let track = scrubbed(track, id, KNOB / 2.0, move |at, window, cx| change(at.x, window, cx)).child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let scale = window.scale_factor();
                let knob = pixels(KNOB, window);
                let thickness = pixels(TRACK_THICKNESS, window);
                let travel = bounds.size.width - knob;
                let center = snap(bounds.left() + knob / 2. + travel * fraction.clamp(0.0, 1.0), scale);
                let top = snap(bounds.center().y - thickness / 2., scale);
                let bar =
                    |left: Pixels, right: Pixels| Bounds::from_corners(point(left, top), point(right, top + thickness));
                window.paint_quad(fill(bar(bounds.left(), bounds.right()), colors.track).corner_radii(thickness / 2.));
                window.paint_quad(fill(bar(bounds.left(), center), colors.fill).corner_radii(thickness / 2.));
                let origin = point(center - knob / 2., snap(bounds.center().y - knob / 2., scale));
                let knob_bounds = Bounds::new(origin, size(knob, knob));
                window.paint_quad(quad(
                    knob_bounds,
                    knob / 2.,
                    colors.knob,
                    hairline(window),
                    colors.frame,
                    BorderStyle::Solid,
                ));
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    );
    div()
        .flex_none()
        .w(rem(CONTROL_WIDTH))
        .h(rem(CONTROL))
        .flex()
        .items_center()
        .gap(rem(LABEL_GAP))
        .child(
            div()
                .flex_none()
                .w(rem(VALUE_WIDTH))
                .flex()
                .justify_end()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(theme.settings.value)
                .child(label),
        )
        .child(track)
}

pub(super) fn scrubbed(
    element: Stateful<gpui::Div>,
    id: ElementId,
    inset: f32,
    apply: impl Fn(Point<f32>, &mut Window, &mut App) + 'static,
) -> Stateful<gpui::Div> {
    let apply = Rc::new(apply);
    let measured: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
    let pressed = apply.clone();
    let seen = measured.clone();
    element
        .child(
            canvas(move |bounds, _, _| seen.set(Some(bounds)), |_, _, _, _| {}).absolute().top_0().left_0().size_full(),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            if let Some(bounds) = measured.get() {
                pressed(fraction(event.position, bounds, inset, window), window, cx);
            }
        })
        .on_drag(Scrub(id.clone()), |_, _, _, cx| cx.new(|_| EmptyView))
        .on_drag_move(move |event: &DragMoveEvent<Scrub>, window, cx| {
            if event.drag(cx).0 == id {
                apply(fraction(event.event.position, event.bounds, inset, window), window, cx);
            }
        })
}

fn fraction(position: Point<Pixels>, bounds: Bounds<Pixels>, inset: f32, window: &Window) -> Point<f32> {
    let inset = pixels(inset, window);
    let along = |offset: Pixels, length: Pixels| {
        let travel = length - inset * 2.;
        if travel <= px(0.) { 0.0 } else { ((offset - inset) / travel).clamp(0.0, 1.0) }
    };
    point(along(position.x - bounds.left(), bounds.size.width), along(position.y - bounds.top(), bounds.size.height))
}
