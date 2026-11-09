use super::{Look, draw, margin};
use crate::theme::ShadowLayer;
use gpui::{Hsla, Rgba};

const WIDTH: usize = 400;
const HEIGHT: usize = 300;
const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

fn layer(color: Rgba, opacity: f32, x: f32, y: f32, blur: f32, spread: f32) -> ShadowLayer {
    ShadowLayer { color: Hsla::from(color), opacity, x, y, blur, spread }
}

fn soft() -> Vec<ShadowLayer> {
    vec![layer(BLACK, 0.28, 0.0, 8.0, 32.0, 0.0), layer(BLACK, 0.14, 0.0, 1.0, 6.0, 0.0)]
}

fn alpha(pixels: &[u32], x: usize, y: usize) -> u32 {
    pixels[y * WIDTH + x] >> 24
}

fn pixels(width: usize, height: usize, margin: f32, scale: f32, look: &Look) -> Vec<u32> {
    let mut target = vec![0; width * height];
    draw(&mut target, width, width, height, margin, scale, look);
    target
}

fn shadow(layers: Vec<ShadowLayer>, radii: [f32; 4]) -> (Vec<u32>, usize) {
    let inset = margin(&layers, 1.0);
    (pixels(WIDTH, HEIGHT, inset as f32, 1.0, &Look { layers, radii }), inset as usize)
}

#[test]
fn shadow_fades_out_before_the_bitmap_edge() {
    let (pixels, _) = shadow(soft(), [0.0; 4]);
    for x in 0..WIDTH {
        assert_eq!(alpha(&pixels, x, 0), 0);
        assert_eq!(alpha(&pixels, x, HEIGHT - 1), 0);
    }
    for y in 0..HEIGHT {
        assert_eq!(alpha(&pixels, 0, y), 0);
        assert_eq!(alpha(&pixels, WIDTH - 1, y), 0);
    }
}

#[test]
fn shadow_falls_below_the_window_more_than_above() {
    let (pixels, inset) = shadow(soft(), [0.0; 4]);
    let gap = 12;
    let above = alpha(&pixels, WIDTH / 2, inset - gap);
    let below = alpha(&pixels, WIDTH / 2, HEIGHT - inset + gap);
    assert!(below > above, "below {below}, above {above}");
    assert!(above > 0);
}

#[test]
fn a_sideways_offset_casts_the_shadow_to_that_side() {
    let (pixels, inset) = shadow(vec![layer(BLACK, 0.5, 12.0, 0.0, 16.0, 0.0)], [0.0; 4]);
    let left = alpha(&pixels, inset - 6, HEIGHT / 2);
    let right = alpha(&pixels, WIDTH - inset + 6, HEIGHT / 2);
    assert!(right > left, "right {right}, left {left}");
}

#[test]
fn spread_grows_a_sharp_shadow_by_its_length() {
    let (inset, look) = (16, Look { layers: vec![layer(BLACK, 1.0, 0.0, 0.0, 0.0, 4.0)], radii: [0.0; 4] });
    assert_eq!(margin(&look.layers, 1.0), 4);
    let pixels = pixels(WIDTH, HEIGHT, inset as f32, 1.0, &look);
    assert_eq!(alpha(&pixels, WIDTH / 2, inset - 2), 255);
    assert_eq!(alpha(&pixels, WIDTH / 2, inset - 6), 0);
}

#[test]
fn shadow_leaves_the_window_area_clear() {
    let (pixels, inset) = shadow(soft(), [0.0; 4]);
    for (x, y) in [(inset, inset), (WIDTH / 2, HEIGHT / 2), (WIDTH - inset - 1, HEIGHT - inset - 1)] {
        assert_eq!(alpha(&pixels, x, y), 0, "at {x}, {y}");
    }
    assert!(alpha(&pixels, WIDTH / 2, HEIGHT - inset) > 0);
}

#[test]
fn a_rounded_corner_is_shadowed_where_the_window_is_cut_away() {
    let ((square, inset), (round, _)) = (shadow(soft(), [0.0; 4]), shadow(soft(), [24.0, 0.0, 0.0, 0.0]));
    assert!(alpha(&round, inset, inset) > 0);
    assert_eq!(alpha(&round, inset + 24, inset + 24), 0);
    let outside = inset - 4;
    assert!(alpha(&round, outside, outside) < alpha(&square, outside, outside));
    let far = WIDTH - inset + 4;
    assert_eq!(alpha(&round, far, outside), alpha(&square, far, outside));
}

#[test]
fn shadow_color_is_premultiplied() {
    let red = Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 };
    let (pixels, inset) = shadow(vec![layer(red, 0.5, 0.0, 8.0, 32.0, 0.0)], [0.0; 4]);
    let below = pixels[(HEIGHT - inset + 4) * WIDTH + WIDTH / 2];
    let (alpha, red, green) = (below >> 24, (below >> 16) & 0xff, (below >> 8) & 0xff);
    assert!(alpha > 0);
    assert!(red.abs_diff(alpha) <= 1);
    assert_eq!(green, 0);
    assert!(alpha < 128);
}
