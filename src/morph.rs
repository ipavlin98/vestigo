use crate::{
    assets,
    settings::{self, Settings},
    theme::{Appearance, Catalog, Theme},
};
use gpui::{
    App, AppContext, Application, Bounds, Context, Corners, Hsla, InteractiveElement, IntoElement, MorphShape,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Point, Render, Size, Styled, TextRenderingMode,
    Window, WindowBounds, WindowOptions, canvas, div, point, px, size,
};
use std::{f32::consts::TAU, rc::Rc, time::Instant};

const WINDOW_WIDTH: f32 = 960.0;
const WINDOW_HEIGHT: f32 = 600.0;

const DROPS: usize = 5;
const DROP_RADIUS: f32 = 28.0;
const DROP_SWELL: f32 = 8.0;
const DROP_ORBIT_SHARE: f32 = 0.22;
const DROP_SPIN: f32 = 0.25;
const DROP_PULSE: f32 = 1.3;
const DROP_SMOOTHNESS: f32 = 56.0;
const CURSOR_RADIUS: f32 = 36.0;

const SHAPE_SMOOTHNESS: f32 = 40.0;
const SHAPE_STEP: f32 = 1.1;
const SHAPE_HOLD: f32 = 0.5;

const ISLAND_SMOOTHNESS: f32 = 44.0;
const ISLAND_CLOSED: (f32, f32) = (128.0, 36.0);
const ISLAND_OPEN: (f32, f32) = (340.0, 156.0);
const ISLAND_OPEN_RADIUS: f32 = 36.0;
const BUBBLE_CLOSED: f32 = 24.0;
const BUBBLE_OPEN: f32 = 52.0;
const BUBBLE_GAP: f32 = 28.0;
const BUBBLE_INSET: f32 = 6.0;

const POINTER_SPRING: Spring = Spring { frequency: 9.0, damping: 0.62 };
const ISLAND_SPRING: Spring = Spring { frequency: 7.5, damping: 0.55 };

pub(crate) fn run() {
    let settings = Settings::load();
    let catalog = Catalog::load(settings::theme_files());
    match gpui_windows::WindowsPlatform::new(false) {
        Ok(windows) => {
            Application::with_platform(Rc::new(windows)).with_assets(assets::Assets).run(move |cx: &mut App| {
                cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
                let system = Appearance::of(cx.window_appearance());
                cx.set_global(catalog.resolve(&settings.theme, system, &settings.theme_edits));
                cx.on_window_closed(|cx, _| cx.quit()).detach();
                let bounds = Bounds::centered(None, size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), cx);
                let options =
                    WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..WindowOptions::default() };
                if let Err(error) = cx.open_window(options, |_, cx| cx.new(|_| Lab::new())) {
                    log::error!("Couldn't open the morph lab: {error:#}");
                    cx.quit();
                }
            })
        }
        Err(error) => log::error!("{error:#}"),
    }
}

struct Lab {
    start: Instant,
    pointer: Option<(Swing, Swing)>,
    island: Swing,
    open: bool,
}

impl Lab {
    fn new() -> Self {
        Self { start: Instant::now(), pointer: None, island: Swing::at(0.0, ISLAND_SPRING), open: false }
    }

