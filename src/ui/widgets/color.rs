use super::super::tokens::*;
use super::slider::scrubbed;
use super::{OVERLAY_ID, frame, glyph, hairline, snapped};
use crate::assets;
use crate::theme::{Swatch, Theme, contrast, device_pixels, pixels, rem, snap};
use gpui::{
    Anchor, AnchoredPositionMode, App, BorderStyle, Bounds, Div, ElementId, Hsla, MouseDownEvent, Pixels, Point, Rgba,
    Stateful, Window, anchored, canvas, deferred, div, fill, linear_color_stop, linear_gradient, point, prelude::*, px,
    quad, size,
};
use std::rc::Rc;

const SWATCH_GROUP: &str = "swatch";
const PICKER_ID: &str = "picker";
const FIELD_ID: &str = "picker-field";
const HUE_ID: &str = "picker-hue";
const ALPHA_ID: &str = "picker-alpha";
const PICKER_WIDTH: f32 = 224.0;
pub(crate) const PICKER_INNER: f32 = PICKER_WIDTH - 2.0 * INSET;
const FIELD_HEIGHT: f32 = 128.0;
const HUE_HEIGHT: f32 = 16.0;
const FIELD_KNOB: f32 = 12.0;
const KNOB_RING_DEVICE: f32 = 2.0;
const SECTORS: usize = 6;
const SECTORS_EPSILON: f32 = 1e-6;
const RIGHT: f32 = 90.0;
const PRESET: f32 = 24.0;
const DOWN: f32 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hsv {
    pub(crate) hue: f32,
    pub(crate) saturation: f32,
    pub(crate) value: f32,
}

impl Hsv {
    pub(crate) fn of(color: Hsla) -> Self {
        let Rgba { r, g, b, .. } = color.to_rgb();
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        let sectors = SECTORS as f32;
        let hue = if delta <= SECTORS_EPSILON {
            0.0
        } else if max == r {
            ((g - b) / delta).rem_euclid(sectors) / sectors
        } else if max == g {
            ((b - r) / delta + 2.0) / sectors
        } else {
            ((r - g) / delta + 4.0) / sectors
        };
        let saturation = if max <= SECTORS_EPSILON { 0.0 } else { delta / max };
        Self { hue, saturation, value: max }
    }

    pub(crate) fn color(self) -> Hsla {
        let sector = self.hue.rem_euclid(1.0) * SECTORS as f32;
        let chroma = self.value * self.saturation;
        let rising = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match sector as usize {
            0 => (chroma, rising, 0.0),
            1 => (rising, chroma, 0.0),
            2 => (0.0, chroma, rising),
            3 => (0.0, rising, chroma),
            4 => (rising, 0.0, chroma),
            _ => (chroma, 0.0, rising),
        };
        let lift = self.value - chroma;
        Hsla::from(Rgba { r: r + lift, g: g + lift, b: b + lift, a: 1.0 })
    }

    fn pure(hue: f32) -> Hsla {
        Self { hue, saturation: 1.0, value: 1.0 }.color()
    }
}

pub(super) fn chip(color: Hsla, theme: &Theme) -> Div {
    div().relative().flex_none().size(rem(KEY_GLYPH)).child(sample(theme.radius.small, color, theme.picker.frame))
}

