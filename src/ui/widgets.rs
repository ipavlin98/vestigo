mod color;
mod field;
mod path;
mod slider;

use color::chip;
pub(crate) use color::{ColorButton, ColorPick, Hsv, PICKER_INNER, Picked, color_button, preset, sample, swatch};
pub(crate) use field::{shortcut_field, text_field};
pub(crate) use path::{path_crumbs, path_label};
pub(crate) use slider::{Slider, slider};

use super::flow::Mark;
use super::scroll::{glide, open_at};
use super::tip::{Tip, TipAnchor, ceil_device, with_tip};
use super::tokens::*;
use crate::assets;
use crate::theme::{BASE_REM, Corners, FADE, GRID, Theme, device_pixels, fade, pixels, rem, snap};
use gpui::{
    Anchor, AnchoredPositionMode, App, BorderStyle, Bounds, ClickEvent, ContentMask, Div, ElementId, Hsla, IntoElement,
    MouseDownEvent, Pixels, ScrollHandle, SharedString, Stateful, TextRun, Window, anchored, canvas, deferred, div,
    fill, font, point, prelude::*, px, quad, size, svg,
};
use std::cell::Cell;
use std::rc::Rc;

const ICON_BUTTON_GROUP: &str = "icon-button";
pub(crate) const DISCLOSURE_GROUP: &str = "disclosure";
const DROPDOWN_GROUP: &str = "dropdown";
const OPTION_GROUP: &str = "option";
const MENU_ID: &str = "menu";
const OPTIONS_ID: &str = "options";
const OPENED_ID: &str = "opened";
const MENU_HEIGHT: f32 = 448.0;
const OVERLAY_ID: &str = "overlay";
const BASELINE_SAMPLE: &str = "Hg";
const CLIP_EPSILON: f32 = 0.5;
const HAIRLINE_DEVICE: f32 = 1.0;
const MARKER_DEVICE: f32 = 2.0;
const DISABLED_OPACITY: f32 = 0.4;

/// What lies under a fading text: one color, or a moving selection fill over a base color, where
/// the fade takes the fill's color exactly over the part the fill covers in this frame.
#[derive(Clone)]
pub(crate) enum Under {
    Color(Hsla),
    Marked { base: Hsla, mark: Hsla, shown: Rc<Cell<Option<Bounds<Pixels>>>> },
}

impl From<Hsla> for Under {
    fn from(color: Hsla) -> Self {
        Self::Color(color)
    }
}

impl Under {
    pub(crate) fn marked(base: Hsla, mark: Hsla, by: &Mark) -> Self {
        Self::Marked { base, mark, shown: by.shown() }
    }
}

pub(crate) fn faded(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    tip: Tip,
    line: f32,
    under: impl Into<Under>,
) -> Stateful<Div> {
    let clipped = Rc::new(Cell::new(None));
    fading(id, content, tip, line, fade_edge(under.into(), clipped.clone()), clipped)
}

pub(crate) fn faded_unless_fits(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    tip: Tip,
    line: f32,
    under: impl Into<Under>,
    fits: bool,
) -> Stateful<Div> {
    if fits {
        fading(id, content, tip, line, div().absolute().right_0(), Rc::new(Cell::new(None)))
    } else {
        faded(id, content, tip, line, under)
    }
}

fn fading(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    tip: Tip,
    line: f32,
    edge: impl IntoElement,
    clipped: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Stateful<Div> {
    let measured = clipped.clone();
    let element = div()
        .relative()
        .h(rem(line))
        .line_height(rem(line))
        .overflow_hidden()
        .child(div().absolute().left_0().whitespace_nowrap().child(content))
        .child(edge)
        .on_children_prepainted(move |children, _, _| {
            if let [text, edge] = children.as_slice() {
                let clipped = f32::from(text.right() - edge.right()) > CLIP_EPSILON;
                let own = Bounds::from_corners(point(text.left(), edge.top()), edge.bottom_right());
                measured.set(clipped.then_some(own));
            }
        })
        .id(id);
    with_tip(element, tip, clipped)
}

fn fade_edge(under: Under, clipped: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if clipped.get().is_some() {
                paint_fade(bounds, &under, window);
            }
        },
    )
    .absolute()
    .top_0()
    .bottom_0()
    .right_0()
    .w(rem(FADE))
}