    fn now(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    fn follow(&mut self, event: &MouseMoveEvent) {
        let now = Instant::now();
        let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
        match &mut self.pointer {
            Some((along, down)) => {
                along.aim(x, now);
                down.aim(y, now);
            }
            None => self.pointer = Some((Swing::at(x, POINTER_SPRING), Swing::at(y, POINTER_SPRING))),
        }
    }

    fn toggle(&mut self, _: &MouseDownEvent) {
        self.open = !self.open;
        self.island.aim(if self.open { 1.0 } else { 0.0 }, Instant::now());
    }

    fn morphs(&self, area: Size<Pixels>, colors: Colors) -> Vec<(Vec<MorphShape>, Pixels)> {
        let now = self.now();
        let instant = Instant::now();
        let (width, height) = (f32::from(area.width), f32::from(area.height));
        let cursor = self
            .pointer
            .as_ref()
            .map(|(along, down)| circle(point(along.value(instant), down.value(instant)), CURSOR_RADIUS, colors.ink));
        let with_cursor = |mut shapes: Vec<MorphShape>| {
            shapes.extend(cursor);
            shapes
        };
        vec![
            (
                with_cursor(drops(point(width * 0.25, height * 0.5), width.min(height), now, colors)),
                px(DROP_SMOOTHNESS),
            ),
            (with_cursor(shapes(point(width * 0.72, height * 0.3), now, colors)), px(SHAPE_SMOOTHNESS)),
            (
                with_cursor(island(point(width * 0.72, height * 0.72), self.island.value(instant), colors)),
                px(ISLAND_SMOOTHNESS),
            ),
        ]
    }
}

impl Render for Lab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let theme = cx.global::<Theme>();
        let colors = Colors { ink: theme.app.title, wash: theme.results.path };
        let morphs = self.morphs(window.viewport_size(), colors);
        div()
            .size_full()
            .bg(theme.app.background)
            .on_mouse_move(cx.listener(|this, event, _, _| this.follow(event)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event, _, _| this.toggle(event)))
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        for (shapes, smoothness) in &morphs {
                            window.paint_morph(shapes, *smoothness);
                        }
                    },
                )
                .size_full(),
            )
    }
}

#[derive(Clone, Copy)]
struct Colors {
    ink: Hsla,
    wash: Hsla,
}

