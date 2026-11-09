use super::tokens::*;
use super::widgets::{frame, hairline};
use crate::settings::Shortcut;
use crate::theme::{Theme, pixels, rem, snap};
use gpui::{
    Action, AnchoredPositionMode, App, AppContext, Bounds, Context, Div, Global, IntoElement, Pixels, Point, Render,
    SharedString, Stateful, Subscription, TextRun, Window, anchored, canvas, div, font, point, prelude::*, px,
};
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

const BREAKS: [char; 6] = ['\\', '/', ' ', '-', '_', '.'];
const BREAK_SHARE: f32 = 0.75;

#[derive(Clone)]
pub(crate) struct Tip {
    text: SharedString,
    area: Option<Bounds<Pixels>>,
    anchor: Option<TipAnchor>,
    start: Start,
    fit: Fit,
    side: Side,
}

#[derive(Clone, Copy)]
enum Start {
    Anchor,
    Text,
}

#[derive(Clone, Copy)]
enum Fit {
    Anchor,
    Text,
}

#[derive(Clone, Copy)]
enum Side {
    Below,
    Left,
}

impl Tip {
    pub(crate) fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), area: None, anchor: None, start: Start::Anchor, fit: Fit::Anchor, side: Side::Below }
    }

    pub(crate) fn inside(self, area: Bounds<Pixels>) -> Self {
        Self { area: Some(area), ..self }
    }

    pub(crate) fn along(self, anchor: TipAnchor) -> Self {
        Self { anchor: Some(anchor), ..self }
    }

    pub(crate) fn at_text(self) -> Self {
        Self { start: Start::Text, ..self }
    }

    pub(crate) fn on_left(self) -> Self {
        Self { side: Side::Left, ..self }
    }

    pub(crate) fn keys(action: &dyn Action, window: &Window, cx: &App) -> Option<Self> {
        let binding = window.highest_precedence_binding_for_action_in(action, &window.focused(cx)?)?;
        let keys: Option<Vec<String>> = binding
            .keystrokes()
            .iter()
            .map(|keystroke| Shortcut::parse(&keystroke.inner().unparse()).map(|shortcut| shortcut.to_string()))
            .collect();
        Some(Self { fit: Fit::Text, ..Self::new(keys?.join(" ")) })
    }
}

pub(crate) fn icon_tip(button: Stateful<Div>, tip: Option<Tip>) -> Stateful<Div> {
    let Some(tip) = tip else {
        return button;
    };
    let glyph = Rc::new(Cell::new(None));
    let measured = glyph.clone();
    let margin = rem((ICON_BUTTON - GLYPH) / 2.0);
    let button = button.relative().child(
        canvas(move |bounds, _, _| measured.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .top(margin)
            .left(margin)
            .size(rem(GLYPH)),
    );
    with_tip(button, tip, glyph)
}

#[derive(Clone, Default)]
pub(crate) struct TipAnchor(Rc<Cell<Option<Bounds<Pixels>>>>);

impl TipAnchor {
    pub(crate) fn track(&self) -> impl IntoElement {
        let cell = self.0.clone();
        canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {}).absolute().top_0().left_0().size_full()
    }
}

#[derive(Default)]
struct Scrolls(u64);

impl Global for Scrolls {}

pub(super) fn hide_tips(cx: &mut App) {
    cx.default_global::<Scrolls>().0 += 1;
}

pub(crate) fn hide_tips_on_keys(cx: &mut App) -> Subscription {
    cx.observe_keystrokes(|_, window, cx| {
        hide_tips(cx);
        window.refresh();
    })
}

pub(super) fn with_tip(element: Stateful<Div>, tip: Tip, clipped: Rc<Cell<Option<Bounds<Pixels>>>>) -> Stateful<Div> {
    element.tooltip(move |_, cx| {
        let scrolls = cx.default_global::<Scrolls>().0;
        let placed = clipped.get().map(|own| {
            let band = tip.anchor.as_ref().and_then(|anchor| anchor.0.get());
            let anchor = band.unwrap_or(own);
            let gap = match (tip.fit, band) {
                (Fit::Text, Some(_)) => INSET,
                (Fit::Text, None) | (Fit::Anchor, _) => FILL_GAP,
            };
            let anchor = match (tip.fit, tip.start) {
                (Fit::Text, _) => {
                    Bounds::from_corners(point(own.left(), anchor.top()), point(own.right(), anchor.bottom()))
                }
                (Fit::Anchor, Start::Anchor) => anchor,
                (Fit::Anchor, Start::Text) => {
                    Bounds::from_corners(point(own.left(), anchor.top()), anchor.bottom_right())
                }
            };
            Placed { text: tip.text.clone(), area: tip.area, anchor, fit: tip.fit, side: tip.side, gap }
        });
        cx.new(|_| TipCard { placed, scrolls }).into()
    })
}

struct Placed {
    text: SharedString,
    area: Option<Bounds<Pixels>>,
    anchor: Bounds<Pixels>,
    fit: Fit,
    side: Side,
    gap: f32,
}

struct TipCard {
    placed: Option<Placed>,
    scrolls: u64,
}