fn paint_fade(bounds: Bounds<Pixels>, under: &Under, window: &mut Window) {
    let (base, over) = match under {
        Under::Color(color) => (*color, None),
        Under::Marked { base, mark, shown } => {
            let over = shown.get().map(|shown| shown.intersect(&bounds)).filter(|over| !over.is_empty());
            (*base, over.map(|over| (*mark, over)))
        }
    };
    let Some((mark, over)) = over else {
        window.paint_quad(fill(bounds, fade(base, 90.)));
        return;
    };
    window.with_content_mask(Some(ContentMask { bounds: over }), |window| {
        window.paint_quad(fill(bounds, fade(mark, 90.)));
    });
    for part in around(bounds, over).into_iter().filter(|part| !part.is_empty()) {
        window.with_content_mask(Some(ContentMask { bounds: part }), |window| {
            window.paint_quad(fill(bounds, fade(base, 90.)));
        });
    }
}

fn around(outer: Bounds<Pixels>, inner: Bounds<Pixels>) -> [Bounds<Pixels>; 4] {
    [
        Bounds::from_corners(outer.origin, point(outer.right(), inner.top())),
        Bounds::from_corners(point(outer.left(), inner.bottom()), outer.bottom_right()),
        Bounds::from_corners(point(outer.left(), inner.top()), point(inner.left(), inner.bottom())),
        Bounds::from_corners(point(inner.right(), inner.top()), point(outer.right(), inner.bottom())),
    ]
}

fn fade_edge_div(under: Hsla) -> Div {
    div().absolute().top_0().bottom_0().right_0().w(rem(FADE)).bg(fade(under, 90.))
}

pub(crate) fn baseline_shift(from: f32, to: f32, theme: &Theme, window: &Window) -> Pixels {
    snap(baseline_below_center(from, theme, window) - baseline_below_center(to, theme, window), window.scale_factor())
}

pub(crate) fn cap_shift(size: f32, theme: &Theme, window: &Window) -> Pixels {
    let text = window.text_system();
    let cap = text.cap_height(text.resolve_font(&font(theme.font.clone())), pixels(size, window));
    snap(cap / 2. - baseline_below_center(size, theme, window), window.scale_factor())
}

fn baseline_below_center(size: f32, theme: &Theme, window: &Window) -> Pixels {
    let run = TextRun {
        len: BASELINE_SAMPLE.len(),
        font: font(theme.font.clone()),
        color: theme.results.name,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(BASELINE_SAMPLE.into(), pixels(size, window), &[run], None);
    (line.ascent - line.descent) / 2.
}

pub(crate) fn glyph(path: &'static str, side: f32, color: Hsla) -> impl IntoElement {
    svg().path(path).size(rem(side)).flex_none().text_color(color)
}

pub(crate) fn window_shape<E: ParentElement>(element: E, corners: Corners) -> E {
    let cut = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let radius = |length: f32| pixels(length, window);
            let radii = gpui::Corners {
                top_left: radius(corners.top_left),
                top_right: radius(corners.top_right),
                bottom_right: radius(corners.bottom_right),
                bottom_left: radius(corners.bottom_left),
            };
            window.cut_corners(bounds, radii);
        },
    );
    element.child(cut.absolute().inset_0())
}

fn snapped(bounds: Bounds<Pixels>, window: &Window) -> Bounds<Pixels> {
    let scale = window.scale_factor();
    Bounds::from_corners(
        point(snap(bounds.left(), scale), snap(bounds.top(), scale)),
        point(snap(bounds.right(), scale), snap(bounds.bottom(), scale)),
    )
}

pub(super) fn hairline(window: &Window) -> Pixels {
    device_pixels(HAIRLINE_DEVICE, window.scale_factor())
}

