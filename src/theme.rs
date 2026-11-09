mod catalog;
mod edit;
mod quick;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use edit::ROLES;

pub(crate) use catalog::{Appearance, Catalog, Entry, Origin, RETIRED};
pub(crate) use edit::{
    CORNERS, GROUPS, Layer, Pane, RADII, Sheet, Slot, ThemeEdits, file_stem, hex, hex_alpha, number, parse_hex,
    parse_hex_alpha,
};
pub(crate) use quick::{
    ACCENTS, BACKGROUNDS, Clarity, Roundness, accent, accent_chosen, accent_slots, blur, blur_on, color_slots, colors,
    contrast, follow_accent, no_shadows, offered, shadows, shadows_on, text_for, theme_accent, translucent,
};

use gpui::{
    Background, Global, Hsla, Pixels, Rems, SharedString, Window, linear_color_stop, linear_gradient, px, rems,
};
use serde::{Deserialize, Serialize};

pub(crate) const FADE: f32 = 24.0;
pub(crate) const BASE_REM: f32 = 16.0;
pub(crate) const GRID: f32 = 4.0;
const SYSTEM: &str = "system";

pub(crate) fn rem(length: f32) -> Rems {
    rems(length / BASE_REM)
}

pub(crate) fn pixels(length: f32, window: &Window) -> Pixels {
    window.rem_size() * (length / BASE_REM)
}

pub(crate) fn zoom_at(zoom: f32, scale: f32) -> f32 {
    let step = (GRID * zoom * scale).round().max(1.0);
    step / (GRID * scale)
}

pub(crate) fn fade(color: Hsla, angle: f32) -> Background {
    linear_gradient(angle, linear_color_stop(color.opacity(0.), 0.), linear_color_stop(color, 1.))
}

pub(crate) fn snap(value: Pixels, scale: f32) -> Pixels {
    px((f32::from(value) * scale).round() / scale)
}

pub(crate) fn device_pixels(count: f32, scale: f32) -> Pixels {
    px(count.max(1.0) / scale)
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub(crate) enum ThemeChoice {
    #[default]
    System,
    Named(String),
}

impl From<String> for ThemeChoice {
    fn from(id: String) -> Self {
        if id == SYSTEM { Self::System } else { Self::Named(id) }
    }
}

impl From<ThemeChoice> for String {
    fn from(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::System => SYSTEM.to_owned(),
            ThemeChoice::Named(id) => id,
        }
    }
}