impl Render for TipCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scrolled = cx.try_global::<Scrolls>().map_or(0, |scrolls| scrolls.0) != self.scrolls;
        let Some(Placed { text, area, anchor, fit, side, gap }) = self.placed.as_ref().filter(|_| !scrolled) else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>();
        let colors = theme.tooltip;
        let scale = window.scale_factor();
        let area = area.unwrap_or_else(|| Bounds::new(Point::default(), window.viewport_size()));
        let area = room(area, *anchor, pixels(INSET, window));
        let inset = pixels(INDENT, window) + hairline(window);
        let span = match fit {
            Fit::Anchor => anchor.size.width,
            Fit::Text => area.size.width,
        };
        let room = f32::from(span - inset * 2.);
        let run = TextRun {
            len: text.len(),
            font: font(theme.font.clone()),
            color: colors.text,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(text.clone(), pixels(DETAIL_SIZE, window), &[run], None);
        let x = |index: usize| f32::from(shaped.x_for_index(index));
        let lines = wrap(text, room, x);
        let widest = lines.iter().map(|line| x(line.end) - x(line.start)).fold(0.0, f32::max);
        let width = match fit {
            Fit::Anchor if lines.len() > 1 => span,
            Fit::Anchor | Fit::Text => ceil_device(px(widest) + inset * 2., scale).min(span),
        };
        let height = pixels(DETAIL_LINE, window) * lines.len() as f32 + pixels(FILL_GAP, window) * 2.;
        let gap = pixels(*gap, window);
        let (below, above) = (anchor.bottom() + gap, anchor.top() - gap - height);
        let (top, left) = match side {
            Side::Left => (anchor.center().y - height / 2., anchor.left() - gap - width),
            Side::Below => {
                let top = if below + height <= area.bottom() || above < area.top() { below } else { above };
                let left = match fit {
                    Fit::Anchor => anchor.left(),
                    Fit::Text => anchor.center().x - width / 2.,
                };
                (top, left)
            }
        };
        let top = top.min(area.bottom() - height).max(area.top());
        let left = left.min(area.right() - width).max(area.left());
        let radius = theme.radius.small;
        let card = div()
            .relative()
            .w(width)
            .px(rem(INDENT))
            .py(rem(FILL_GAP))
            .rounded(rem(radius))
            .bg(colors.background)
            .font_family(theme.font.clone())
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(colors.text)
            .children(lines.into_iter().map(|line| {
                div().whitespace_nowrap().child(SharedString::from(text.get(line).unwrap_or_default().to_owned()))
            }))
            .child(frame(radius, colors.frame));
        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .position(point(snap(left, scale), snap(top, scale)))
            .child(card)
            .into_any_element()
    }
}

fn room(area: Bounds<Pixels>, anchor: Bounds<Pixels>, margin: Pixels) -> Bounds<Pixels> {
    let start = (area.left() + margin).min(anchor.left().max(area.left()));
    let end = (area.right() - margin).max(anchor.right().min(area.right()));
    Bounds::from_corners(point(start, area.top() + margin), point(end, area.bottom() - margin))
}

fn wrap(text: &str, width: f32, x: impl Fn(usize) -> f32) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let base = x(start);
        let (mut fit, mut cut) = (None, None);
        for (offset, ch) in text.get(start..).unwrap_or_default().char_indices() {
            let end = start + offset + ch.len_utf8();
            let reach = x(end) - base;
            if reach > width && fit.is_some() {
                break;
            }
            fit = Some(end);
            if BREAKS.contains(&ch) && reach >= width * BREAK_SHARE {
                cut = Some(end);
            }
        }
        let Some(fit) = fit else { break };
        let end = if fit == text.len() { fit } else { cut.unwrap_or(fit) };
        lines.push(start..end);
        start = end;
    }
    lines
}

pub(super) fn ceil_device(value: Pixels, scale: f32) -> Pixels {
    px((f32::from(value) * scale).ceil() / scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monospace(text: &str) -> impl Fn(usize) -> f32 + '_ {
        |index| text.get(..index).map_or(0, |head| head.chars().count()) as f32
    }

    fn lines(text: &str, width: f32) -> Vec<&str> {
        wrap(text, width, monospace(text)).into_iter().filter_map(|line| text.get(line)).collect()
    }

    fn edges(left: f32, top: f32, right: f32, bottom: f32) -> Bounds<Pixels> {
        Bounds::from_corners(point(px(left), px(top)), point(px(right), px(bottom)))
    }

    fn sides(bounds: Bounds<Pixels>) -> [f32; 4] {
        [bounds.left(), bounds.top(), bounds.right(), bounds.bottom()].map(f32::from)
    }

    #[test]
    fn a_tip_keeps_clear_of_the_edges_but_may_line_up_with_an_anchor_closer_to_them() {
        let area = edges(0.0, 0.0, 400.0, 300.0);
        let margin = px(INSET);
        assert_eq!(sides(room(area, edges(4.0, 40.0, 396.0, 76.0), margin)), [4.0, 8.0, 396.0, 292.0]);
        assert_eq!(sides(room(area, edges(16.0, 40.0, 200.0, 76.0), margin)), [8.0, 8.0, 392.0, 292.0]);
    }

    #[test]
    fn a_short_text_stays_on_one_line() {
        assert_eq!(lines("readme.md", 20.0), ["readme.md"]);
    }

    #[test]
    fn lines_are_filled_instead_of_breaking_early() {
        assert_eq!(lines("C:\\a\\0123456789abcdef", 10.0), ["C:\\a\\01234", "56789abcde", "f"]);
    }

    #[test]
    fn a_separator_near_the_end_of_a_line_is_the_break() {
        assert_eq!(lines("C:\\Users\\Alex\\Desktop", 10.0), ["C:\\Users\\", "Alex\\Deskt", "op"]);
        assert_eq!(lines("Developer PowerShell for VS", 12.0), ["Developer ", "PowerShell ", "for VS"]);
    }

    #[test]
    fn non_ascii_text_breaks_between_characters() {
        assert_eq!(lines("документы", 4.0), ["доку", "мент", "ы"]);
    }
}
