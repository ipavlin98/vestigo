use super::Corners;
use super::catalog::{Object, merge, resolve_palette};
use gpui::{Hsla, Rgba, SharedString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const HEX_DIGITS: u32 = 16;
const SHORT_HEX: usize = 3;
const LONG_HEX: usize = 6;
const ALPHA_HEX: usize = 2;
const CHANNEL: f32 = 255.0;
const HUNDREDTHS: f64 = 100.0;
const SAME_EPSILON: f64 = 1e-6;
const STEM_LIMIT: usize = 64;
const STEM_FALLBACK: &str = "theme";
const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: [&str; 4] = ["con", "prn", "aux", "nul"];
const NUMBERED: [&str; 2] = ["com", "lpt"];

pub(crate) const ROLES: [&str; 13] = [
    "background",
    "surface",
    "text",
    "muted",
    "faint",
    "accent",
    "selected",
    "hover",
    "border",
    "divider",
    "selection",
    "track",
    "shadow",
];

pub(crate) const RADII: [&str; 3] = ["large", "small", "toggle"];
pub(crate) const CORNERS: [&str; 4] = ["top_left", "top_right", "bottom_right", "bottom_left"];

pub(crate) struct Group {
    pub(crate) key: &'static str,
    pub(crate) fields: &'static [&'static str],
}

