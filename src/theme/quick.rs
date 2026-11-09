use super::catalog::design;
use super::edit::{GROUPS, Pane, Sheet, Slot, hex, number, parse_hex};
use gpui::{Hsla, Rgba};
use serde_json::{Value, json};

pub(crate) const ACCENTS: [&str; 8] =
    ["#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#f97316", "#22c55e", "#14b8a6", "#f59e0b"];
const OFFERED: usize = 7;

const ACCENT_ROLE: &str = "accent";
const ACCENT_ELEMENTS: [(&str, &str); 6] = [
    ("results", "match"),
    ("input", "caret"),
    ("toggle", "on"),
    ("slider", "fill"),
    ("dropdown", "check"),
    ("header", "arrow"),
];
const SHADOWED: [Pane; 3] = [Pane::Launcher, Pane::Explorer, Pane::App];

pub(crate) const BACKGROUNDS: [&str; 8] =
    ["#ffffff", "#f6f3ec", "#eef2f7", "#1f1f23", "#000000", "#0f172a", "#1e1e2e", "#002b36"];

const SHADES: [(&str, f32, f32); 9] = [
    ("surface", 0.0, 0.0),
    ("muted", 0.66, 0.68),
    ("faint", 0.48, 0.47),
    ("selected", 0.056, 0.118),
    ("hover", 0.034, 0.072),
    ("border", 0.09, 0.169),
    ("divider", 0.06, 0.13),
    ("selection", 0.18, 0.257),
    ("track", 0.18, 0.257),
];
const DARK_TEXT: f32 = 0.91;
const LIGHT_TEXT: f32 = 0.93;
const READABLE: f32 = 3.0;
const LUMINANCE: [f32; 3] = [0.2126, 0.7152, 0.0722];
const LINEAR_KNEE: f32 = 0.04045;
const LINEAR_SLOPE: f32 = 12.92;
const GAMMA_OFFSET: f32 = 0.055;
const GAMMA_SCALE: f32 = 1.055;
const GAMMA: f32 = 2.4;
const CONTRAST_OFFSET: f32 = 0.05;

type Look = Vec<(Slot, Value)>;

pub(crate) fn offered(presets: &'static [&'static str], own: Option<Hsla>) -> Vec<(usize, &'static str)> {
    let own = own.map(hex);
    presets.iter().copied().enumerate().filter(|(_, color)| parse_hex(color).map(hex) != own).take(OFFERED).collect()
}

pub(crate) fn accent(color: &str) -> Look {
    std::iter::once((Slot::Palette(ACCENT_ROLE.into()), Value::from(color)))
        .chain(ACCENT_ELEMENTS.map(|(group, field)| (Slot::Element(group, field), Value::from(ACCENT_ROLE))))
        .collect()
}

