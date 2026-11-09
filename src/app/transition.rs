use crate::platform::{self, Area, Silhouette};
use crate::theme::Chrome;
use curve::{beneath, minimum_jerk};
use gpui::{
    App, AsyncApp, Bounds, Context, Corners, Hsla, IntoElement, MorphShadow, MorphShape, Render, Rgba, Styled, Task,
    WeakEntity, Window, canvas, div, point, px, size,
};
use std::time::{Duration, Instant};

mod curve;
#[cfg(test)]
mod tests;

const COVER: Duration = Duration::from_millis(64);
const GROW: Duration = Duration::from_millis(300);
const REVEAL: Duration = Duration::from_millis(140);
const WAIT_LIMIT: Duration = Duration::from_millis(1500);
const TICK: Duration = Duration::from_millis(8);
const SETTLED_PIXELS: f32 = 2.0;
const SHADOW_REACH: f32 = 1.5;

/// The search bar growing into the Vestigo window, drawn in a click-through overlay above both.
/// The bar's content dissolves into its plain shape while the shape widens evenly into the
/// window: every edge, corner radius, color and shadow layer goes from the bar's to the window's
/// along the minimum-jerk profile (`curve.rs`). The first frame is exactly the bar and the last
/// exactly the window. Whatever lies under the overlay gives way: the bar's shadow goes as the
/// overlay appears, the window comes forward without its shadow, the overlay's surface dissolves
/// into its content, and the window's shadow replaces the overlay's in one composition.
#[derive(Default)]
pub(crate) struct Transition {
    run: Option<Run>,
    task: Option<Task<()>>,
}

/// One end of the transition: where it is on screen, its color and the shadow it has.
pub(crate) struct Side {
    silhouette: Silhouette,
    color: Hsla,
    shadow: Vec<MorphShadow>,
}

impl Side {
    /// `unit` is device pixels per interface pixel of this end's window, so its shadow keeps the
    /// size of its interface.
    pub(crate) fn new(silhouette: Silhouette, color: Hsla, chrome: Chrome, unit: f32) -> Self {
        let layers = chrome.shadow.iter().filter(|_| silhouette.casts_shadow);
        let shadow = layers
            .map(|layer| {
                let mut color = layer.color;
                color.a *= layer.opacity;
                MorphShadow {
                    offset: point(px(layer.x * unit), px(layer.y * unit)),
                    spread: px(layer.spread * unit),
                    blur: px(layer.blur * unit),
                    color,
                }
            })
            .collect();
        Self { silhouette, color, shadow }
    }
}

#[derive(Clone)]
struct Run {
    start: Instant,
    origin: (i32, i32),
    from: Silhouette,
    to: Silhouette,
    colors: (Rgba, Rgba),
    shadows: (Vec<MorphShadow>, Vec<MorphShadow>),
    revealed: Option<Instant>,
}

/// What the windows under the overlay do on one tick.
struct Step {
    settled: bool,
    revealed: Option<Instant>,
    panel: f32,
    window: f32,
}

