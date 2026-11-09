use super::edit::{Sheet, ThemeEdits};
use super::{SYSTEM, Theme, ThemeChoice};
use gpui::{Global, Hsla, WindowAppearance};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

const BASE: &str = include_str!("../../resources/themes/base.json");
const LIGHT: &str = include_str!("../../resources/themes/light.json");
const DARK: &str = include_str!("../../resources/themes/dark.json");
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.5..=2.0;
const SHADOW_REACH: f32 = 128.0;

const EXAMPLES: [(&str, &str); 3] = [
    ("catppuccin-mocha", include_str!("../../resources/themes/catppuccin-mocha.json")),
    ("dracula", include_str!("../../resources/themes/dracula.json")),
    ("soft-sand", include_str!("../../resources/themes/soft-sand.json")),
];

/// The example files older versions copied into the themes folder, now shipped inside the app.
pub(crate) const RETIRED: [(&str, &str); 2] = [
    ("catppuccin-mocha.json", include_str!("../../resources/themes/retired/catppuccin-mocha.json")),
    ("dracula.json", include_str!("../../resources/themes/retired/dracula.json")),
];

pub(crate) type Object = Map<String, Value>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    const ALL: [Self; 2] = [Self::Light, Self::Dark];

    pub(crate) fn of(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::Light,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Light => LIGHT,
            Self::Dark => DARK,
        }
    }
}

/// Where a theme comes from: the two system looks and the examples ship inside the app and can't
/// be deleted, the rest are the user's files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    System(Appearance),
    Example,
    File,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) origin: Origin,
    theme: Theme,
    own: Object,
    source: Object,
}

impl Entry {
    pub(crate) fn deletable(&self) -> bool {
        self.origin == Origin::File
    }

    pub(crate) fn sheet(&self, edits: &ThemeEdits) -> Sheet {
        Sheet::new(self.id.clone(), self.name.clone(), &self.own, &self.source, edits.of(&self.id))
    }