#[derive(Clone, Copy)]
pub(crate) enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

pub(crate) fn divider(edge: Edge, color: Hsla) -> impl IntoElement {
    line(edge, color, |_| HAIRLINE_DEVICE)
}

pub(crate) fn drop_marker(edge: Edge, color: Hsla) -> impl IntoElement {
    line(edge, color, |scale| (MARKER_DEVICE * scale).round())
}

fn line(edge: Edge, color: Hsla, devices: fn(f32) -> f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let bounds = snapped(bounds, window);
            let scale = window.scale_factor();
            let thickness = device_pixels(devices(scale), scale);
            let line = match edge {
                Edge::Top => Bounds::new(bounds.origin, size(bounds.size.width, thickness)),
                Edge::Bottom => {
                    Bounds::new(point(bounds.left(), bounds.bottom() - thickness), size(bounds.size.width, thickness))
                }
                Edge::Left => Bounds::new(bounds.origin, size(thickness, bounds.size.height)),
                Edge::Right => {
                    Bounds::new(point(bounds.right() - thickness, bounds.top()), size(thickness, bounds.size.height))
                }
            };
            window.paint_quad(fill(line, color));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(super) fn frame(radius: f32, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let bounds = snapped(bounds, window);
            let inside = color.opacity(0.);
            let border = quad(bounds, pixels(radius, window), inside, hairline(window), color, BorderStyle::Solid);
            window.paint_quad(border);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn edge_fades(scroll: ScrollHandle, under: Hsla, inset: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let above = -scroll.offset().y;
            let below = scroll.max_offset().y - above;
            let height = pixels(EDGE_FADE, window).min(bounds.size.height / 2.);
            let side = pixels(inset, window);
            let edge = size(bounds.size.width - side * 2., height);
            let (top, bottom) = (fade_strength(above, height), fade_strength(below, height));
            if top > 0. {
                let origin = point(bounds.left() + side, bounds.top());
                window.paint_quad(fill(Bounds::new(origin, edge), fade(under.opacity(top), 0.)));
            }
            if bottom > 0. {
                let origin = point(bounds.left() + side, bounds.bottom() - height);
                window.paint_quad(fill(Bounds::new(origin, edge), fade(under.opacity(bottom), 180.)));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn fade_strength(hidden: Pixels, height: Pixels) -> f32 {
    if height <= px(0.) { 0. } else { (hidden / height).clamp(0., 1.) }
}

#[derive(Clone, Copy)]
pub(crate) enum Key {
    Text(&'static str),
    Enter,
}

pub(crate) const CTRL: Key = Key::Text("Ctrl");

pub(crate) fn hint_chip(keys: &[Key], label: &'static str, theme: &Theme, window: &Window) -> impl IntoElement {
    hint_keys(keys, theme, window).child(label)
}

pub(crate) fn hint_slot(
    keys: &[Key],
    label: &'static str,
    among: &[&'static str],
    theme: &Theme,
    window: &Window,
) -> impl IntoElement + use<> {
    let measure = |text: &'static str| label_width(text, theme, window);
    let width = steady_width(measure(label), among.iter().map(|text| measure(text)), pixels(LABEL_SLACK, window));
    let width = ceil_device(width, window.scale_factor());
    hint_keys(keys, theme, window).child(div().w(width).flex_none().whitespace_nowrap().child(label))
}

pub(crate) fn hint_keys(keys: &[Key], theme: &Theme, window: &Window) -> Div {
    let colors = theme.keys;
    let shift = cap_shift(DETAIL_SIZE, theme, window);
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rem(LABEL_GAP))
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .text_color(colors.label)
        .child(div().flex().gap(rem(KEY_GAP)).children(keys.iter().map(|key| key_chip(*key, shift, theme))))
}

fn label_width(text: &'static str, theme: &Theme, window: &Window) -> Pixels {
    detail_width(text.into(), theme.keys.label, theme, window)
}

fn detail_width(text: SharedString, color: Hsla, theme: &Theme, window: &Window) -> Pixels {
    text_width(text, DETAIL_SIZE, color, theme, window)
}

fn text_width(text: SharedString, size: f32, color: Hsla, theme: &Theme, window: &Window) -> Pixels {
    let run = TextRun {
        len: text.len(),
        font: font(theme.font.clone()),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window.text_system().shape_line(text, pixels(size, window), &[run], None).width
}

pub(crate) fn slot_width(text: SharedString, size: f32, theme: &Theme, window: &Window) -> Pixels {
    ceil_device(text_width(text, size, theme.results.shortcut, theme, window), window.scale_factor())
}

fn steady_width(width: Pixels, among: impl Iterator<Item = Pixels>, slack: Pixels) -> Pixels {
    let mut wider: Vec<f32> = among.map(f32::from).filter(|other| *other > f32::from(width)).collect();
    wider.sort_by(f32::total_cmp);
    let mut steady = f32::from(width);
    for other in wider {
        if other - steady > f32::from(slack) {
            break;
        }
        steady = other;
    }
    px(steady)
}

pub(crate) fn key_hint(key: Key, theme: &Theme, window: &Window) -> impl IntoElement {
    let shift = cap_shift(DETAIL_SIZE, theme, window);
    key_chip(key, shift, theme).flex_none().text_size(rem(DETAIL_SIZE)).line_height(rem(DETAIL_LINE))
}

fn key_chip(key: Key, shift: Pixels, theme: &Theme) -> Div {
    let colors = theme.keys;
    div()
        .relative()
        .h(rem(KEY))
        .min_w(rem(match key {
            Key::Text(text) if text.chars().count() > 1 => KEY_WIDE,
            Key::Text(_) | Key::Enter => KEY,
        }))
        .px(rem(KEY_PADDING))
        .flex()
        .items_center()
        .justify_center()
        .text_color(colors.glyph)
        .child(match key {
            Key::Text(text) => div().relative().top(shift).child(text).into_any_element(),
            Key::Enter => glyph(assets::ENTER, KEY_GLYPH, colors.glyph).into_any_element(),
        })
        .child(frame(theme.radius.small, colors.frame))
}

pub(crate) fn icon_button(id: impl Into<ElementId>, path: &'static str, theme: &Theme) -> Stateful<Div> {
    icon_toggle(id, path, false, theme)
}

pub(crate) fn icon_toggle(id: impl Into<ElementId>, path: &'static str, on: bool, theme: &Theme) -> Stateful<Div> {
    let colors = theme.icon_button;
    let glyph = if on { colors.hover_glyph } else { colors.glyph };
    let margin = (ICON_BUTTON - GLYPH) / 2.0;
    div()
        .id(id)
        .group(ICON_BUTTON_GROUP)
        .flex_none()
        .size(rem(ICON_BUTTON))
        .mx(rem(-margin))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .block_mouse_except_scroll()
        .child(
            svg()
                .path(path)
                .size(rem(GLYPH))
                .flex_none()
                .text_color(glyph)
                .group_hover(ICON_BUTTON_GROUP, |style| style.text_color(colors.hover_glyph)),
        )
}

pub(crate) fn idle_icon_button(id: impl Into<ElementId>, path: &'static str, theme: &Theme) -> Stateful<Div> {
    let margin = (ICON_BUTTON - GLYPH) / 2.0;
    div()
        .id(id)
        .flex_none()
        .size(rem(ICON_BUTTON))
        .mx(rem(-margin))
        .flex()
        .items_center()
        .justify_center()
        .block_mouse_except_scroll()
        .opacity(DISABLED_OPACITY)
        .child(glyph(path, GLYPH, theme.icon_button.glyph))
}

pub(crate) fn disclosure(open: bool, theme: &Theme) -> Div {
    let colors = theme.icon_button;
    let margin = (ICON_BUTTON - GLYPH) / 2.0;
    div().flex_none().size(rem(ICON_BUTTON)).mx(rem(-margin)).flex().items_center().justify_center().child(
        svg()
            .path(if open { assets::CHEVRON_UP } else { assets::CHEVRON_DOWN })
            .size(rem(GLYPH))
            .flex_none()
            .text_color(colors.glyph)
            .group_hover(DISCLOSURE_GROUP, |style| style.text_color(colors.hover_glyph)),
    )
}

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let colors = theme.button;
    let radius = theme.radius.large;
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(rem(BUTTON_WIDTH))
        .h(rem(CONTROL))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rem(radius))
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .text_color(if enabled { colors.text } else { colors.disabled })
        .block_mouse_except_scroll()
        .when(enabled, |button| button.cursor_pointer().hover(|style| style.bg(colors.hover)).on_click(on_click))
        .child(label)
        .child(frame(radius, colors.frame))
}

pub(crate) fn toggle(id: impl Into<ElementId>, on: bool, theme: &Theme) -> Stateful<Div> {
    let colors = theme.toggle;
    switch(id, on, if on { colors.on } else { colors.off }, colors.knob, theme).cursor_pointer()
}

pub(crate) fn disabled_toggle(id: impl Into<ElementId>, theme: &Theme) -> Stateful<Div> {
    switch(id, false, theme.toggle.off, theme.button.disabled, theme)
}

fn switch(id: impl Into<ElementId>, on: bool, track: Hsla, knob: Hsla, theme: &Theme) -> Stateful<Div> {
    let inset = (TRACK_HEIGHT - KNOB) / 2.0;
    let radius = theme.radius.toggle;
    div()
        .id(id)
        .flex_none()
        .w(rem(TRACK_WIDTH))
        .h(rem(TRACK_HEIGHT))
        .p(rem(inset))
        .flex()
        .when(on, |track| track.justify_end())
        .rounded(rem(radius))
        .block_mouse_except_scroll()
        .bg(track)
        .child(div().size(rem(KNOB)).rounded(rem((radius - inset).max(0.0))).bg(knob))
}

pub(crate) struct Choice<T> {
    pub(crate) value: T,
    pub(crate) label: SharedString,
    pub(crate) note: Option<SharedString>,
    pub(crate) swatch: Option<Hsla>,
}

pub(crate) struct Picker<T> {
    pub(crate) id: &'static str,
    pub(crate) choices: Vec<Choice<T>>,
    pub(crate) current: T,
    pub(crate) open: bool,
    pub(crate) under: Hsla,
    pub(crate) scroll: ScrollHandle,
}

pub(crate) fn dropdown<T: Clone + PartialEq + 'static>(
    picker: Picker<T>,
    toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    pick: impl Fn(T, &mut Window, &mut App) + 'static,
    theme: &Theme,
    window: &Window,
) -> Div {
    let Picker { id, choices, current, open, under, scroll } = picker;
    let fitted = fitted_width(&choices, theme, window);
    let width = rem(fitted.unwrap_or(DROPDOWN_MAX));
    let shown = choices.iter().find(|choice| choice.value == current);
    let label = shown.map(|choice| choice.label.clone()).unwrap_or_default();
    let swatch = shown.and_then(|choice| choice.swatch);
    let trigger = trigger(id, label, swatch, open, under, fitted.is_some(), theme).w(width).on_click(toggle);
    let root = div().relative().flex_none().text_size(rem(DETAIL_SIZE)).line_height(rem(DETAIL_LINE)).child(trigger);
    if !open {
        return root;
    }
    let viewport = window.viewport_size();
    let overlay = div()
        .id(ElementId::Name(format!("{id}-{OVERLAY_ID}").into()))
        .w(viewport.width)
        .h(viewport.height)
        .occlude()
        .on_any_mouse_down(close);
    let overlay = anchored().position_mode(AnchoredPositionMode::Window).position(point(px(0.), px(0.))).child(overlay);
    let menu = menu(id, choices, &current, Rc::new(pick), scroll, theme, window);
    let menu = anchored()
        .anchor(Anchor::TopRight)
        .snap_to_window_with_margin(pixels(FILL_GAP, window))
        .child(div().w(width).child(menu));
    let over_trigger = div().size_full().cursor_pointer();
    root.child(deferred(overlay).with_priority(1))
        .child(div().absolute().inset_0().child(deferred(over_trigger).with_priority(2)))
        .child(div().absolute().top(rem(CONTROL + FILL_GAP)).right_0().child(deferred(menu).with_priority(3)))
}

fn fitted_width<T>(choices: &[Choice<T>], theme: &Theme, window: &Window) -> Option<f32> {
    let measure = |text: &SharedString| {
        f32::from(detail_width(text.clone(), theme.dropdown.text, theme, window)) * BASE_REM
            / f32::from(window.rem_size())
    };
    let widest = choices
        .iter()
        .map(|choice| measure(&choice.label) + choice.note.as_ref().map_or(0., |note| measure(note) + LABEL_GAP))
        .fold(0., f32::max);
    let chip = if choices.iter().any(|choice| choice.swatch.is_some()) { KEY_GLYPH + LABEL_GAP } else { 0. };
    let width = 2. * (FILL_GAP + INDENT) + chip + widest + LABEL_GAP + KEY_GLYPH;
    Some(((width / GRID).ceil() * GRID).max(CONTROL_WIDTH)).filter(|width| *width <= DROPDOWN_MAX)
}

fn label_edge(fits: bool, under: Hsla) -> Div {
    if fits { div().absolute().right_0() } else { fade_edge_div(under) }
}

fn trigger(
    id: &'static str,
    label: SharedString,
    swatch: Option<Hsla>,
    open: bool,
    under: Hsla,
    fits: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let colors = theme.dropdown;
    let radius = theme.radius.large;
    let edge = label_edge(fits, if open { colors.hover } else { under })
        .when(!fits, |edge| edge.group_hover(DROPDOWN_GROUP, move |style| style.bg(fade(colors.hover, 90.))));
    let anchor = TipAnchor::default();
    div()
        .id(id)
        .group(DROPDOWN_GROUP)
        .relative()
        .h(rem(CONTROL))
        .px(rem(FILL_GAP + INDENT))
        .flex()
        .items_center()
        .gap(rem(LABEL_GAP))
        .rounded(rem(radius))
        .cursor_pointer()
        .block_mouse_except_scroll()
        .text_color(colors.text)
        .when(open, |trigger| trigger.bg(colors.hover))
        .hover(|style| style.bg(colors.hover))
        .child(anchor.track())
        .children(swatch.map(|color| chip(color, theme)))
        .child(
            fading("label", label.clone(), Tip::new(label).along(anchor), DETAIL_LINE, edge, Rc::new(Cell::new(None)))
                .flex_1()
                .min_w_0(),
        )
        .child(glyph(assets::CHEVRON_DOWN, KEY_GLYPH, colors.glyph))
        .child(frame(radius, colors.frame))
}

type Pick<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

fn menu<T: Clone + PartialEq + 'static>(
    id: &'static str,
    choices: Vec<Choice<T>>,
    current: &T,
    pick: Pick<T>,
    scroll: ScrollHandle,
    theme: &Theme,
    window: &Window,
) -> Stateful<Div> {
    let colors = theme.dropdown;
    let chosen = choices.iter().position(|choice| &choice.value == current).unwrap_or_default();
    let chips = choices.iter().any(|choice| choice.swatch.is_some());
    let count = choices.len();
    let fits = fitted_width(&choices, theme, window).is_some();
    let options = choices.into_iter().enumerate().map(|(position, Choice { value, label, note, swatch })| {
        let chosen = &value == current;
        let edge = label_edge(fits, if chosen { colors.chosen } else { colors.menu }).when(!chosen && !fits, |edge| {
            edge.group_hover(OPTION_GROUP, move |style| style.bg(fade(colors.hover, 90.)))
        });
        let pick = pick.clone();
        let anchor = TipAnchor::default();
        div()
            .id(ElementId::NamedInteger(id.into(), position as u64))
            .group(OPTION_GROUP)
            .relative()
            .flex_none()
            .h(rem(CONTROL))
            .px(rem(INDENT))
            .flex()
            .items_center()
            .gap(rem(LABEL_GAP))
            .rounded(rem(theme.radius.small))
            .cursor_pointer()
            .block_mouse_except_scroll()
            .text_color(if chosen { colors.chosen_text } else { colors.option })
            .when(chosen, |option| option.bg(colors.chosen))
            .when(!chosen, |option| option.hover(|style| style.bg(colors.hover).text_color(colors.hover_text)))
            .child(anchor.track())
            .children(
                swatch
                    .map(|color| chip(color, theme).into_any_element())
                    .or_else(|| chips.then(|| div().flex_none().size(rem(KEY_GLYPH)).into_any_element())),
            )
            .child(
                fading(
                    "label",
                    label.clone(),
                    Tip::new(label).along(anchor),
                    DETAIL_LINE,
                    edge,
                    Rc::new(Cell::new(None)),
                )
                .flex_1()
                .min_w_0(),
            )
            .children(note.map(|note| div().flex_none().text_color(colors.note).child(note)))
            .child(
                div()
                    .flex_none()
                    .size(rem(KEY_GLYPH))
                    .flex()
                    .children(chosen.then(|| glyph(assets::CHECK, KEY_GLYPH, colors.check))),
            )
            .on_click(move |_, window, cx| pick(value.clone(), window, cx))
    });
    let radius = theme.radius.large;
    let list = div()
        .id(OPTIONS_ID)
        .max_h(rem(MENU_HEIGHT))
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .p(rem(FILL_GAP))
        .flex()
        .flex_col()
        .gap(inner_gap(window))
        .children(options);
    div()
        .id(ElementId::Name(format!("{id}-{MENU_ID}").into()))
        .relative()
        .w_full()
        .rounded(rem(radius))
        .bg(colors.menu)
        .occlude()
        .child(open_at(OPENED_ID, scroll.clone(), menu_rows(chosen, count, window)))
        .child(glide(MENU_ID, scroll.clone(), None))
        .child(list)
        .child(edge_fades(scroll, colors.menu, FILL_GAP))
        .child(frame(radius, colors.frame))
}

fn menu_rows(chosen: usize, count: usize, window: &Window) -> (Pixels, Pixels, Pixels) {
    let (padding, row, gap) = (pixels(FILL_GAP, window), pixels(CONTROL, window), inner_gap(window));
    let content = padding * 2. + row * count as f32 + gap * count.saturating_sub(1) as f32;
    (padding + (row + gap) * chosen as f32, row, content)
}

fn inner_gap(window: &Window) -> Pixels {
    pixels(FILL_GAP, window) - device_pixels(HAIRLINE_DEVICE, window.scale_factor())
}

#[cfg(test)]
mod tests {
    use super::{fade_strength, steady_width};
    use gpui::px;

    const HEIGHT: f32 = 16.0;
    const QUARTER: f32 = 4.0;
    const PAST: f32 = 40.0;

    #[test]
    fn an_edge_fade_grows_with_the_hidden_content_and_is_gone_at_the_edge() {
        let height = px(HEIGHT);
        assert_eq!(fade_strength(px(0.), height), 0.);
        assert_eq!(fade_strength(-px(QUARTER), height), 0.);
        assert_eq!(fade_strength(px(QUARTER), height), 0.25);
        assert_eq!(fade_strength(px(PAST), height), 1.);
    }

    #[test]
    fn labels_a_few_pixels_apart_share_one_width_and_a_far_one_keeps_its_own() {
        let measured = [50.0_f32, 53.0, 60.0, 90.0];
        let [choose, go_to, open, long] = measured;
        let slack = px(QUARTER * 2.);
        let steady = |width: f32| f32::from(steady_width(px(width), measured.into_iter().map(px), slack));
        assert_eq!(steady(choose), open);
        assert_eq!(steady(go_to), open);
        assert_eq!(steady(open), open);
        assert_eq!(steady(long), long);
    }
}