macro_rules! colors {
    ($name:ident { $($(#[$meta:meta])* $field:ident),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct $name {
            $($(#[$meta])* pub(crate) $field: Hsla,)*
        }

        impl $name {
            fn all(self) -> Vec<Hsla> {
                vec![$(self.$field),*]
            }
        }
    };
}

colors!(Surface { background, divider });
colors!(Page { background, title, divider });
colors!(Sidebar { background, title, divider });
colors!(Nav { text, hover, hover_text, current, current_text, count, found });
colors!(Caption { glyph, hover, close_hover });
colors!(Input { text, placeholder, caret, selection, scope });
colors!(Results {
    name,
    #[serde(rename = "match")]
    matched,
    path,
    detail,
    shortcut,
    icon,
    selected,
    picked,
    empty,
    heading,
    loading
});
colors!(Header { text, hover_text, arrow });
colors!(StatusBar { text, place, separator });
colors!(Keys { frame, glyph, label });
colors!(IconButton { glyph, hover_glyph });
colors!(SettingsPage {
    section,
    label,
    hint,
    value,
    divider,
    #[serde(rename = "match")]
    matched,
    found
});
colors!(Toggle { on, off, knob });
colors!(Button { text, disabled, frame, hover });
colors!(Dropdown { text, glyph, frame, hover, menu, option, hover_text, chosen, chosen_text, check, note });
colors!(Tooltip { background, text, frame });
colors!(Field { frame, focus, glyph, note });
colors!(Activity { empty, full });
colors!(Slider { track, fill, knob, frame });
colors!(Swatch { frame, hover, knob, ring });

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rounding {
    Large,
    Small,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Radius {
    pub(crate) large: f32,
    pub(crate) small: f32,
    pub(crate) toggle: f32,
}

impl Radius {
    pub(crate) fn of(self, rounding: Rounding) -> f32 {
        match rounding {
            Rounding::Large => self.large,
            Rounding::Small => self.small,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(from = "CornerSpec")]
pub(crate) struct Corners {
    pub(crate) top_left: f32,
    pub(crate) top_right: f32,
    pub(crate) bottom_right: f32,
    pub(crate) bottom_left: f32,
}

impl Corners {
    pub(crate) fn all(self) -> [f32; 4] {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
    }

    pub(crate) fn flipped(self) -> Self {
        Self {
            top_left: self.bottom_left,
            top_right: self.bottom_right,
            bottom_right: self.top_right,
            bottom_left: self.top_left,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CornerSpec {
    Same(f32),
    Each(EachCorner),
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EachCorner {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

impl From<CornerSpec> for Corners {
    fn from(spec: CornerSpec) -> Self {
        match spec {
            CornerSpec::Same(radius) => {
                Self { top_left: radius, top_right: radius, bottom_right: radius, bottom_left: radius }
            }
            CornerSpec::Each(each) => Self {
                top_left: each.top_left,
                top_right: each.top_right,
                bottom_right: each.bottom_right,
                bottom_left: each.bottom_left,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ShadowLayer {
    pub(crate) color: Hsla,
    #[serde(default = "full")]
    pub(crate) opacity: f32,
    #[serde(default)]
    pub(crate) x: f32,
    #[serde(default)]
    pub(crate) y: f32,
    #[serde(default)]
    pub(crate) blur: f32,
    #[serde(default)]
    pub(crate) spread: f32,
}

fn full() -> f32 {
    1.0
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Chrome {
    pub(crate) corners: Corners,
    pub(crate) opacity: f32,
    pub(crate) blur: bool,
    pub(crate) shadow: Vec<ShadowLayer>,
}

impl Chrome {
    pub(crate) fn frosted(&self) -> bool {
        self.blur && self.opacity < 1.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Windows {
    pub(crate) launcher: Chrome,
    pub(crate) dialog: Chrome,
    pub(crate) explorer: Chrome,
    pub(crate) app: Chrome,
}

impl Windows {
    pub(crate) fn all(&self) -> [&Chrome; 4] {
        [&self.launcher, &self.dialog, &self.explorer, &self.app]
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Theme {
    pub(crate) font: SharedString,
    pub(crate) zoom: f32,
    pub(crate) radius: Radius,
    pub(crate) launcher: Surface,
    pub(crate) app: Page,
    pub(crate) sidebar: Sidebar,
    pub(crate) nav: Nav,
    pub(crate) caption: Caption,
    pub(crate) input: Input,
    pub(crate) results: Results,
    pub(crate) header: Header,
    pub(crate) status: StatusBar,
    pub(crate) keys: Keys,
    pub(crate) icon_button: IconButton,
    pub(crate) settings: SettingsPage,
    pub(crate) toggle: Toggle,
    pub(crate) button: Button,
    pub(crate) dropdown: Dropdown,
    pub(crate) tooltip: Tooltip,
    pub(crate) field: Field,
    pub(crate) activity: Activity,
    pub(crate) slider: Slider,
    pub(crate) picker: Swatch,
    pub(crate) windows: Windows,
}

impl Global for Theme {}

impl Theme {
    fn colors(&self) -> impl Iterator<Item = Hsla> {
        [
            self.launcher.all(),
            self.app.all(),
            self.sidebar.all(),
            self.nav.all(),
            self.caption.all(),
            self.input.all(),
            self.results.all(),
            self.header.all(),
            self.status.all(),
            self.keys.all(),
            self.icon_button.all(),
            self.settings.all(),
            self.toggle.all(),
            self.button.all(),
            self.dropdown.all(),
            self.tooltip.all(),
            self.field.all(),
            self.activity.all(),
            self.slider.all(),
            self.picker.all(),
        ]
        .into_iter()
        .flatten()
    }
}
