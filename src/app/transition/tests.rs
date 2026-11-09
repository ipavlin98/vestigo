use super::*;
use crate::theme::ShadowLayer;

const BAR: [f32; 4] = [400.0, 100.0, 1000.0, 520.0];
const WINDOW: [f32; 4] = [200.0, 80.0, 1200.0, 780.0];
const LATE: Duration = Duration::from_secs(3);

fn bar() -> Silhouette {
    Silhouette { left: 400, top: 100, right: 1000, bottom: 520, radii: [8; 4], opacity: 1.0, casts_shadow: true }
}

fn window() -> Silhouette {
    Silhouette { left: 200, top: 80, right: 1200, bottom: 780, radii: [12; 4], opacity: 1.0, casts_shadow: true }
}

fn layer(blur: f32) -> ShadowLayer {
    ShadowLayer { color: Hsla { h: 0.0, s: 0.0, l: 0.0, a: 1.0 }, opacity: 0.2, x: 0.0, y: 8.0, blur, spread: 0.0 }
}

fn side(silhouette: Silhouette, light: f32, shadow: Vec<ShadowLayer>) -> Side {
    let chrome = Chrome { shadow, ..Chrome::default() };
    Side::new(silhouette, Hsla { h: 0.0, s: 0.0, l: light, a: 1.0 }, chrome, 1.0)
}

fn run(start: Instant) -> Run {
    Run::new(side(bar(), 1.0, vec![layer(16.0)]), side(window(), 0.0, vec![layer(40.0), layer(8.0)]), start)
}

#[test]
fn the_first_frame_is_the_bar() {
    let start = Instant::now();
    let shape = run(start).shape(start);
    assert_eq!(shape.edges, BAR);
    assert_eq!(shape.radii, [8.0; 4]);
    assert_eq!(shape.color.a, 0.0);
}

#[test]
fn the_shape_widens_evenly_without_rounding_or_overshoot() {
    let start = Instant::now();
    let run = run(start);
    let mut last = BAR;
    for step in 0..60 {
        let shape = run.shape(start + Duration::from_millis(step * 10));
        let progress = run.progress(start + Duration::from_millis(step * 10));
        for edge in 0..4 {
            let between = |value: f32| value >= BAR[edge].min(WINDOW[edge]) && value <= BAR[edge].max(WINDOW[edge]);
            assert!(between(shape.edges[edge]));
            assert!((shape.edges[edge] - BAR[edge]).abs() >= (last[edge] - BAR[edge]).abs() - 1e-3);
            let share = (shape.edges[edge] - BAR[edge]) / (WINDOW[edge] - BAR[edge]);
            assert!((share - progress).abs() < 1e-3);
        }
        assert!(shape.radii.iter().all(|radius| (8.0..=12.0).contains(radius)));
        last = shape.edges;
    }
}

#[test]
fn it_comes_to_rest_exactly_on_the_window() {
    let start = Instant::now();
    let run = run(start);
    assert!(!run.settled(start + COVER));
    let late = start + LATE;
    assert!(run.settled(late));
    let shape = run.shape(late);
    assert_eq!(shape.edges, WINDOW);
    assert_eq!(shape.radii, [12.0; 4]);
    assert!(shape.color.r < 0.01 && shape.color.a > 0.99);
}

#[test]
fn the_shadow_goes_from_the_bar_to_the_window() {
    let start = Instant::now();
    let run = run(start);
    let blurs = |now: Instant| -> Vec<f32> { run.shades(now).iter().map(|shadow| f32::from(shadow.blur)).collect() };
    assert_eq!(blurs(start), vec![16.0]);
    assert_eq!(blurs(start + GROW / 2), vec![8.0, 28.0], "the first layers pair up, the extra one fades in");
    assert_eq!(blurs(start + LATE), vec![8.0, 40.0], "the theme's first layer is painted last, on top");
}

#[test]
fn a_shadow_both_ends_share_never_changes_on_the_way() {
    let start = Instant::now();
    let both = vec![layer(32.0), layer(6.0)];
    let run = Run::new(side(bar(), 1.0, both.clone()), side(window(), 0.0, both), start);
    for step in 0..40 {
        let shades = run.shades(start + Duration::from_millis(step * 10));
        assert_eq!(shades.len(), 2);
        for (shadow, blur) in shades.iter().zip([6.0, 32.0]) {
            assert_eq!(f32::from(shadow.blur), blur);
            assert!((shadow.color.a - 0.2).abs() < 1e-4, "got {}", shadow.color.a);
        }
    }
}

#[test]
fn each_end_keeps_the_shadow_size_of_its_own_scale() {
    let start = Instant::now();
    let chrome = |blur: f32| Chrome { shadow: vec![layer(blur)], ..Chrome::default() };
    let color = Hsla { h: 0.0, s: 0.0, l: 1.0, a: 1.0 };
    let from = Side::new(bar(), color, chrome(16.0), 1.0);
    let to = Side::new(window(), color, chrome(16.0), 2.0);
    let run = Run::new(from, to, start);
    let cast =
        |now: Instant| run.shades(now).first().map(|shadow| (f32::from(shadow.blur), f32::from(shadow.offset.y)));
    assert_eq!(cast(start), Some((16.0, 8.0)));
    assert_eq!(cast(start + LATE), Some((32.0, 16.0)));
}

#[test]
fn a_maximized_window_casts_no_shadow_at_the_end() {
    let start = Instant::now();
    let maximized = Silhouette { casts_shadow: false, ..window() };
    let run = Run::new(side(bar(), 1.0, vec![layer(16.0)]), side(maximized, 0.0, vec![layer(40.0)]), start);
    assert_eq!(run.shades(start).len(), 1);
    assert!(run.shades(start + LATE).is_empty());
}

#[test]
fn the_shadow_stays_whole_until_the_window_takes_it_over() {
    let start = Instant::now();
    let mut run = run(start);
    let settled = run.shades(start + LATE);
    run.revealed = Some(start + LATE);
    let dissolved = run.shades(start + LATE + REVEAL);
    assert_eq!(dissolved.len(), settled.len());
    for (dissolved, settled) in dissolved.iter().zip(&settled) {
        assert_eq!(dissolved.color.a, settled.color.a);
    }
    assert_eq!(run.shape(start + LATE + REVEAL).color.a, 0.0, "the surface itself is gone by then");
}

#[test]
fn a_translucent_shape_and_the_window_under_it_stay_as_opaque_as_the_window() {
    let start = Instant::now();
    let glass = Silhouette { opacity: 0.6, ..window() };
    let mut run = Run::new(side(bar(), 1.0, Vec::new()), side(glass, 0.0, Vec::new()), start);
    run.revealed = Some(start + LATE);
    for step in 0..=20 {
        let now = start + LATE + REVEAL * step / 20;
        let layer = run.shape(now).color.a;
        let together = 1.0 - (1.0 - run.step(now).window) * (1.0 - layer);
        assert!((together - 0.6).abs() < 1e-4, "got {together}");
    }
}

#[test]
fn the_overlay_leaves_room_for_the_shadows() {
    let area = run(Instant::now()).area();
    let room = 40.0 * SHADOW_REACH + 8.0;
    assert!(area.left as f32 <= WINDOW[0] - room && area.bottom as f32 >= WINDOW[3] + room);
}