impl Transition {
    pub(crate) fn start(
        &mut self,
        from: Side,
        to: Side,
        covered: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Area {
        let run = Run::new(from, to, Instant::now());
        let area = run.area();
        let run = Run { origin: (area.left, area.top), ..run };
        let start = run.start;
        self.run = Some(run);
        self.task = Some(cx.spawn(async move |this, cx| drive(this, start, covered, cx).await));
        cx.notify();
        area
    }

    fn reveal(&mut self, now: Instant) {
        if let Some(run) = self.run.as_mut().filter(|run| run.revealed.is_none()) {
            run.revealed = Some(now);
        }
    }

    pub(crate) fn finish(&mut self, cx: &mut Context<Self>) {
        self.run = None;
        self.task = None;
        cx.notify();
    }
}

async fn drive(this: WeakEntity<Transition>, start: Instant, covered: impl FnOnce(&mut App), cx: &mut AsyncApp) {
    let mut covered = Some(covered);
    let mut settled: Option<Instant> = None;
    loop {
        cx.background_executor().timer(TICK).await;
        let now = Instant::now();
        let Ok(Some(step)) = this.read_with(cx, |transition, _| transition.run.as_ref().map(|run| run.step(now)))
        else {
            return;
        };
        if covered.is_some() {
            platform::veil_panel(step.panel);
        }
        if now >= start + COVER
            && let Some(covered) = covered.take()
        {
            cx.update(covered);
        }
        if settled.is_none() && step.settled {
            settled = Some(now);
            platform::release_app_window();
        }
        if settled.is_some() {
            platform::veil_app_window(step.window);
        }
        platform::paint_transition();
        let waited = settled.is_some_and(|since| now >= since + WAIT_LIMIT);
        if step.revealed.is_none() && (settled.is_some() && platform::app_window_revealed() || waited) {
            let _ = this.update(cx, |transition, _| transition.reveal(now));
        }
        if step.revealed.is_some_and(|revealed| now >= revealed + REVEAL) {
            platform::end_transition();
            let _ = this.update(cx, |transition, cx| transition.finish(cx));
            return;
        }
    }
}

impl Run {
    fn new(from: Side, to: Side, start: Instant) -> Self {
        let tint = |color: Hsla, opacity: f32| {
            let mut rgba = Rgba::from(color);
            rgba.a *= opacity;
            rgba
        };
        Self {
            start,
            origin: (0, 0),
            from: from.silhouette,
            to: to.silhouette,
            colors: (tint(from.color, from.silhouette.opacity), tint(to.color, to.silhouette.opacity)),
            shadows: (from.shadow, to.shadow),
            revealed: None,
        }
    }

