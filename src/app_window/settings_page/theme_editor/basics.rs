use super::super::{Row, setting_row};
use super::ThemeEditor;
use super::{ACCENT, BACKGROUND, Menu, Paint, TEXT, cluster, edit, follow_background, reset_colors, set, set_paint};
use crate::assets;
use crate::i18n::Strings;
use crate::theme::{
    ACCENTS, BACKGROUNDS, Clarity, Pane, Roundness, Sheet, Slot, Theme, accent, accent_chosen, accent_slots, blur,
    blur_on, follow_accent, hex, no_shadows, offered, parse_hex, shadows, shadows_on, text_for, theme_accent,
    translucent,
};
use crate::ui::{Choice, disabled_toggle, icon_button, preset, toggle};
use gpui::{AnyElement, App, SharedString, Window, prelude::*};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick<T> {
    Theme,
    Custom,
    Level(T),
}

type Look = Vec<(Slot, Value)>;

struct Levels<T: 'static> {
    menu: Menu,
    all: &'static [T],
    slots: Vec<Slot>,
    look: fn(T) -> Look,
}

fn pick<T: Copy>(sheet: &Sheet, slots: &[Slot], levels: &[T], look: impl Fn(T) -> Look) -> Pick<T> {
    if !sheet.edited_any(slots) {
        return Pick::Theme;
    }
    levels.iter().copied().find(|level| sheet.wears(&look(*level))).map_or(Pick::Custom, Pick::Level)
}

fn rounding_label(level: Roundness, strings: &Strings) -> &'static str {
    match level {
        Roundness::Square => strings.rounding_square,
        Roundness::Slight => strings.rounding_slight,
        Roundness::Medium => strings.rounding_medium,
        Roundness::Round => strings.rounding_round,
    }
}

fn clarity_label(level: Clarity, strings: &Strings) -> &'static str {
    match level {
        Clarity::Solid => strings.clarity_solid,
        Clarity::Light => strings.clarity_light,
        Clarity::Medium => strings.clarity_medium,
        Clarity::Strong => strings.clarity_strong,
    }
}

fn choices<T: Copy>(
    current: Pick<T>,
    levels: &[T],
    label: impl Fn(T) -> &'static str,
    strings: &Strings,
) -> Vec<Choice<Pick<T>>> {
    let choice = |value, label: &'static str| Choice { value, label: label.into(), note: None, swatch: None };
    std::iter::once(choice(Pick::Theme, strings.as_in_theme))
        .chain(matches!(current, Pick::Custom).then(|| choice(Pick::Custom, strings.own_setting)))
        .chain(levels.iter().map(|level| choice(Pick::Level(*level), label(*level))))
        .collect()
}

impl ThemeEditor {
    pub(super) fn color_rows(
        &self,
        sheet: &Sheet,
        theme: &Theme,
        strings: &Strings,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let text = Slot::Palette(TEXT.into());
        let color = sheet.color(&text).unwrap_or_default();
        let matched = sheet.color(&Slot::Palette(BACKGROUND.into())).map(text_for).map(hex);
        let reset = (sheet.edited(&text) && matched != Some(hex(color))).then(|| {
            icon_button("reset-text", assets::RESET, theme)
                .mx_0()
                .on_click(self.listener(|this, _, cx| {
                    this.close_picker(cx);
                    edit(cx, follow_background);
                }))
                .into_any_element()
        });
        let text = cluster([reset, Some(self.color_control(Paint::Text, color, theme, window, cx))]);
        vec![
            setting_row(
                strings.accent_color,
                vec![strings.accent_hint.into()],
                vec![self.accents(sheet, theme, window, cx)],
            ),
            setting_row(
                strings.background_color,
                vec![strings.background_hint.into()],
                vec![self.backgrounds(sheet, theme, window, cx)],
            ),
            setting_row(strings.text_color, vec![strings.text_hint.into()], vec![text]),
        ]
    }

    pub(super) fn form_rows(
        &self,
        sheet: &Sheet,
        theme: &Theme,
        strings: &'static Strings,
        window: &Window,
    ) -> Vec<Row> {
        let rounding = self.level_menu(
            Levels { menu: Menu::Rounding, all: &Roundness::ALL, slots: Roundness::slots(), look: Roundness::look },
            move |level| rounding_label(level, strings),
            sheet,
            (theme, strings),
            window,
        );
        let clarity = self.level_menu(
            Levels { menu: Menu::Clarity, all: &Clarity::ALL, slots: Clarity::slots(), look: Clarity::look },
            move |level| clarity_label(level, strings),
            sheet,
            (theme, strings),
            window,
        );
        let blurred = blur_on(sheet);
        let blur = if Pane::ALL.into_iter().any(|pane| translucent(sheet, pane)) {
            toggle("theme-blur", blurred, theme)
                .on_click(move |_, _, cx| edit(cx, |sheet, edits| edits.apply(sheet, &blur(!blurred))))
        } else {
            disabled_toggle("theme-blur", theme)
        };
        let lit = shadows_on(sheet);
        let shadow = toggle("theme-shadows", lit, theme).on_click(move |_, _, cx| {
            edit(
                cx,
                |sheet, edits| {
                    if lit { edits.apply(sheet, &no_shadows()) } else { edits.apply(sheet, &shadows(sheet)) }
                },
            )
        });
        vec![
            setting_row(strings.font, vec![strings.font_hint.into()], vec![self.font_menu(sheet, theme, window)]),
            setting_row(strings.rounding, vec![strings.rounding_hint.into()], vec![rounding]),
            setting_row(strings.transparency, vec![strings.transparency_hint.into()], vec![clarity]),
            setting_row(strings.window_blur, vec![strings.window_blur_hint.into()], vec![blur.into_any_element()]),
            setting_row(
                strings.window_shadows,
                vec![strings.window_shadows_hint.into()],
                vec![shadow.into_any_element()],
            ),
        ]
    }