fn drops(center: Point<f32>, extent: f32, now: f32, colors: Colors) -> Vec<MorphShape> {
    let orbit = extent * DROP_ORBIT_SHARE;
    (0..DROPS)
        .map(|index| {
            let share = index as f32 / DROPS as f32;
            let angle = TAU * share + now * DROP_SPIN;
            let radius = DROP_RADIUS + DROP_SWELL * (now * DROP_PULSE + TAU * share).sin();
            circle(point(center.x + orbit * angle.cos(), center.y + orbit * angle.sin()), radius, colors.wash)
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Frame {
    x: f32,
    width: f32,
    height: f32,
    radius: f32,
}

impl Frame {
    const fn new(x: f32, width: f32, height: f32, radius: f32) -> Self {
        Self { x, width, height, radius }
    }

    fn mix(self, other: Self, amount: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * amount;
        Self::new(
            mix(self.x, other.x),
            mix(self.width, other.width),
            mix(self.height, other.height),
            mix(self.radius, other.radius),
        )
    }

    fn shape(self, center: Point<f32>, color: Hsla) -> MorphShape {
        rounded(point(center.x + self.x, center.y), self.width, self.height, self.radius, color)
    }
}

const SQUARE: Frame = Frame::new(0.0, 120.0, 120.0, 16.0);
const ROUND: Frame = Frame::new(0.0, 120.0, 120.0, 60.0);
const PILL: Frame = Frame::new(0.0, 232.0, 72.0, 36.0);

const KEYFRAMES: [(Frame, Frame); 5] = [
    (SQUARE, SQUARE),
    (ROUND, ROUND),
    (PILL, PILL),
    (Frame::new(-108.0, 72.0, 72.0, 36.0), Frame::new(108.0, 72.0, 72.0, 36.0)),
    (Frame::new(-40.0, 88.0, 88.0, 44.0), Frame::new(52.0, 56.0, 56.0, 28.0)),
];

fn shapes(center: Point<f32>, now: f32, colors: Colors) -> Vec<MorphShape> {
    let period = SHAPE_STEP + SHAPE_HOLD;
    let cycle = now / period;
    let step = cycle.floor() as usize;
    let amount = smoothstep(((cycle.fract() * period) / SHAPE_STEP).min(1.0));
    let frames = |index: usize| KEYFRAMES.get(index % KEYFRAMES.len()).copied().unwrap_or((SQUARE, SQUARE));
    let (from, to) = (frames(step), frames(step + 1));
    vec![from.0.mix(to.0, amount).shape(center, colors.ink), from.1.mix(to.1, amount).shape(center, colors.wash)]
}

fn island(center: Point<f32>, openness: f32, colors: Colors) -> Vec<MorphShape> {
    let mix = |from: f32, to: f32| from + (to - from) * openness;
    let width = mix(ISLAND_CLOSED.0, ISLAND_OPEN.0).max(0.0);
    let height = mix(ISLAND_CLOSED.1, ISLAND_OPEN.1).max(0.0);
    let radius = mix(ISLAND_CLOSED.1 / 2.0, ISLAND_OPEN_RADIUS);
    let bubble = mix(BUBBLE_CLOSED, BUBBLE_OPEN).max(0.0);
    let closed_x = ISLAND_CLOSED.0 / 2.0 - BUBBLE_CLOSED / 2.0 - BUBBLE_INSET;
    let open_x = ISLAND_OPEN.0 / 2.0 + BUBBLE_GAP + BUBBLE_OPEN / 2.0;
    vec![
        rounded(center, width, height, radius, colors.ink),
        circle(point(center.x + mix(closed_x, open_x), center.y), bubble / 2.0, colors.wash),
    ]
}

fn circle(center: Point<f32>, radius: f32, color: Hsla) -> MorphShape {
    rounded(center, radius * 2.0, radius * 2.0, radius, color)
}

fn rounded(center: Point<f32>, width: f32, height: f32, radius: f32, color: Hsla) -> MorphShape {
    MorphShape {
        bounds: Bounds::centered_at(point(px(center.x), px(center.y)), size(px(width), px(height))),
        corner_radii: Corners::all(px(radius.max(0.0))),
        color,
    }
}

fn smoothstep(amount: f32) -> f32 {
    amount * amount * (3.0 - 2.0 * amount)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Spring {
    frequency: f32,
    damping: f32,
}

/// A value swinging toward its target on an underdamped spring, solved in closed form: a pure
/// function of time that only a new target changes, so painting it never mutates anything.
#[derive(Clone, Copy, Debug)]
struct Swing {
    from: f32,
    velocity: f32,
    to: f32,
    since: Instant,
    spring: Spring,
}

impl Swing {
    fn at(value: f32, spring: Spring) -> Self {
        Self { from: value, velocity: 0.0, to: value, since: Instant::now(), spring }
    }

    fn aim(&mut self, to: f32, now: Instant) {
        let (value, velocity) = self.state(now);
        *self = Self { from: value, velocity, to, since: now, spring: self.spring };
    }

    fn value(&self, now: Instant) -> f32 {
        self.state(now).0
    }

    fn state(&self, now: Instant) -> (f32, f32) {
        let time = now.saturating_duration_since(self.since).as_secs_f32();
        let Spring { frequency, damping } = self.spring;
        let decay = damping * frequency;
        let damped = frequency * (1.0 - damping * damping).sqrt();
        let offset = self.from - self.to;
        let lean = (self.velocity + decay * offset) / damped;
        let fade = (-decay * time).exp();
        let (sin, cos) = (damped * time).sin_cos();
        let value = self.to + fade * (offset * cos + lean * sin);
        let velocity = fade * ((lean * damped - decay * offset) * cos - (offset * damped + decay * lean) * sin);
        (value, velocity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const SPRING: Spring = Spring { frequency: 8.0, damping: 0.6 };

    #[test]
    fn a_swing_starts_where_it_was_and_settles_on_the_target() {
        let start = Instant::now();
        let mut swing = Swing::at(10.0, SPRING);
        swing.aim(50.0, start);
        assert!((swing.value(start) - 10.0).abs() < 1e-4);
        assert!((swing.value(start + Duration::from_secs(4)) - 50.0).abs() < 1e-2);
    }

    #[test]
    fn a_new_target_keeps_position_and_speed() {
        let start = Instant::now();
        let mut swing = Swing::at(0.0, SPRING);
        swing.aim(100.0, start);
        let turn = start + Duration::from_millis(100);
        let (value, velocity) = swing.state(turn);
        swing.aim(-100.0, turn);
        let (after, speed) = swing.state(turn);
        assert!((after - value).abs() < 1e-3);
        assert!((speed - velocity).abs() < 1e-2);
        assert!(velocity > 0.0);
    }
}