    fn share(now: Instant, since: Instant, length: Duration) -> f32 {
        (now.saturating_duration_since(since).as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// How far the shape is from the bar to the window, 0 to 1.
    fn progress(&self, now: Instant) -> f32 {
        minimum_jerk(Self::share(now, self.start, GROW))
    }

    fn settled(&self, now: Instant) -> bool {
        let left = (1.0 - self.progress(now)) * travel(&self.from, &self.to);
        left < SETTLED_PIXELS
    }

    fn gone(&self, now: Instant) -> f32 {
        self.revealed.map_or(0.0, |revealed| minimum_jerk(Self::share(now, revealed, REVEAL)))
    }

    fn step(&self, now: Instant) -> Step {
        let shape = self.shape(now);
        Step {
            settled: self.settled(now),
            revealed: self.revealed,
            panel: beneath(self.from.opacity, shape.color.a),
            window: beneath(self.to.opacity, shape.color.a),
        }
    }

    /// The shape now, in device pixels relative to the overlay.
    fn shape(&self, now: Instant) -> Shaped {
        let progress = self.progress(now);
        let presence = minimum_jerk(Self::share(now, self.start, COVER)) * (1.0 - self.gone(now));
        let (left, top) = (self.origin.0 as f32, self.origin.1 as f32);
        let [l, t, r, b] = std::array::from_fn(|edge| {
            let (from, to) = (sides(&self.from), sides(&self.to));
            mix(from.get(edge).copied().unwrap_or(0.0), to.get(edge).copied().unwrap_or(0.0), progress)
        });
        let radii = std::array::from_fn(|corner| {
            let radius = |silhouette: &Silhouette| silhouette.radii.get(corner).copied().unwrap_or(0) as f32;
            mix(radius(&self.from), radius(&self.to), progress)
        });
        let (from, to) = self.colors;
        let mut color = mix_colors(from, to, progress);
        color.a *= presence;
        Shaped { edges: [l - left, t - top, r - left, b - top], radii, color }
    }

    /// The shadow the shape casts now, last layer first as shadow windows stack them. Layers go
    /// pair by pair from the bar's to the window's, so a shadow both share stays exactly itself;
    /// a layer without a pair fades in or out. It stays whole while the surface dissolves: the
    /// window's own shadow only replaces it at the end.
    fn shades(&self, now: Instant) -> Vec<MorphShadow> {
        let weight = self.progress(now);
        let (bar, window) = &self.shadows;
        let clear = |shadow: &MorphShadow| MorphShadow { color: shadow.color.opacity(0.), ..*shadow };
        (0..bar.len().max(window.len()))
            .rev()
            .filter_map(|layer| {
                let (from, to) = (bar.get(layer), window.get(layer));
                let from = from.copied().or_else(|| to.map(clear))?;
                let to = to.copied().unwrap_or_else(|| clear(&from));
                Some(MorphShadow {
                    offset: point(
                        px(mix(f32::from(from.offset.x), f32::from(to.offset.x), weight)),
                        px(mix(f32::from(from.offset.y), f32::from(to.offset.y), weight)),
                    ),
                    spread: px(mix(f32::from(from.spread), f32::from(to.spread), weight)),
                    blur: px(mix(f32::from(from.blur), f32::from(to.blur), weight)),
                    color: Hsla::from(mix_colors(Rgba::from(from.color), Rgba::from(to.color), weight)),
                })
            })
            .filter(|shadow| shadow.color.a > 0.0)
            .collect()
    }

    /// The overlay: both ends with room for the shadows.
    fn area(&self) -> Area {
        let (from, to) = (&self.from, &self.to);
        let (left, top) = (from.left.min(to.left), from.top.min(to.top));
        let (right, bottom) = (from.right.max(to.right), from.bottom.max(to.bottom));
        let shadow = self
            .shadows
            .0
            .iter()
            .chain(&self.shadows.1)
            .map(|shadow| {
                let offset = f32::from(shadow.offset.x).abs().max(f32::from(shadow.offset.y).abs());
                offset + f32::from(shadow.spread).max(0.0) + f32::from(shadow.blur) * SHADOW_REACH
            })
            .fold(0.0, f32::max);
        let margin = shadow.ceil() as i32 + 1;
        Area { left: left - margin, top: top - margin, right: right + margin, bottom: bottom + margin }
    }
}

#[derive(Clone, Copy, Debug)]
struct Shaped {
    edges: [f32; 4],
    radii: [f32; 4],
    color: Rgba,
}

impl Shaped {
    fn morph(self, scale: f32) -> MorphShape {
        let [left, top, right, bottom] = self.edges.map(|edge| edge / scale);
        let most = ((right - left).min(bottom - top) / 2.0).max(0.0);
        let [top_left, top_right, bottom_right, bottom_left] =
            self.radii.map(|radius| px((radius / scale).clamp(0.0, most)));
        MorphShape {
            bounds: Bounds::new(
                point(px(left), px(top)),
                size(px((right - left).max(0.0)), px((bottom - top).max(0.0))),
            ),
            corner_radii: Corners { top_left, top_right, bottom_right, bottom_left },
            color: Hsla::from(self.color),
        }
    }
}

impl Render for Transition {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(run) = self.run.clone() else { return div().size_full().into_any_element() };
        window.request_animation_frame();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let scale = window.scale_factor();
                let now = Instant::now();
                let shape = [run.shape(now).morph(scale)];
                for shadow in run.shades(now) {
                    let shadow = MorphShadow {
                        offset: shadow.offset.map(|length| length / scale),
                        spread: shadow.spread / scale,
                        blur: shadow.blur / scale,
                        ..shadow
                    };
                    window.paint_morph_shadow(&shape, px(0.), shadow);
                }
                window.paint_morph(&shape, px(0.));
            },
        )
        .size_full()
        .into_any_element()
    }
}

fn sides(silhouette: &Silhouette) -> [f32; 4] {
    [silhouette.left, silhouette.top, silhouette.right, silhouette.bottom].map(|side| side as f32)
}

/// The longest way any edge goes.
fn travel(from: &Silhouette, to: &Silhouette) -> f32 {
    sides(from).iter().zip(sides(to)).map(|(from, to)| (to - from).abs()).fold(0.0, f32::max)
}

fn mix(from: f32, to: f32, amount: f32) -> f32 {
    from + (to - from) * amount
}

fn mix_colors(from: Rgba, to: Rgba, amount: f32) -> Rgba {
    Rgba {
        r: mix(from.r, to.r, amount),
        g: mix(from.g, to.g, amount),
        b: mix(from.b, to.b, amount),
        a: mix(from.a, to.a, amount),
    }
}