pub(crate) fn accent_slots() -> Vec<Slot> {
    accent("").into_iter().map(|(slot, _)| slot).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Roundness {
    Square,
    Slight,
    Medium,
    Round,
}

impl Roundness {
    pub(crate) const ALL: [Self; 4] = [Self::Square, Self::Slight, Self::Medium, Self::Round];

    fn radii(self) -> [f64; 4] {
        match self {
            Self::Square => [0.0, 0.0, 0.0, 0.0],
            Self::Slight => [4.0, 2.0, 12.0, 4.0],
            Self::Medium => [6.0, 4.0, 12.0, 8.0],
            Self::Round => [10.0, 6.0, 12.0, 12.0],
        }
    }

    pub(crate) fn look(self) -> Look {
        let [large, small, toggle, window] = self.radii();
        let radii = [("large", large), ("small", small), ("toggle", toggle)];
        let radii = radii.map(|(which, radius)| (Slot::Radius(which), number(radius)));
        let corners = Pane::ALL.map(|pane| (Slot::Corners(pane), outer_corners(pane, window)));
        radii.into_iter().chain(corners).collect()
    }

    pub(crate) fn slots() -> Vec<Slot> {
        Self::Square.look().into_iter().map(|(slot, _)| slot).collect()
    }
}

fn outer_corners(pane: Pane, radius: f64) -> Value {
    let radius = number(radius);
    match pane {
        Pane::Launcher | Pane::App => radius,
        Pane::Dialog => json!({ "bottom_left": radius, "bottom_right": radius }),
        Pane::Explorer => json!({ "top_left": radius }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clarity {
    Solid,
    Light,
    Medium,
    Strong,
}

impl Clarity {
    pub(crate) const ALL: [Self; 4] = [Self::Solid, Self::Light, Self::Medium, Self::Strong];

    fn opacity(self) -> f64 {
        match self {
            Self::Solid => 1.0,
            Self::Light => 0.92,
            Self::Medium => 0.84,
            Self::Strong => 0.72,
        }
    }

    pub(crate) fn look(self) -> Look {
        Pane::ALL.map(|pane| (Slot::Opacity(pane), number(self.opacity()))).to_vec()
    }

    pub(crate) fn slots() -> Vec<Slot> {
        Self::Solid.look().into_iter().map(|(slot, _)| slot).collect()
    }
}

pub(crate) fn translucent(sheet: &Sheet, pane: Pane) -> bool {
    sheet.number(&Slot::Opacity(pane)) < 1.0
}

pub(crate) fn blur_on(sheet: &Sheet) -> bool {
    Pane::ALL.into_iter().any(|pane| sheet.flag(&Slot::Blur(pane)))
}

pub(crate) fn blur(on: bool) -> Look {
    Pane::ALL.map(|pane| (Slot::Blur(pane), Value::Bool(on))).to_vec()
}

pub(crate) fn shadows_on(sheet: &Sheet) -> bool {
    Pane::ALL.into_iter().any(|pane| !sheet.shadow(pane).is_empty())
}

pub(crate) fn no_shadows() -> Look {
    Pane::ALL.map(|pane| (Slot::Shadow(pane), json!([]))).to_vec()
}

pub(crate) fn shadows(sheet: &Sheet) -> Look {
    Pane::ALL
        .map(|pane| {
            let slot = Slot::Shadow(pane);
            let original =
                sheet.original(&slot).filter(|layers| layers.as_array().is_some_and(|layers| !layers.is_empty()));
            let layers = match original {
                Some(layers) => layers.clone(),
                None if SHADOWED.contains(&pane) => default_shadow(),
                None => json!([]),
            };
            (slot, layers)
        })
        .to_vec()
}

fn default_shadow() -> Value {
    json!([
        { "color": "shadow", "opacity": 0.28, "y": 8, "blur": 32 },
        { "color": "shadow", "opacity": 0.14, "y": 1, "blur": 6 }
    ])
}

pub(crate) fn colors(sheet: &Sheet, background: Hsla, text: Hsla) -> Look {
    let lightness = background.l;
    let shades = SHADES.map(|(name, light, dark)| {
        let share = dark + (light - dark) * lightness;
        (role(name), Value::from(hex(mix(background, text, share))))
    });
    let accent = (!accent_chosen(sheet)).then(|| (role(ACCENT_ROLE), accent_value(sheet, background, text)));
    [(role("background"), Value::from(hex(background))), (role("text"), Value::from(hex(text)))]
        .into_iter()
        .chain(shades)
        .chain(accent)
        .chain(restyled(sheet))
        .collect()
}

pub(crate) fn color_slots(sheet: &Sheet) -> Vec<Slot> {
    derived().map(role).chain(restyled(sheet).into_iter().map(|(slot, _)| slot)).collect()
}

fn derived() -> impl Iterator<Item = &'static str> {
    ["background", "text"].into_iter().chain(SHADES.map(|(name, _, _)| name))
}

fn restyled(sheet: &Sheet) -> Look {
    let design = design();
    GROUPS
        .iter()
        .flat_map(|group| group.fields.iter().map(move |field| (group.key, *field)))
        .filter_map(|(group, field)| {
            let role = design.get(group)?.get(field)?.as_str()?;
            let slot = Slot::Element(group, field);
            let own = sheet.original(&slot).and_then(Value::as_str);
            let accented = ACCENT_ELEMENTS.contains(&(group, field));
            (!accented && derived().any(|name| name == role) && own != Some(role)).then(|| (slot, Value::from(role)))
        })
        .collect()
}

pub(crate) fn accent_chosen(sheet: &Sheet) -> bool {
    let slot = role(ACCENT_ROLE);
    let recolored = sheet.edited(&slot) && sheet.color(&slot).map(hex) != theme_accent(sheet).map(hex);
    recolored || sheet.edited_any(&ACCENT_ELEMENTS.map(|(group, field)| Slot::Element(group, field)))
}

pub(crate) fn theme_accent(sheet: &Sheet) -> Option<Hsla> {
    let (background, text) = (sheet.color(&role("background"))?, sheet.color(&role("text"))?);
    Some(readable_accent(sheet, background, text))
}

pub(crate) fn follow_accent(sheet: &Sheet) -> Look {
    let (Some(background), Some(text)) = (sheet.color(&role("background")), sheet.color(&role("text"))) else {
        return Vec::new();
    };
    vec![(role(ACCENT_ROLE), accent_value(sheet, background, text))]
}

fn readable_accent(sheet: &Sheet, background: Hsla, text: Hsla) -> Hsla {
    match sheet.original_color(&role(ACCENT_ROLE)) {
        Some(accent) if contrast(accent, background) >= READABLE => accent,
        _ => text,
    }
}

fn accent_value(sheet: &Sheet, background: Hsla, text: Hsla) -> Value {
    let slot = role(ACCENT_ROLE);
    match sheet.original(&slot) {
        Some(original)
            if sheet.original_color(&slot).is_some_and(|accent| contrast(accent, background) >= READABLE) =>
        {
            original.clone()
        }
        _ => Value::from(hex(text)),
    }
}

fn role(name: &'static str) -> Slot {
    Slot::Palette(name.into())
}

pub(crate) fn text_for(background: Hsla) -> Hsla {
    let (ink, share) = if luminance(background) > luminance(mix(black(), white(), 0.5)) {
        (black(), DARK_TEXT)
    } else {
        (white(), LIGHT_TEXT)
    };
    mix(background, ink, share)
}

pub(crate) fn contrast(first: Hsla, second: Hsla) -> f32 {
    let (first, second) = (luminance(first), luminance(second));
    (first.max(second) + CONTRAST_OFFSET) / (first.min(second) + CONTRAST_OFFSET)
}

fn luminance(color: Hsla) -> f32 {
    let rgb = color.to_rgb();
    let linear = |channel: f32| {
        if channel <= LINEAR_KNEE {
            channel / LINEAR_SLOPE
        } else {
            ((channel + GAMMA_OFFSET) / GAMMA_SCALE).powf(GAMMA)
        }
    };
    let [r, g, b] = LUMINANCE;
    r * linear(rgb.r) + g * linear(rgb.g) + b * linear(rgb.b)
}

fn mix(from: Hsla, to: Hsla, share: f32) -> Hsla {
    let (from, to) = (from.to_rgb(), to.to_rgb());
    let blend = |from: f32, to: f32| from + (to - from) * share;
    Hsla::from(Rgba { r: blend(from.r, to.r), g: blend(from.g, to.g), b: blend(from.b, to.b), a: 1.0 })
}

fn black() -> Hsla {
    Hsla::from(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 })
}

fn white() -> Hsla {
    Hsla::from(Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 })
}
