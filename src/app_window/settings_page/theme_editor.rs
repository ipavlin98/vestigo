mod basics;
mod paint;
mod panes;
mod rows;

use super::super::menus::{MenuId, Menus, dropdown};
use super::{Folds, change, confirmed};
use crate::i18n::{Language, Strings, word};
use crate::input::{Changed, TextInput};
use crate::platform;
use crate::settings::{self, Settings};
use crate::theme::{
    Appearance, Catalog, Pane, Sheet, Slot, Theme, ThemeChoice, ThemeEdits, file_stem, hex, hex_alpha, parse_hex,
    parse_hex_alpha, rem,
};
use crate::ui::{self, Choice, ColorPick, Hsv, LABEL_GAP, Picker, Slider, slider};
use gpui::{
    AnyElement, App, AppContext, Context, Div, ElementId, Entity, Focusable, Hsla, SharedString, Subscription, Task,
    WeakEntity, Window, div, prelude::*,
};
use paint::{follow_background, paint_color, paint_value, reset_colors, set_paint, store, translucent};
use serde_json::Value;

const ZOOMS: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
const HUNDRED: f64 = 100.0;
const ACCENT: &str = "accent";
const BACKGROUND: &str = "background";
const TEXT: &str = "text";
const SHADOW: &str = "shadow";
const RADIUS: Scale = Scale { start: 0.0, end: 16.0, step: 1.0, percent: false };
const TOGGLE_RADIUS: Scale = Scale { start: 0.0, end: 12.0, step: 1.0, percent: false };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app_window) enum Fold {
    Pane(Pane),
    Group(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app_window) enum Unfold {
    Advanced,
    Fold(Fold),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app_window) enum Paint {
    Accent,
    Background,
    Text,
    Palette(usize),
    Element(usize, usize),
    Layer(Pane, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    Zoom,
    Font,
    Rounding,
    Clarity,
    Role(Paint),
}

impl MenuId for Menu {
    fn id(self) -> &'static str {
        match self {
            Self::Zoom => "theme-zoom",
            Self::Font => "font",
            Self::Rounding => "rounding",
            Self::Clarity => "clarity",
            Self::Role(_) => "role",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Role {
    Named(SharedString),
    Custom,
}

enum Saved {
    Done(String),
    Failed,
}

struct Picking {
    paint: Paint,
    hsv: Hsv,
    alpha: Option<f32>,
}

impl Picking {
    fn field(&self) -> String {
        match self.alpha {
            Some(alpha) => hex_alpha(self.hsv.color().opacity(alpha)),
            None => hex(self.hsv.color()),
        }
    }
}

#[derive(Clone, Copy)]
struct Scale {
    start: f64,
    end: f64,
    step: f64,
    percent: bool,
}

impl Scale {
    fn fraction(self, value: f64) -> f32 {
        ((value - self.start) / (self.end - self.start)).clamp(0.0, 1.0) as f32
    }

    fn value(self, fraction: f32) -> f64 {
        let raw = self.start + f64::from(fraction) * (self.end - self.start);
        let stepped = (raw / self.step).round() * self.step;
        ((stepped * HUNDRED).round() / HUNDRED).clamp(self.start, self.end)
    }

    fn label(self, value: f64) -> SharedString {
        if self.percent { percent(value) } else { format!("{} px", value.round()).into() }
    }
}

fn percent(value: f64) -> SharedString {
    format!("{}%", (value * HUNDRED).round()).into()
}

pub(super) struct ThemeEditor {
    this: WeakEntity<Self>,
    menus: Menus<Menu>,
    advanced: bool,
    fold: Option<Fold>,
    split: Vec<Pane>,
    picking: Option<Picking>,
    resetting: Option<Task<()>>,
    deleting: Option<Task<()>>,
    delete_failed: bool,
    saved: Option<Saved>,
    shown: Option<String>,
    fonts: Vec<SharedString>,
    name: Entity<TextInput>,
    hex: Entity<TextInput>,
    _subscriptions: Vec<Subscription>,
}

impl ThemeEditor {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let [name, hex] = [(); 2].map(|_| cx.new(TextInput::field));
        let subscriptions = vec![
            ui::paint_changes(cx),
            cx.subscribe(&name, |this, name, _: &Changed, cx| {
                if !name.read(cx).text().is_empty() {
                    this.saved = None;
                }
                cx.notify();
            }),
            cx.subscribe(&hex, |this, _, _: &Changed, cx| this.hex_changed(cx)),
            ui::paint_changes_of(&name, cx),
            ui::paint_changes_of(&hex, cx),
        ];
        Self {
            this: cx.weak_entity(),
            menus: Menus::default(),
            advanced: false,
            fold: None,
            split: Vec::new(),
            picking: None,
            resetting: None,
            deleting: None,
            delete_failed: false,
            saved: None,
            shown: None,
            fonts: Vec::new(),
            name,
            hex,
            _subscriptions: subscriptions,
        }
    }

    fn listener<E: ?Sized>(
        &self,
        act: impl Fn(&mut Self, &E, &mut Context<Self>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let this = self.this.clone();
        move |event, _, cx| {
            let _ = this.update(cx, |editor, cx| act(editor, event, cx));
        }
    }

    fn picker<T>(&self, menu: Menu, choices: Vec<Choice<T>>, current: T, theme: &Theme) -> Picker<T> {
        self.menus.picker(menu, choices, current, theme)
    }

    fn dropdown<T: Clone + PartialEq + 'static>(
        &self,
        menu: Menu,
        picker: Picker<T>,
        apply: impl Fn(T, &mut App) + 'static,
        theme: &Theme,
        window: &Window,
    ) -> Div {
        dropdown(self.this.clone(), |this: &mut Self| &mut this.menus, menu, picker, apply, theme, window)
    }

    pub(super) fn close_menus(&mut self, cx: &mut Context<Self>) {
        if self.menus.close() {
            cx.notify();
        }
    }

    pub(super) fn dismiss(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menus.close() {
            cx.notify();
            return true;
        }
        self.close_picker(cx)
    }

    pub(super) fn inputs(&self) -> [&Entity<TextInput>; 2] {
        [&self.name, &self.hex]
    }

    pub(super) fn set_placeholders(&self, strings: &'static Strings, cx: &mut App) {
        self.name.update(cx, |input, _| input.set_placeholder(strings.theme_name));
        self.hex.update(cx, |input, _| input.set_placeholder("#000000"));
    }

    pub(super) fn restyle(&self, theme: &Theme, cx: &mut App) {
        self.name.update(cx, |input, cx| input.set_under(theme.app.background, cx));
        self.hex.update(cx, |input, cx| input.set_under(theme.dropdown.menu, cx));
    }

    fn shows(&self, unfold: Unfold) -> bool {
        match unfold {
            Unfold::Advanced => self.advanced,
            Unfold::Fold(fold) => self.fold == Some(fold),
        }
    }

    pub(super) fn shows_all(&self, needs: &[Unfold]) -> bool {
        needs.iter().all(|&need| self.shows(need))
    }
}

pub(super) fn current_sheet(cx: &App) -> Option<Sheet> {
    let settings = cx.global::<Settings>();
    let system = Appearance::of(cx.window_appearance());
    cx.global::<Catalog>().current(&settings.theme, system).map(|entry| entry.sheet(&settings.theme_edits))
}

fn edit(cx: &mut App, apply: impl FnOnce(&Sheet, &mut ThemeEdits)) {
    let Some(sheet) = current_sheet(cx) else { return };
    let mut edits = cx.global::<Settings>().theme_edits.clone();
    apply(&sheet, &mut edits);
    if edits != cx.global::<Settings>().theme_edits {
        change(cx, |settings| settings.theme_edits = edits);
    }
}

fn set(slot: Slot, value: Value, cx: &mut App) {
    edit(cx, |sheet, edits| edits.set(sheet, &slot, value));
}

fn role_label(name: &str, strings: &Strings) -> SharedString {
    word(strings.theme_roles, name).map_or_else(|| SharedString::from(name.to_owned()), SharedString::from)
}

fn with_changes(hint: impl Into<String>, changed: usize, language: Language) -> SharedString {
    let hint = hint.into();
    match (changed, hint.is_empty()) {
        (0, _) => hint.into(),
        (_, true) => language.changed(changed).into(),
        (_, false) => format!("{hint} · {}", language.changed(changed)).into(),
    }
}

fn dial(id: String, value: f64, scale: Scale, write: impl Fn(f64, &mut App) + 'static, theme: &Theme) -> AnyElement {
    let dial = Slider { id: ElementId::Name(id.into()), fraction: scale.fraction(value), label: scale.label(value) };
    slider(dial, move |fraction, _, cx| write(scale.value(fraction), cx), theme).into_any_element()
}

pub(super) fn cluster(controls: impl IntoIterator<Item = Option<AnyElement>>) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rem(LABEL_GAP))
        .children(controls.into_iter().flatten())
        .into_any_element()
}

impl ThemeEditor {
    fn unfolded(&self, unfold: Unfold, folds: Folds) -> bool {
        match folds {
            Folds::All => true,
            Folds::Chosen => self.shows(unfold),
        }
    }

    pub(super) fn unfold_all(&mut self, needs: &[Unfold], cx: &mut Context<Self>) {
        for &need in needs {
            if self.shows(need) {
                continue;
            }
            match need {
                Unfold::Advanced => self.toggle_advanced(cx),
                Unfold::Fold(fold) => self.toggle_fold(fold, cx),
            }
        }
    }

    pub(super) fn load_fonts(&mut self, cx: &mut Context<Self>) {
        if !self.fonts.is_empty() {
            return;
        }
        let text = cx.text_system().clone();
        cx.spawn(async move |this, cx| {
            let fonts = cx.background_spawn(async move { text.all_font_names() }).await;
            let fonts = fonts.into_iter().filter(|font| !font.starts_with('.')).map(SharedString::from).collect();
            let _ = this.update(cx, |this, cx| {
                this.fonts = fonts;
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_advanced(&mut self, cx: &mut Context<Self>) {
        self.advanced = !self.advanced;
        self.picking = None;
        self.menus.close();
        cx.notify();
    }

    fn toggle_fold(&mut self, fold: Fold, cx: &mut Context<Self>) {
        self.fold = if self.fold == Some(fold) { None } else { Some(fold) };
        self.picking = None;
        self.menus.close();
        cx.notify();
    }

    fn toggle_picker(&mut self, paint: Paint, cx: &mut Context<Self>) {
        if self.picking.as_ref().is_some_and(|picking| picking.paint == paint) {
            self.close_picker(cx);
            return;
        }
        let Some(sheet) = current_sheet(cx) else { return };
        let Some(color) = paint_color(&sheet, paint) else { return };
        self.open_picker(paint, color, translucent(&sheet, paint), cx);
    }

    fn open_picker(&mut self, paint: Paint, color: Hsla, translucent: bool, cx: &mut Context<Self>) {
        let alpha = translucent.then_some(color.a);
        let picking = Picking { paint, hsv: Hsv::of(color), alpha };
        let value = picking.field();
        self.picking = Some(picking);
        self.resetting = None;
        self.menus.close();
        self.hex.update(cx, |input, cx| input.set_text(&value, cx));
        cx.notify();
    }

    fn pick_custom(&mut self, paint: Paint, cx: &mut Context<Self>) {
        let Some(sheet) = current_sheet(cx) else { return };
        let Some(color) = paint_color(&sheet, paint) else { return };
        let translucent = translucent(&sheet, paint);
        set_paint(paint, if translucent { hex_alpha(color) } else { hex(color) }, cx);
        self.open_picker(paint, color, translucent, cx);
    }

    pub(super) fn close_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.picking.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn pick(&mut self, pick: ColorPick, cx: &mut Context<Self>) {
        let Some(picking) = self.picking.as_mut() else { return };
        match pick {
            ColorPick::Shade(hsv) => picking.hsv = hsv,
            ColorPick::Alpha(alpha) => picking.alpha = picking.alpha.map(|_| alpha),
        }
        let value = picking.field();
        let shade = matches!(pick, ColorPick::Shade(_));
        store(picking, shade, !shade, cx);
        self.hex.update(cx, |input, cx| input.set_text(&value, cx));
        cx.notify();
    }

    fn hex_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.hex.read(cx).text().to_owned();
        let Some(picking) = self.picking.as_mut() else { return };
        let parsed = if picking.alpha.is_some() { parse_hex_alpha(&text) } else { parse_hex(&text) };
        let Some(color) = parsed else { return };
        let typed = Picking { paint: picking.paint, hsv: Hsv::of(color), alpha: picking.alpha.map(|_| color.a) };
        if typed.field() == picking.field() {
            return;
        }
        let (shade, faded) = (hex(color) != hex(picking.hsv.color()), typed.alpha != picking.alpha);
        *picking = typed;
        store(picking, shade, faded, cx);
        cx.notify();
    }

    pub(super) fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = current_sheet(cx) else { return };
        self.resetting = None;
        if self.shown.as_deref() != Some(sheet.id.as_str()) {
            self.shown = Some(sheet.id.clone());
            self.deleting = None;
            self.delete_failed = false;
            self.picking = None;
            self.split.clear();
        }
    }

    fn reset_theme(&mut self, cx: &mut Context<Self>) {
        if !confirmed(self, |this| &mut this.resetting, cx) {
            return;
        }
        self.picking = None;
        edit(cx, |sheet, edits| edits.clear(&sheet.id));
        cx.notify();
    }

    pub(super) fn deleting(&self) -> bool {
        self.deleting.is_some()
    }

    pub(super) fn delete_failed(&self) -> bool {
        self.delete_failed
    }

    pub(super) fn delete_theme(&mut self, id: String, cx: &mut Context<Self>) {
        if !confirmed(self, |this| &mut this.deleting, cx) {
            return;
        }
        let path = settings::theme_path(&id).to_string_lossy().into_owned();
        cx.spawn(async move |this, cx| {
            let reloaded = cx
                .background_spawn(async move {
                    platform::recycle(&[path])?;
                    anyhow::Ok(Catalog::load(settings::theme_files()))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.delete_failed = reloaded.is_err();
                match reloaded {
                    Ok(catalog) => {
                        cx.set_global(catalog);
                        change(cx, |settings| {
                            if settings.theme == ThemeChoice::Named(id.clone()) {
                                settings.theme = ThemeChoice::System;
                            }
                            settings.theme_edits.clear(&id);
                        });
                    }
                    Err(error) => log::warn!("Couldn't delete the theme {id}: {error:#}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.name.focus_handle(cx).is_focused(window) {
            self.save_theme(cx);
            true
        } else if self.hex.focus_handle(cx).is_focused(window) {
            self.close_picker(cx)
        } else {
            false
        }
    }

    fn save_theme(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_owned();
        let Some(sheet) = current_sheet(cx) else { return };
        if name.is_empty() {
            return;
        }
        let text = sheet.export(&name);
        let stem = file_stem(&name);
        let reserved = cx.global::<Catalog>().reserved();
        let original = sheet.id;
        cx.spawn(async move |this, cx| {
            let saved = cx
                .background_spawn(async move {
                    let taken = |id: &str| reserved.iter().any(|known| known.eq_ignore_ascii_case(id));
                    let id = settings::create_theme(&stem, taken, &text)?;
                    anyhow::Ok((id, Catalog::load(settings::theme_files())))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let saved = saved.and_then(|(id, catalog)| {
                    let loaded = catalog.entries().iter().any(|entry| entry.id == id);
                    if loaded { Ok((id, catalog)) } else { Err(anyhow::anyhow!("the new theme {id} didn't load")) }
                });
                this.saved = Some(match saved {
                    Ok((id, catalog)) => {
                        cx.set_global(catalog);
                        change(cx, |settings| {
                            settings.theme = ThemeChoice::Named(id);
                            settings.theme_edits.clear(&original);
                        });
                        this.name.update(cx, |input, cx| input.set_text("", cx));
                        Saved::Done(name)
                    }
                    Err(error) => {
                        log::warn!("Couldn't save the theme: {error:#}");
                        Saved::Failed
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::Scale;
    use crate::i18n::{Language, word};
    use crate::theme::{GROUPS, ROLES};

    #[test]
    fn every_element_of_a_theme_has_a_name_in_every_language() {
        for language in [Language::English, Language::Russian] {
            let strings = language.strings();
            for role in ROLES {
                assert!(word(strings.theme_roles, role).is_some(), "{language:?}: role {role}");
                assert!(word(strings.theme_role_hints, role).is_some(), "{language:?}: what {role} colors");
            }
            for group in &GROUPS {
                assert!(word(strings.theme_groups, group.key).is_some(), "{language:?}: group {}", group.key);
                assert!(word(strings.theme_group_hints, group.key).is_some(), "{language:?}: where {}", group.key);
                for field in group.fields {
                    assert!(word(strings.theme_fields, field).is_some(), "{language:?}: {}.{field}", group.key);
                }
            }
        }
    }

    #[test]
    fn a_dial_lands_on_its_steps_and_stays_in_range() {
        let opacity = Scale { start: 0.3, end: 1.0, step: 0.01, percent: true };
        assert_eq!(opacity.value(0.0), 0.3);
        assert_eq!(opacity.value(1.0), 1.0);
        assert_eq!(opacity.value(0.5), 0.65);
        assert_eq!(opacity.label(0.28), "28%");
        let offset = Scale { start: -32.0, end: 32.0, step: 1.0, percent: false };
        assert_eq!(offset.value(0.5), 0.0);
        assert_eq!(offset.value(0.26), -15.0);
        assert_eq!(offset.fraction(64.0), 1.0, "values past the range keep the knob at the end");
        assert_eq!(offset.label(-4.0), "-4 px");
    }
}