pub(crate) const GROUPS: [Group; 20] = [
    Group { key: "launcher", fields: &["background", "divider"] },
    Group { key: "app", fields: &["background", "title", "divider"] },
    Group { key: "sidebar", fields: &["background", "title", "divider"] },
    Group { key: "nav", fields: &["text", "hover", "hover_text", "current", "current_text", "count", "found"] },
    Group { key: "caption", fields: &["glyph", "hover", "close_hover"] },
    Group { key: "input", fields: &["text", "placeholder", "caret", "selection", "scope"] },
    Group {
        key: "results",
        fields: &[
            "name", "match", "path", "detail", "shortcut", "icon", "selected", "picked", "empty", "heading", "loading",
        ],
    },
    Group { key: "header", fields: &["text", "hover_text", "arrow"] },
    Group { key: "status", fields: &["text", "place", "separator"] },
    Group { key: "keys", fields: &["frame", "glyph", "label"] },
    Group { key: "icon_button", fields: &["glyph", "hover_glyph"] },
    Group { key: "settings", fields: &["section", "label", "hint", "value", "divider", "match", "found"] },
    Group { key: "toggle", fields: &["on", "off", "knob"] },
    Group { key: "button", fields: &["text", "disabled", "frame", "hover"] },
    Group {
        key: "dropdown",
        fields: &[
            "text",
            "glyph",
            "frame",
            "hover",
            "menu",
            "option",
            "hover_text",
            "chosen",
            "chosen_text",
            "check",
            "note",
        ],
    },
    Group { key: "tooltip", fields: &["background", "text", "frame"] },
    Group { key: "field", fields: &["frame", "focus", "glyph", "note"] },
    Group { key: "activity", fields: &["empty", "full"] },
    Group { key: "slider", fields: &["track", "fill", "knob", "frame"] },
    Group { key: "picker", fields: &["frame", "hover", "knob", "ring"] },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pane {
    Launcher,
    Dialog,
    Explorer,
    App,
}

impl Pane {
    pub(crate) const ALL: [Self; 4] = [Self::Launcher, Self::Dialog, Self::Explorer, Self::App];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
            Self::Dialog => "dialog",
            Self::Explorer => "explorer",
            Self::App => "app",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Palette(SharedString),
    Element(&'static str, &'static str),
    Font,
    Zoom,
    Radius(&'static str),
    Corners(Pane),
    Opacity(Pane),
    Blur(Pane),
    Shadow(Pane),
}

impl Slot {
    fn path(&self) -> Vec<&str> {
        match self {
            Self::Palette(name) => vec!["palette", name],
            Self::Element(group, field) => vec![group, field],
            Self::Font => vec!["font"],
            Self::Zoom => vec!["zoom"],
            Self::Radius(which) => vec!["radius", which],
            Self::Corners(pane) => vec!["windows", pane.key(), "corners"],
            Self::Opacity(pane) => vec!["windows", pane.key(), "opacity"],
            Self::Blur(pane) => vec!["windows", pane.key(), "blur"],
            Self::Shadow(pane) => vec!["windows", pane.key(), "shadow"],
        }
    }

    fn same(&self, original: &Value, value: &Value) -> bool {
        match self {
            Self::Corners(_) => corners(original) == corners(value),
            Self::Shadow(_) => layers(original) == layers(value),
            Self::Palette(_)
            | Self::Element(..)
            | Self::Font
            | Self::Zoom
            | Self::Radius(_)
            | Self::Opacity(_)
            | Self::Blur(_) => same(original, value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Layer {
    pub(crate) color: String,
    #[serde(default = "full")]
    pub(crate) opacity: f64,
    #[serde(default)]
    pub(crate) x: f64,
    #[serde(default)]
    pub(crate) y: f64,
    #[serde(default)]
    pub(crate) blur: f64,
    #[serde(default)]
    pub(crate) spread: f64,
}

fn full() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ThemeEdits(BTreeMap<String, Object>);

impl ThemeEdits {
    pub(crate) fn of(&self, id: &str) -> Option<&Object> {
        self.0.get(id)
    }

    pub(crate) fn set(&mut self, sheet: &Sheet, slot: &Slot, value: Value) {
        let value = tidy(value);
        let path = slot.path();
        let unchanged = lookup(&sheet.original, &path).is_some_and(|original| slot.same(original, &value));
        let overlay = self.0.entry(sheet.id.clone()).or_default();
        if unchanged {
            remove(overlay, &path);
        } else {
            insert(overlay, &path, value);
        }
        if overlay.is_empty() {
            self.0.remove(&sheet.id);
        }
    }

    pub(crate) fn reset(&mut self, id: &str, slot: &Slot) {
        let Some(overlay) = self.0.get_mut(id) else { return };
        remove(overlay, &slot.path());
        if overlay.is_empty() {
            self.0.remove(id);
        }
    }

    pub(crate) fn apply(&mut self, sheet: &Sheet, look: &[(Slot, Value)]) {
        for (slot, value) in look {
            self.set(sheet, slot, value.clone());
        }
    }

    pub(crate) fn reset_all(&mut self, id: &str, slots: &[Slot]) {
        for slot in slots {
            self.reset(id, slot);
        }
    }

    pub(crate) fn clear(&mut self, id: &str) {
        self.0.remove(id);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Sheet {
    pub(crate) id: String,
    pub(crate) name: String,
    own: Object,
    original: Object,
    merged: Object,
    overlay: Object,
    palette: Object,
}

impl Sheet {
    pub(super) fn new(id: String, name: String, own: &Object, source: &Object, overlay: Option<&Object>) -> Self {
        let overlay = overlay.cloned().unwrap_or_default();
        let mut merged = source.clone();
        merge(&mut merged, overlay.clone());
        let palette = match merged.get("palette") {
            Some(Value::Object(palette)) => resolve_palette(palette.clone()).unwrap_or_default(),
            _ => Object::new(),
        };
        Self { id, name, own: own.clone(), original: source.clone(), merged, overlay, palette }
    }

    pub(crate) fn value(&self, slot: &Slot) -> Option<&Value> {
        lookup(&self.merged, &slot.path())
    }

    pub(crate) fn original(&self, slot: &Slot) -> Option<&Value> {
        lookup(&self.original, &slot.path())
    }

    pub(crate) fn original_color(&self, slot: &Slot) -> Option<Hsla> {
        let palette = match self.original.get("palette") {
            Some(Value::Object(palette)) => resolve_palette(palette.clone()).unwrap_or_default(),
            _ => Object::new(),
        };
        let value = self.original(slot)?.as_str()?;
        let color = palette.get(value).and_then(Value::as_str).unwrap_or(value);
        serde_json::from_value(Value::String(color.to_owned())).ok()
    }

    pub(crate) fn wears(&self, look: &[(Slot, Value)]) -> bool {
        look.iter()
            .all(|(slot, value)| self.value(slot).is_some_and(|current| slot.same(current, &tidy(value.clone()))))
    }

    pub(crate) fn edited_any(&self, slots: &[Slot]) -> bool {
        slots.iter().any(|slot| self.edited(slot))
    }

    pub(crate) fn edited(&self, slot: &Slot) -> bool {
        lookup(&self.overlay, &slot.path()).is_some()
    }

    pub(crate) fn has_edits(&self) -> bool {
        !self.overlay.is_empty()
    }

    pub(crate) fn edits_under(&self, path: &[&str]) -> usize {
        if path.is_empty() {
            return self.overlay.values().map(count).sum();
        }
        lookup(&self.overlay, path).map_or(0, count)
    }

    pub(crate) fn palette(&self) -> Vec<SharedString> {
        ROLES.iter().filter(|role| self.palette.contains_key(**role)).map(|role| SharedString::from(*role)).collect()
    }

    pub(crate) fn paint(&self, value: &str) -> Option<Hsla> {
        let color = match self.palette.get(value) {
            Some(Value::String(color)) => color.as_str(),
            _ => value,
        };
        serde_json::from_value(Value::String(color.to_owned())).ok()
    }

    pub(crate) fn text(&self, slot: &Slot) -> Option<&str> {
        self.value(slot)?.as_str()
    }

    pub(crate) fn color(&self, slot: &Slot) -> Option<Hsla> {
        self.paint(self.text(slot)?)
    }

    pub(crate) fn number(&self, slot: &Slot) -> f64 {
        self.value(slot).and_then(Value::as_f64).unwrap_or_default()
    }

    pub(crate) fn flag(&self, slot: &Slot) -> bool {
        self.value(slot).and_then(Value::as_bool).unwrap_or_default()
    }

    pub(crate) fn corners(&self, pane: Pane) -> Corners {
        self.value(&Slot::Corners(pane)).and_then(corners).unwrap_or_default()
    }

    pub(crate) fn shadow(&self, pane: Pane) -> Vec<Layer> {
        self.value(&Slot::Shadow(pane)).and_then(layers).unwrap_or_default()
    }

    pub(crate) fn export(&self, name: &str) -> String {
        let mut body = self.own.clone();
        merge(&mut body, self.overlay.clone());
        let mut theme = Object::new();
        theme.insert("name".to_owned(), Value::String(name.to_owned()));
        theme.extend(body);
        serde_json::to_string_pretty(&theme).unwrap_or_default()
    }
}

pub(crate) fn number(value: f64) -> Value {
    let rounded = (value * HUNDREDTHS).round() / HUNDREDTHS;
    if rounded.fract() == 0.0 { Value::from(rounded as i64) } else { Value::from(rounded) }
}

pub(crate) fn hex(color: Hsla) -> String {
    let rgb = color.to_rgb();
    let channel = |value: f32| (value.clamp(0.0, 1.0) * CHANNEL).round() as u8;
    format!("#{:02x}{:02x}{:02x}", channel(rgb.r), channel(rgb.g), channel(rgb.b))
}

pub(crate) fn hex_alpha(color: Hsla) -> String {
    let alpha = (color.a.clamp(0.0, 1.0) * CHANNEL).round() as u8;
    if alpha == u8::MAX { hex(color) } else { format!("{}{alpha:02x}", hex(color)) }
}

pub(crate) fn parse_hex(text: &str) -> Option<Hsla> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let wide: String = match digits.len() {
        SHORT_HEX => digits.chars().flat_map(|digit| [digit, digit]).collect(),
        LONG_HEX => digits.to_owned(),
        _ => return None,
    };
    let value = u32::from_str_radix(&wide, HEX_DIGITS).ok()?;
    let channel = |shift: u32| ((value >> shift) & 0xff) as f32 / CHANNEL;
    Some(Hsla::from(Rgba { r: channel(16), g: channel(8), b: channel(0), a: 1.0 }))
}

pub(crate) fn parse_hex_alpha(text: &str) -> Option<Hsla> {
    let text = text.trim();
    let digits = text.strip_prefix('#').unwrap_or(text);
    if digits.len() != LONG_HEX + ALPHA_HEX {
        return parse_hex(digits);
    }
    let (color, alpha) = (digits.get(..LONG_HEX)?, digits.get(LONG_HEX..)?);
    if !alpha.chars().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let alpha = u8::from_str_radix(alpha, HEX_DIGITS).ok()?;
    Some(parse_hex(color)?.opacity(f32::from(alpha) / CHANNEL))
}

pub(crate) fn file_stem(name: &str) -> String {
    let cleaned: String =
        name.chars().map(|char| if FORBIDDEN.contains(&char) || char.is_control() { '-' } else { char }).collect();
    let stem: String =
        cleaned.trim_matches(|char: char| char == '.' || char.is_whitespace()).chars().take(STEM_LIMIT).collect();
    let stem = stem.trim_end_matches(|char: char| char == '.' || char.is_whitespace()).to_owned();
    if stem.is_empty() {
        return STEM_FALLBACK.to_owned();
    }
    let lower = stem.to_lowercase();
    let device = RESERVED.contains(&lower.as_str())
        || NUMBERED.iter().any(|prefix| {
            lower.strip_prefix(prefix).is_some_and(|rest| rest.len() == 1 && rest.chars().all(|c| c.is_ascii_digit()))
        });
    if device { format!("{stem}-{STEM_FALLBACK}") } else { stem }
}

fn tidy(value: Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() && float.fract() == 0.0 => Value::from(float as i64),
            _ => Value::Number(number),
        },
        Value::Array(values) => Value::Array(values.into_iter().map(tidy).collect()),
        Value::Object(object) => Value::Object(object.into_iter().map(|(key, value)| (key, tidy(value))).collect()),
        Value::Null | Value::Bool(_) | Value::String(_) => value,
    }
}

fn corners(value: &Value) -> Option<Corners> {
    serde_json::from_value(value.clone()).ok()
}

fn layers(value: &Value) -> Option<Vec<Layer>> {
    serde_json::from_value(value.clone()).ok()
}

fn same(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.as_f64().unwrap_or_default() - right.as_f64().unwrap_or_default()).abs() < SAME_EPSILON
        }
        (Value::String(left), Value::String(right)) => left.eq_ignore_ascii_case(right),
        _ => left == right,
    }
}

fn count(value: &Value) -> usize {
    match value {
        Value::Object(object) => object.values().map(count).sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => 1,
    }
}

fn lookup<'a>(object: &'a Object, path: &[&str]) -> Option<&'a Value> {
    let (last, parents) = path.split_last()?;
    let mut current = object;
    for key in parents {
        current = current.get(*key)?.as_object()?;
    }
    current.get(*last)
}

fn insert(object: &mut Object, path: &[&str], value: Value) {
    let Some((last, parents)) = path.split_last() else { return };
    let mut current = object;
    for key in parents {
        let child = current.entry((*key).to_owned()).or_insert_with(|| Value::Object(Object::new()));
        if !child.is_object() {
            *child = Value::Object(Object::new());
        }
        let Value::Object(child) = child else { return };
        current = child;
    }
    current.insert((*last).to_owned(), value);
}

fn remove(object: &mut Object, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else { return };
    if rest.is_empty() {
        object.remove(*first);
        return;
    }
    let Some(Value::Object(child)) = object.get_mut(*first) else { return };
    remove(child, rest);
    if child.is_empty() {
        object.remove(*first);
    }
}