    fn level_menu<T: Copy + PartialEq + 'static>(
        &self,
        Levels { menu, all: levels, slots, look }: Levels<T>,
        label: impl Fn(T) -> &'static str,
        sheet: &Sheet,
        (theme, strings): (&Theme, &Strings),
        window: &Window,
    ) -> AnyElement {
        let current = pick(sheet, &slots, levels, look);
        let picker = self.picker(menu, choices(current, levels, label, strings), current, theme);
        self.dropdown(
            menu,
            picker,
            move |choice, cx| match choice {
                Pick::Theme => edit(cx, |sheet, edits| edits.reset_all(&sheet.id, &slots)),
                Pick::Custom => {}
                Pick::Level(level) => edit(cx, |sheet, edits| edits.apply(sheet, &look(level))),
            },
            theme,
            window,
        )
        .into_any_element()
    }

    fn accents(&self, sheet: &Sheet, theme: &Theme, window: &Window, cx: &App) -> AnyElement {
        let chosen_by_hand = accent_chosen(sheet);
        let presets = offered(&ACCENTS, theme_accent(sheet));
        let chosen = presets
            .iter()
            .map(|(index, _)| *index)
            .find(|index| ACCENTS.get(*index).is_some_and(|color| sheet.wears(&accent(color))));
        let own = preset("accent-theme", theme_accent(sheet), !chosen_by_hand, theme).on_click(|_, _, cx| {
            edit(cx, |sheet, edits| {
                edits.reset_all(&sheet.id, &accent_slots());
                edits.apply(sheet, &follow_accent(sheet));
            })
        });
        let samples = presets.into_iter().map(|(index, color)| {
            preset(("accent", index), parse_hex(color), chosen_by_hand && chosen == Some(index), theme)
                .on_click(move |_, _, cx| edit(cx, |sheet, edits| edits.apply(sheet, &accent(color))))
                .into_any_element()
        });
        let custom = chosen_by_hand && chosen.is_none();
        let current = sheet.color(&Slot::Palette(ACCENT.into()));
        let trigger = preset("accent-custom", current.filter(|_| custom), custom, theme);
        let custom = self.color_control_with(Paint::Accent, trigger, theme, window, cx);
        cluster(std::iter::once(Some(own.into_any_element())).chain(samples.map(Some)).chain([Some(custom)]))
    }

    fn backgrounds(&self, sheet: &Sheet, theme: &Theme, window: &Window, cx: &App) -> AnyElement {
        let slot = Slot::Palette(BACKGROUND.into());
        let edited = sheet.edited(&slot);
        let current = sheet.color(&slot).map(hex);
        let presets = offered(&BACKGROUNDS, sheet.original_color(&slot));
        let chosen = presets
            .iter()
            .find(|(_, color)| parse_hex(color).map(hex) == current)
            .map(|(index, _)| *index)
            .filter(|_| edited);
        let own = preset("background-theme", sheet.original_color(&slot), !edited, theme)
            .on_click(|_, _, cx| edit(cx, reset_colors));
        let samples = presets.into_iter().map(|(index, color)| {
            preset(("background", index), parse_hex(color), chosen == Some(index), theme)
                .on_click(move |_, _, cx| set_paint(Paint::Background, color.to_owned(), cx))
                .into_any_element()
        });
        let custom = edited && chosen.is_none();
        let trigger = preset("background-custom", sheet.color(&slot).filter(|_| custom), custom, theme);
        let custom = self.color_control_with(Paint::Background, trigger, theme, window, cx);
        cluster(std::iter::once(Some(own.into_any_element())).chain(samples.map(Some)).chain([Some(custom)]))
    }

    pub(super) fn font_menu(&self, sheet: &Sheet, theme: &Theme, window: &Window) -> AnyElement {
        let font = SharedString::from(sheet.text(&Slot::Font).unwrap_or_default().to_owned());
        let fonts = &self.fonts;
        let fonts = fonts
            .iter()
            .cloned()
            .chain((!fonts.contains(&font)).then(|| font.clone()))
            .map(|name| Choice { value: name.clone(), label: name, note: None, swatch: None })
            .collect();
        let fonts = self.picker(Menu::Font, fonts, font, theme);
        let fonts = self.dropdown(
            Menu::Font,
            fonts,
            |font, cx| set(Slot::Font, Value::String(font.to_string()), cx),
            theme,
            window,
        );
        cluster([self.reset_icon(&Slot::Font, sheet, theme), Some(fonts.into_any_element())])
    }
}