    fn edited(&self, edits: &ThemeEdits) -> Theme {
        let Some(overlay) = edits.of(&self.id) else { return self.theme.clone() };
        let mut source = self.source.clone();
        merge(&mut source, overlay.clone());
        compile(source).unwrap_or_else(|error| {
            log::warn!("Couldn't apply the changes to the theme {}: {error}", self.id);
            self.theme.clone()
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Catalog {
    entries: Vec<Entry>,
    failures: Vec<String>,
}

impl Global for Catalog {}

impl Catalog {
    pub(crate) fn load(files: Vec<(String, String)>) -> Self {
        let mut catalog = Self::default();
        let system =
            Appearance::ALL.map(|appearance| (appearance.id(), appearance.source(), Origin::System(appearance)));
        let examples = EXAMPLES.map(|(id, text)| (id, text, Origin::Example));
        for (id, text, origin) in system.into_iter().chain(examples) {
            match parse(text, id, origin) {
                Ok(entry) => catalog.entries.push(entry),
                Err(error) => log::error!("The built-in {id} theme is broken: {error}"),
            }
        }
        for (id, text) in files {
            let taken = id.eq_ignore_ascii_case(SYSTEM)
                || catalog.entries.iter().any(|entry| entry.id.eq_ignore_ascii_case(&id));
            let parsed = if taken {
                Err("this name is taken by a built-in theme".to_owned())
            } else {
                parse(&text, &id, Origin::File)
            };
            match parsed {
                Ok(entry) => catalog.entries.push(entry),
                Err(error) => {
                    log::warn!("Couldn't load the theme {id}.json: {error}");
                    catalog.failures.push(format!("{id}.json: {error}"));
                }
            }
        }
        catalog
    }

    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(crate) fn failures(&self) -> &[String] {
        &self.failures
    }

    pub(crate) fn current(&self, choice: &ThemeChoice, system: Appearance) -> Option<&Entry> {
        let chosen = match choice {
            ThemeChoice::System => None,
            ThemeChoice::Named(id) => self.entries.iter().find(|entry| &entry.id == id),
        };
        chosen.or_else(|| self.entries.iter().find(|entry| entry.origin == Origin::System(system)))
    }

    pub(crate) fn resolve(&self, choice: &ThemeChoice, system: Appearance, edits: &ThemeEdits) -> Theme {
        self.current(choice, system).map(|entry| entry.edited(edits)).unwrap_or_default()
    }

    pub(crate) fn reserved(&self) -> Vec<String> {
        let built_in = self.entries.iter().filter(|entry| !entry.deletable()).map(|entry| entry.id.clone());
        std::iter::once(SYSTEM.to_owned()).chain(built_in).collect()
    }
}

fn parse(text: &str, id: &str, origin: Origin) -> Result<Entry, String> {
    let mut own = object(text)?;
    let name = match own.remove("name") {
        None => id.to_owned(),
        Some(Value::String(name)) => name,
        Some(_) => return Err("`name` must be a string".to_owned()),
    };
    let source = layered(own.clone())?;
    let theme = compile(source.clone())?;
    Ok(Entry { id: id.to_owned(), name, origin, theme, own, source })
}

fn layered(mut own: Object) -> Result<Object, String> {
    let mut theme = object(BASE)?;
    match own.remove("base") {
        None => {}
        Some(Value::String(base)) => {
            let appearance = Appearance::ALL
                .into_iter()
                .find(|appearance| appearance.id() == base)
                .ok_or_else(|| format!("unknown base `{base}`, it can be `light` or `dark`"))?;
            let mut inherited = object(appearance.source())?;
            inherited.remove("name");
            merge(&mut theme, inherited);
        }
        Some(_) => return Err("`base` must be a string".to_owned()),
    }
    merge(&mut theme, own);
    Ok(theme)
}

fn compile(mut theme: Object) -> Result<Theme, String> {
    let palette = match theme.remove("palette") {
        None => Object::new(),
        Some(Value::Object(palette)) => resolve_palette(palette)?,
        Some(_) => return Err("`palette` must be an object".to_owned()),
    };
    for value in theme.values_mut().filter(|value| value.is_object()) {
        substitute(value, &palette);
    }
    let theme = build(theme)?;
    check(&theme)?;
    Ok(theme)
}

pub(super) fn design() -> Object {
    object(BASE).unwrap_or_default()
}

fn object(text: &str) -> Result<Object, String> {
    match serde_json::from_str(text).map_err(|error| error.to_string())? {
        Value::Object(object) => Ok(object),
        _ => Err("a theme is a JSON object".to_owned()),
    }
}

pub(super) fn merge(target: &mut Object, overlay: Object) {
    for (key, value) in overlay {
        match (target.get_mut(&key), value) {
            (Some(Value::Object(inner)), Value::Object(overlay)) => merge(inner, overlay),
            (_, value) => {
                target.insert(key, value);
            }
        }
    }
}

pub(super) fn resolve_palette(mut palette: Object) -> Result<Object, String> {
    for _ in 0..palette.len() {
        let known = palette.clone();
        let mut changed = false;
        for value in palette.values_mut() {
            if let Value::String(name) = value
                && let Some(Value::String(color)) = known.get(name.as_str())
                && color != name
            {
                *name = color.clone();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for (role, value) in &palette {
        serde_json::from_value::<Hsla>(value.clone()).map_err(|error| format!("palette.{role}: {error}"))?;
    }
    Ok(palette)
}

fn substitute(value: &mut Value, palette: &Object) {
    match value {
        Value::String(name) => {
            if let Some(color) = palette.get(name.as_str()).cloned() {
                *value = color;
            }
        }
        Value::Object(object) => object.values_mut().for_each(|value| substitute(value, palette)),
        Value::Array(values) => values.iter_mut().for_each(|value| substitute(value, palette)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn build(mut sections: Object) -> Result<Theme, String> {
    let theme = Theme {
        font: section(&mut sections, "font")?,
        zoom: section(&mut sections, "zoom")?,
        radius: section(&mut sections, "radius")?,
        launcher: section(&mut sections, "launcher")?,
        app: section(&mut sections, "app")?,
        sidebar: section(&mut sections, "sidebar")?,
        nav: section(&mut sections, "nav")?,
        caption: section(&mut sections, "caption")?,
        input: section(&mut sections, "input")?,
        results: section(&mut sections, "results")?,
        header: section(&mut sections, "header")?,
        status: section(&mut sections, "status")?,
        keys: section(&mut sections, "keys")?,
        icon_button: section(&mut sections, "icon_button")?,
        settings: section(&mut sections, "settings")?,
        toggle: section(&mut sections, "toggle")?,
        button: section(&mut sections, "button")?,
        dropdown: section(&mut sections, "dropdown")?,
        tooltip: section(&mut sections, "tooltip")?,
        field: section(&mut sections, "field")?,
        activity: section(&mut sections, "activity")?,
        slider: section(&mut sections, "slider")?,
        picker: section(&mut sections, "picker")?,
        windows: section(&mut sections, "windows")?,
    };
    match sections.keys().next() {
        Some(key) => Err(format!("unknown section `{key}`")),
        None => Ok(theme),
    }
}

fn section<T: DeserializeOwned>(sections: &mut Object, key: &str) -> Result<T, String> {
    let value = sections.remove(key).ok_or_else(|| format!("`{key}` is missing"))?;
    serde_json::from_value(value).map_err(|error| format!("{key}: {error}"))
}

fn check(theme: &Theme) -> Result<(), String> {
    if !ZOOM_RANGE.contains(&theme.zoom) {
        return Err(format!("zoom must be from {} to {}", ZOOM_RANGE.start(), ZOOM_RANGE.end()));
    }
    let windows = theme.windows.all();
    let corners = windows.iter().flat_map(|chrome| chrome.corners.all());
    let radii = [theme.radius.large, theme.radius.small, theme.radius.toggle];
    if radii.into_iter().chain(corners).any(|radius| radius.is_nan() || radius < 0.0) {
        return Err("radii and corners can't be negative".to_owned());
    }
    let mut layers = windows.iter().flat_map(|chrome| &chrome.shadow);
    let opacities = windows.iter().map(|chrome| chrome.opacity).chain(layers.clone().map(|layer| layer.opacity));
    if opacities.into_iter().any(|opacity| !(0.0..=1.0).contains(&opacity)) {
        return Err("opacity must be from 0 to 1".to_owned());
    }
    let mut lengths = layers.clone().flat_map(|layer| [layer.x, layer.y, layer.spread, layer.blur]);
    if lengths.any(|length| !(-SHADOW_REACH..=SHADOW_REACH).contains(&length)) || layers.any(|layer| layer.blur < 0.0) {
        return Err(format!("shadow lengths must be from -{SHADOW_REACH} to {SHADOW_REACH}, blur can't be negative"));
    }
    if theme.colors().any(|color| color.a < 1.0) {
        return Err("element colors must be opaque, make a window see-through with its opacity".to_owned());
    }
    Ok(())
}