pub(crate) fn sample(radius: f32, color: Hsla, edge: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let bounds = snapped(bounds, window);
            window.paint_quad(quad(bounds, pixels(radius, window), color, hairline(window), edge, BorderStyle::Solid));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn swatch(id: impl Into<ElementId>, color: Hsla, open: bool, theme: &Theme) -> Stateful<Div> {
    let colors = theme.picker;
    let radius = theme.radius.large;
    div()
        .id(id)
        .group(SWATCH_GROUP)
        .relative()
        .flex_none()
        .size(rem(CONTROL))
        .cursor_pointer()
        .block_mouse_except_scroll()
        .children(layers(radius, color, if open { colors.hover } else { colors.frame }, colors.hover))
}

pub(crate) fn preset(id: impl Into<ElementId>, fill: Option<Hsla>, chosen: bool, theme: &Theme) -> Stateful<Div> {
    let colors = theme.picker;
    let sample = div()
        .id(id)
        .group(SWATCH_GROUP)
        .relative()
        .flex_none()
        .size(rem(PRESET))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .block_mouse_except_scroll()
        .children(layers(PRESET / 2.0, fill.unwrap_or(colors.frame.opacity(0.)), colors.frame, colors.hover));
    match fill {
        Some(color) => {
            let mark =
                if contrast(color, colors.knob) >= contrast(color, colors.ring) { colors.knob } else { colors.ring };
            sample.children(chosen.then(|| glyph(assets::CHECK, KEY_GLYPH, mark)))
        }
        None => sample.child(glyph(assets::PIPETTE, KEY_GLYPH, theme.icon_button.glyph)),
    }
}

fn layers(radius: f32, fill: Hsla, rest: Hsla, hover: Hsla) -> [Div; 2] {
    let layer = || div().absolute().top_0().left_0().size_full();
    [
        layer().group_hover(SWATCH_GROUP, |style| style.opacity(0.)).child(sample(radius, fill, rest)),
        layer().opacity(0.).group_hover(SWATCH_GROUP, |style| style.opacity(1.)).child(sample(radius, fill, hover)),
    ]
}

pub(crate) struct ColorButton {
    pub(crate) swatch: Stateful<Div>,
    pub(crate) picking: Option<Picked>,
}

pub(crate) struct Picked {
    pub(crate) hsv: Hsv,
    pub(crate) alpha: Option<f32>,
    pub(crate) field: Div,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ColorPick {
    Shade(Hsv),
    Alpha(f32),
}

pub(crate) fn color_button(
    button: ColorButton,
    change: impl Fn(ColorPick, &mut Window, &mut App) + 'static,
    close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    theme: &Theme,
    window: &Window,
) -> Div {
    let ColorButton { swatch, picking } = button;
    let root = div().relative().flex_none().child(swatch);
    let Some(picked) = picking else { return root };
    let viewport = window.viewport_size();
    let overlay = div()
        .id(ElementId::Name(format!("{PICKER_ID}-{OVERLAY_ID}").into()))
        .w(viewport.width)
        .h(viewport.height)
        .occlude()
        .on_any_mouse_down(close);
    let overlay = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(overlay);
    let picker = anchored()
        .anchor(Anchor::TopRight)
        .snap_to_window_with_margin(pixels(FILL_GAP, window))
        .child(picker(picked, Rc::new(change), theme));
    root.child(deferred(overlay).with_priority(1))
        .child(div().absolute().top_0().right(rem(CONTROL + FILL_GAP)).child(deferred(picker).with_priority(3)))
}

type Change = Rc<dyn Fn(ColorPick, &mut Window, &mut App)>;

fn picker(picked: Picked, change: Change, theme: &Theme) -> Stateful<Div> {
    let Picked { hsv, alpha, field } = picked;
    let colors = theme.dropdown;
    let radius = theme.radius.large;
    div()
        .id(PICKER_ID)
        .relative()
        .w(rem(PICKER_WIDTH))
        .p(rem(INSET))
        .flex()
        .flex_col()
        .gap(rem(INSET))
        .rounded(rem(radius))
        .bg(colors.menu)
        .occlude()
        .child(shade(hsv, change.clone(), theme))
        .child(hue(hsv, change.clone(), theme))
        .children(alpha.map(|alpha| opacity(hsv, alpha, change, theme)))
        .child(field)
        .child(frame(radius, colors.frame))
}

fn shade(hsv: Hsv, change: Change, theme: &Theme) -> Stateful<Div> {
    let knobs = theme.picker;
    let radius = theme.radius.small;
    let area = div().id(FIELD_ID).relative().w_full().h(rem(FIELD_HEIGHT)).cursor_pointer().block_mouse_except_scroll();
    scrubbed(area, FIELD_ID.into(), FIELD_KNOB / 2.0, move |at, window, cx| {
        change(ColorPick::Shade(Hsv { saturation: at.x, value: 1.0 - at.y, ..hsv }), window, cx)
    })
    .child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let corners = gpui::Corners::all(pixels(radius, window));
                let inset = pixels(FIELD_KNOB / 2.0, window);
                let white = Hsv { saturation: 0.0, value: 1.0, ..hsv }.color();
                let black = Hsv { value: 0.0, ..hsv }.color();
                let (left, right) = ramp(bounds.size.width, inset);
                let (top, bottom) = ramp(bounds.size.height, inset);
                let across = linear_gradient(
                    RIGHT,
                    linear_color_stop(white, left),
                    linear_color_stop(Hsv::pure(hsv.hue), right),
                );
                let down =
                    linear_gradient(DOWN, linear_color_stop(black.opacity(0.), top), linear_color_stop(black, bottom));
                window.paint_quad(fill(bounds, across).corner_radii(corners));
                window.paint_quad(fill(bounds, down).corner_radii(corners));
                let center = point(
                    along(bounds.left(), bounds.size.width, inset, hsv.saturation),
                    along(bounds.top(), bounds.size.height, inset, 1.0 - hsv.value),
                );
                knob(center, FIELD_KNOB, hsv.color(), knobs, window);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

fn hue(hsv: Hsv, change: Change, theme: &Theme) -> Stateful<Div> {
    let knobs = theme.picker;
    let bar = div().id(HUE_ID).relative().w_full().h(rem(HUE_HEIGHT)).cursor_pointer().block_mouse_except_scroll();
    scrubbed(bar, HUE_ID.into(), HUE_HEIGHT / 2.0, move |at, window, cx| {
        change(ColorPick::Shade(Hsv { hue: at.x.min(1.0), ..hsv }), window, cx)
    })
    .child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let scale = window.scale_factor();
                let inset = pixels(HUE_HEIGHT / 2.0, window);
                let round = bounds.size.height / 2.;
                let at = |sector: usize| along(bounds.left(), bounds.size.width, inset, sector as f32 / SECTORS as f32);
                for sector in 0..SECTORS {
                    let (first, last) = (sector == 0, sector + 1 == SECTORS);
                    let start = if first { bounds.left() } else { snap(at(sector), scale) };
                    let end = if last { bounds.right() } else { snap(at(sector + 1), scale) };
                    let part = Bounds::from_corners(point(start, bounds.top()), point(end, bounds.bottom()));
                    let corners = gpui::Corners {
                        top_left: if first { round } else { px(0.) },
                        bottom_left: if first { round } else { px(0.) },
                        top_right: if last { round } else { px(0.) },
                        bottom_right: if last { round } else { px(0.) },
                    };
                    let span = end - start;
                    let gradient = linear_gradient(
                        RIGHT,
                        linear_color_stop(Hsv::pure(sector as f32 / SECTORS as f32), (at(sector) - start) / span),
                        linear_color_stop(
                            Hsv::pure((sector + 1) as f32 / SECTORS as f32),
                            (at(sector + 1) - start) / span,
                        ),
                    );
                    window.paint_quad(fill(part, gradient).corner_radii(corners));
                }
                let center = point(along(bounds.left(), bounds.size.width, inset, hsv.hue), bounds.center().y);
                knob(center, HUE_HEIGHT, Hsv::pure(hsv.hue), knobs, window);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

fn opacity(hsv: Hsv, alpha: f32, change: Change, theme: &Theme) -> Stateful<Div> {
    let knobs = theme.picker;
    let bar = div().id(ALPHA_ID).relative().w_full().h(rem(HUE_HEIGHT)).cursor_pointer().block_mouse_except_scroll();
    scrubbed(bar, ALPHA_ID.into(), HUE_HEIGHT / 2.0, move |at, window, cx| {
        change(ColorPick::Alpha(at.x.clamp(0.0, 1.0)), window, cx)
    })
    .child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let color = hsv.color();
                let bounds = snapped(bounds, window);
                let inset = pixels(HUE_HEIGHT / 2.0, window);
                let (clear, solid) = ramp(bounds.size.width, inset);
                let gradient = linear_gradient(
                    RIGHT,
                    linear_color_stop(color.opacity(0.), clear),
                    linear_color_stop(color, solid),
                );
                let round = bounds.size.height / 2.;
                window.paint_quad(quad(bounds, round, gradient, hairline(window), knobs.frame, BorderStyle::Solid));
                let center = point(along(bounds.left(), bounds.size.width, inset, alpha), bounds.center().y);
                knob(center, HUE_HEIGHT, color, knobs, window);
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full(),
    )
}

fn along(start: Pixels, length: Pixels, inset: Pixels, fraction: f32) -> Pixels {
    start + inset + (length - inset * 2.) * fraction.clamp(0.0, 1.0)
}

fn ramp(length: Pixels, inset: Pixels) -> (f32, f32) {
    let share = if length > px(0.) { inset / length } else { 0.0 };
    (share, 1.0 - share)
}

fn knob(center: Point<Pixels>, diameter: f32, color: Hsla, colors: Swatch, window: &mut Window) {
    let scale = window.scale_factor();
    let side = pixels(diameter, window);
    let origin = point(snap(center.x - side / 2., scale), snap(center.y - side / 2., scale));
    let inner = Bounds::new(origin, size(side, side));
    let ring = device_pixels(KNOB_RING_DEVICE, scale);
    let line = hairline(window);
    window.paint_quad(fill(inner.dilate(line), colors.ring).corner_radii(side / 2. + line));
    window.paint_quad(quad(inner, side / 2., color, ring, colors.knob, BorderStyle::Solid));
}

#[cfg(test)]
mod tests {
    use super::Hsv;
    use gpui::{Hsla, Rgba};

    fn rgb(r: f32, g: f32, b: f32) -> Hsla {
        Hsla::from(Rgba { r, g, b, a: 1.0 })
    }

    #[test]
    fn colors_survive_the_trip_through_hue_saturation_and_value() {
        for color in
            [rgb(1.0, 0.0, 0.5), rgb(0.2, 0.6, 0.4), rgb(0.1, 0.1, 0.1), rgb(1.0, 1.0, 1.0), rgb(0.3, 0.5, 0.9)]
        {
            let back = Hsv::of(color).color().to_rgb();
            let original = color.to_rgb();
            for (got, want) in [(back.r, original.r), (back.g, original.g), (back.b, original.b)] {
                assert!((got - want).abs() < 1e-4, "{color:?} came back as {back:?}");
            }
        }
        let pink = Hsv::of(rgb(1.0, 0.0, 0.5));
        assert!((pink.hue - 330.0 / 360.0).abs() < 1e-4 && pink.saturation == 1.0 && pink.value == 1.0);
        assert_eq!(Hsv::of(rgb(0.5, 0.5, 0.5)).saturation, 0.0, "grays have no saturation");
    }
}
