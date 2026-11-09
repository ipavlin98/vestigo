use super::tip::hide_tips;
use crate::theme::pixels;
use gpui::{
    DispatchPhase, HitboxBehavior, IntoElement, Pixels, ScrollHandle, ScrollWheelEvent, Styled, canvas, point, px,
};
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(40);
const FIRST_FRAME: Duration = Duration::from_millis(8);
const LONGEST_FRAME: Duration = Duration::from_millis(50);
const SETTLE_EPSILON: f32 = 0.5;
const MIN_SPEED: f32 = 320.0;
const EDGE_ZONE: f32 = 48.0;
const EDGE_SPEED: f32 = 1200.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reveal {
    Nearest,
    WholeRows,
}

pub(crate) fn glide(id: &'static str, handle: ScrollHandle, reveal: Option<(usize, f32, Reveal)>) -> impl IntoElement {
    glider(id, handle, reveal, None)
}

pub(crate) fn glide_rows(id: &'static str, handle: ScrollHandle, rests: Vec<Pixels>) -> impl IntoElement {
    glider(id, handle, None, Some(rests))
}

fn glider(
    id: &'static str,
    handle: ScrollHandle,
    reveal: Option<(usize, f32, Reveal)>,
    rests: Option<Vec<Pixels>>,
) -> impl IntoElement {
    let listened = handle.clone();
    canvas(
        move |bounds, window, cx| {
            let state = window.use_keyed_state(id, cx, |_, _| Glide::default());
            let row = reveal.map(|(index, row, how)| (index, pixels(row, window), how));
            let offset = handle.offset();
            let max = handle.max_offset().y;
            let (next, moving) = state.update(cx, |glide, _| {
                let jump =
                    row.and_then(|(index, row, how)| glide.reveal(offset.y, max, index, row, bounds.size.height, how));
                let next = jump.or_else(|| glide.step(offset.y, Instant::now()));
                (next, glide.moving())
            });
            if let Some(next) = next {
                handle.set_offset(point(offset.x, next));
                hide_tips(cx);
            }
            if moving {
                window.request_animation_frame();
            }
            (window.insert_hitbox(bounds, HitboxBehavior::Normal), state)
        },
        move |_, (hitbox, state), window, _| {
            let view = window.current_view();
            let line = window.line_height();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                let max = listened.max_offset().y;
                if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) || max <= px(0.) {
                    return;
                }
                cx.stop_propagation();
                hide_tips(cx);
                let (current, delta) = (listened.offset().y, event.delta.pixel_delta(line).y);
                state.update(cx, |glide, _| match &rests {
                    Some(rests) => glide.wheel_to_rest(current, max, delta, rests),
                    None => glide.wheel(current, max, delta),
                });
                cx.notify(view);
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn edge_scroll(id: &'static str, handle: ScrollHandle) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let tick = window.use_keyed_state(id, cx, |_, _| None::<Instant>);
            let zone = pixels(EDGE_ZONE, window).min(bounds.size.height / 2.);
            let y = window.mouse_position().y;
            let push = if !cx.has_active_drag() || zone <= px(0.) {
                0.
            } else if y < bounds.top() + zone {
                -((bounds.top() + zone - y) / zone).min(1.)
            } else if y > bounds.bottom() - zone {
                ((y - (bounds.bottom() - zone)) / zone).min(1.)
            } else {
                0.
            };
            let now = Instant::now();
            let last = tick.update(cx, |tick, _| if push == 0. { tick.take() } else { tick.replace(now) });
            if push == 0. {
                return;
            }
            window.request_animation_frame();
            let elapsed = last.map_or(FIRST_FRAME, |last| now.saturating_duration_since(last)).min(LONGEST_FRAME);
            let step = pixels(EDGE_SPEED, window) * (push * push.abs() * elapsed.as_secs_f32());
            let offset = handle.offset();
            let next = (offset.y - step).clamp(-handle.max_offset().y, px(0.));
            if next != offset.y {
                handle.set_offset(point(offset.x, next));
                hide_tips(cx);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(crate) fn open_at(
    id: &'static str,
    handle: ScrollHandle,
    (top, row, content): (Pixels, Pixels, Pixels),
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let opened = window.use_keyed_state(id, cx, |_, _| false);
            if !opened.update(cx, |opened, _| std::mem::replace(opened, true)) {
                let viewport = bounds.size.height;
                let centered = top - (viewport - row) / 2.;
                handle.set_offset(point(px(0.), -centered.clamp(px(0.), (content - viewport).max(px(0.)))));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

pub(crate) fn scroll_to_child(
    id: &'static str,
    handle: ScrollHandle,
    child: Option<usize>,
    request: usize,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            let seen = window.use_keyed_state(id, cx, |_, _| None::<usize>);
            if seen.update(cx, |seen, _| seen.replace(request)) != Some(request)
                && let Some(child) = child
            {
                handle.scroll_to_item(child);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

pub(crate) fn scroll_to_span(
    id: &'static str,
    handle: ScrollHandle,
    span: Option<(f32, f32)>,
    request: usize,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let seen = window.use_keyed_state(id, cx, |_, _| None::<usize>);
            if seen.update(cx, |seen, _| seen.replace(request)) != Some(request)
                && let Some((top, bottom)) = span
            {
                let offset = handle.offset();
                let span = (pixels(top, window), pixels(bottom, window));
                handle.set_offset(point(offset.x, -span_shown(-offset.y, span, bounds.size.height)));
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_full()
}

fn span_shown(shown: Pixels, (top, bottom): (Pixels, Pixels), viewport: Pixels) -> Pixels {
    shown.min(top).max(bottom - viewport)
}

#[derive(Default)]
struct Glide {
    target: Option<Pixels>,
    written: Option<Pixels>,
    tick: Option<Instant>,
    revealed: Option<usize>,
    wanted: Option<Pixels>,
}

fn nearest_rest(wanted: Pixels, max: Pixels, rests: &[Pixels]) -> Pixels {
    let distance = |rest: Pixels| f32::from(rest + wanted).abs();
    let candidates = rests.iter().copied().filter(|&rest| rest <= max).chain([max]);
    -candidates.min_by(|&one, &other| distance(one).total_cmp(&distance(other))).unwrap_or(max)
}

impl Glide {
    fn moving(&self) -> bool {
        self.target.is_some()
    }

    fn destination(&self, current: Pixels) -> Pixels {
        self.target.filter(|_| self.written == Some(current)).unwrap_or(current)
    }

    fn aim(&mut self, current: Pixels, max: Pixels, target: Pixels) {
        self.target = Some(target.clamp(-max, px(0.)));
        self.written = Some(current);
    }

    fn wheel(&mut self, current: Pixels, max: Pixels, delta: Pixels) {
        let destination = self.destination(current);
        self.aim(current, max, destination + delta);
    }

    fn wheel_to_rest(&mut self, current: Pixels, max: Pixels, delta: Pixels, rests: &[Pixels]) {
        let start = self.wanted.filter(|_| self.written == Some(current)).unwrap_or(current);
        let wanted = (start + delta).clamp(-max, px(0.));
        self.aim(current, max, nearest_rest(wanted, max, rests));
        self.wanted = Some(wanted);
    }

    fn reveal(
        &mut self,
        current: Pixels,
        max: Pixels,
        index: usize,
        row: Pixels,
        viewport: Pixels,
        how: Reveal,
    ) -> Option<Pixels> {
        if self.revealed.replace(index) == Some(index) || row <= px(0.) {
            return None;
        }
        let shown = -self.destination(current);
        let start = match how {
            Reveal::Nearest => shown,
            Reveal::WholeRows => row * (shown / row).round(),
        };
        let (top, bottom) = (row * index as f32, row * (index + 1) as f32);
        let target = (-start.min(top).max(bottom - viewport)).clamp(-max, px(0.));
        match how {
            Reveal::Nearest => {
                if target != -shown {
                    self.aim(current, max, target);
                }
                None
            }
            Reveal::WholeRows => {
                self.target = None;
                self.tick = None;
                (target != current).then_some(target)
            }
        }
    }

    fn step(&mut self, current: Pixels, now: Instant) -> Option<Pixels> {
        let Some(target) = self.target.filter(|_| self.written == Some(current)) else {
            self.target = None;
            self.tick = None;
            return None;
        };
        let elapsed = self.tick.map_or(FIRST_FRAME, |tick| now.saturating_duration_since(tick)).min(LONGEST_FRAME);
        let share = 1.0 - (-elapsed.as_secs_f32() / SETTLE.as_secs_f32()).exp();
        let gap = f32::from(target - current);
        let least = (MIN_SPEED * elapsed.as_secs_f32()).max(SETTLE_EPSILON);
        let next =
            if gap.abs() <= least { target } else { current + px(gap.signum() * (gap * share).abs().max(least)) };
        self.written = Some(next);
        if next == target {
            self.target = None;
            self.tick = None;
        } else {
            self.tick = Some(now);
        }
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: f32 = 40.0;
    const VIEWPORT: f32 = 120.0;
    const MAX: f32 = 400.0;
    const NOTCH: f32 = 60.0;
    const FAR: f32 = 1_000.0;
    const SHOWN: f32 = 100.0;
    const BETWEEN: f32 = 52.0;
    const DISPLAY_FRAME: Duration = Duration::from_millis(16);

    fn settle(glide: &mut Glide, mut offset: Pixels) -> (Pixels, u32) {
        let start = Instant::now();
        let mut frames = 0;
        while let Some(next) = glide.step(offset, start + FIRST_FRAME * frames) {
            offset = next;
            frames += 1;
            assert!(frames < 1_000, "the glide never settles");
        }
        (offset, frames)
    }

    #[test]
    fn wheel_notches_add_up_and_stop_at_the_ends() {
        let mut glide = Glide::default();
        glide.wheel(px(0.), px(MAX), px(-NOTCH));
        glide.wheel(px(0.), px(MAX), px(-NOTCH));
        assert_eq!(glide.target, Some(px(-NOTCH * 2.0)));
        let (offset, frames) = settle(&mut glide, px(0.));
        assert_eq!(offset, px(-NOTCH * 2.0));
        assert!(frames > 1, "the list jumped instead of gliding");
        glide.wheel(offset, px(MAX), px(FAR));
        assert_eq!(glide.target, Some(px(0.)));
        glide.wheel(offset, px(MAX), px(-FAR));
        assert_eq!(glide.target, Some(px(-MAX)));
    }

    #[test]
    fn a_span_scrolls_in_by_the_least_and_stays_when_visible() {
        let span = (px(ROW * 2.0), px(ROW * 3.0));
        assert_eq!(span_shown(px(0.), span, px(VIEWPORT)), px(0.), "already visible");
        assert_eq!(span_shown(px(0.), (px(ROW * 4.0), px(ROW * 5.0)), px(VIEWPORT)), px(ROW * 2.0), "below");
        assert_eq!(span_shown(px(ROW * 3.0), span, px(VIEWPORT)), px(ROW * 2.0), "above");
    }

    const PITCH: f32 = 36.0;
    const SHORT: f32 = 52.0;
    const TINY: f32 = 8.0;
    const RESTS: [Pixels; 4] = [px(0.), px(PITCH), px(PITCH * 2.0), px(PITCH * 3.0)];

    #[test]
    fn the_wheel_stops_only_on_row_boundaries_and_the_end() {
        let mut glide = Glide::default();
        glide.wheel_to_rest(px(0.), px(MAX), px(-NOTCH), &RESTS);
        assert_eq!(glide.target, Some(px(-PITCH * 2.0)));
        glide.wheel_to_rest(px(0.), px(MAX), px(-NOTCH), &RESTS);
        assert_eq!(glide.target, Some(px(-PITCH * 3.0)), "notches add up before snapping");
        glide.wheel_to_rest(px(0.), px(MAX), px(-FAR), &RESTS);
        assert_eq!(glide.target, Some(px(-MAX)), "the end of the list is a rest too");
        let mut glide = Glide::default();
        glide.wheel_to_rest(px(0.), px(SHORT), px(-FAR), &RESTS);
        assert_eq!(glide.target, Some(px(-SHORT)), "rests past the end don't count");
    }

    #[test]
    fn small_wheel_steps_add_up_to_a_row() {
        let mut glide = Glide::default();
        for _ in 0..2 {
            glide.wheel_to_rest(px(0.), px(MAX), px(-TINY), &RESTS);
            assert_eq!(glide.target, Some(px(0.)));
        }
        glide.wheel_to_rest(px(0.), px(MAX), px(-TINY), &RESTS);
        assert_eq!(glide.target, Some(px(-PITCH)));
    }

    #[test]
    fn an_outside_jump_restarts_the_wheel_from_where_the_list_is() {
        let mut glide = Glide::default();
        glide.wheel_to_rest(px(0.), px(MAX), px(-NOTCH), &RESTS);
        glide.wheel_to_rest(px(-PITCH * 3.0), px(MAX), px(NOTCH), &RESTS);
        assert_eq!(glide.target, Some(px(-PITCH)), "from the row keys moved to, not from the old wheel position");
    }

    #[test]
    fn a_row_past_an_edge_scrolls_just_into_view() {
        let mut glide = Glide::default();
        glide.reveal(px(0.), px(MAX), 4, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, Some(px(VIEWPORT - ROW * 5.0)));
        let mut glide = Glide::default();
        glide.reveal(px(-SHOWN), px(MAX), 2, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, Some(px(-ROW * 2.0)));
        let mut glide = Glide::default();
        glide.reveal(px(-ROW), px(MAX), 2, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, None, "a fully visible row doesn't move the list");
    }

    #[test]
    fn a_clipped_row_scrolls_in_so_no_row_is_cut() {
        let mut glide = Glide::default();
        glide.reveal(px(-BETWEEN), px(MAX), 1, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, Some(px(-ROW)));
        let mut glide = Glide::default();
        glide.reveal(px(-BETWEEN), px(MAX), 4, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, Some(px(VIEWPORT - ROW * 5.0)));
    }

    #[test]
    fn keys_line_up_whole_rows_even_when_the_row_is_visible() {
        let mut glide = Glide::default();
        glide.reveal(px(-BETWEEN), px(MAX), 2, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, None, "the pointer leaves a fully visible row where it is");
        let mut glide = Glide::default();
        assert_eq!(glide.reveal(px(-BETWEEN), px(MAX), 2, px(ROW), px(VIEWPORT), Reveal::WholeRows), Some(px(-ROW)));
    }

    #[test]
    fn each_key_press_past_the_edge_jumps_one_row() {
        let mut glide = Glide::default();
        assert_eq!(glide.reveal(px(0.), px(MAX), 3, px(ROW), px(VIEWPORT), Reveal::WholeRows), Some(px(-ROW)));
        assert!(!glide.moving(), "keys glided instead of jumping");
        assert_eq!(glide.reveal(px(-ROW), px(MAX), 4, px(ROW), px(VIEWPORT), Reveal::WholeRows), Some(px(-ROW * 2.0)));
    }

    #[test]
    fn a_key_press_stops_a_wheel_glide() {
        let mut glide = Glide::default();
        glide.wheel(px(0.), px(MAX), px(-SHOWN * 2.0));
        assert_eq!(glide.reveal(px(0.), px(MAX), 0, px(ROW), px(VIEWPORT), Reveal::WholeRows), None);
        assert!(!glide.moving());
    }

    #[test]
    fn the_wheel_can_leave_the_selected_row_behind() {
        let mut glide = Glide::default();
        glide.reveal(px(0.), px(MAX), 0, px(ROW), px(VIEWPORT), Reveal::Nearest);
        glide.wheel(px(0.), px(MAX), px(-SHOWN * 2.0));
        let (offset, _) = settle(&mut glide, px(0.));
        glide.reveal(offset, px(MAX), 0, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, None);
        glide.reveal(offset, px(MAX), 1, px(ROW), px(VIEWPORT), Reveal::Nearest);
        assert_eq!(glide.target, Some(px(-ROW)));
    }

    #[test]
    fn a_glide_ends_without_a_slow_crawl() {
        let mut glide = Glide::default();
        glide.wheel(px(0.), px(MAX), px(-NOTCH));
        let start = Instant::now();
        let (mut offset, mut frames) = (px(0.), 0);
        while let Some(next) = glide.step(offset, start + DISPLAY_FRAME * frames) {
            offset = next;
            frames += 1;
        }
        assert_eq!(offset, px(-NOTCH));
        assert!(frames <= 8, "the last pixels crawled for {frames} frames");
    }

    #[test]
    fn an_outside_jump_cancels_the_glide() {
        let mut glide = Glide::default();
        glide.wheel(px(0.), px(MAX), px(-SHOWN * 2.0));
        let start = Instant::now();
        assert!(glide.step(px(0.), start).is_some());
        assert_eq!(glide.step(px(0.), start + FIRST_FRAME), None);
        assert!(!glide.moving());
    }
}
