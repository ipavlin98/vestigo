use super::super::lists::intro;
use super::super::{Block, Folding, Folds, Heading, Row, field_row_with, fold_row, section, setting_row};
use super::ThemeEditor;
use super::{Fold, Menu, Paint, RADIUS, Role, Saved, TOGGLE_RADIUS, Unfold, ZOOMS, cluster, current_sheet, dial, edit};
use super::{paint_color, paint_value, percent, role_label, set, set_paint, with_changes};
use crate::assets;
use crate::i18n::{Language, Strings, word};
use crate::theme::{GROUPS, RADII, Sheet, Slot, Theme, hex_alpha, number};
use crate::ui::{
    CONTROL_WIDTH, Choice, ColorButton, PICKER_INNER, Picked, button, color_button, icon_button, swatch, text_field,
};
use gpui::{
    AnyElement, App, Div, ElementId, Focusable, Hsla, MouseDownEvent, SharedString, Stateful, Window, prelude::*,
};

const ADVANCED: [Unfold; 1] = [Unfold::Advanced];

impl ThemeEditor {
    pub(in super::super) fn section(
        &self,
        title: &str,
        folds: Folds,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Block> {
        let strings = language.strings();
        let Some(sheet) = current_sheet(cx) else { return Vec::new() };
        let advanced = self.unfolded(Unfold::Advanced, folds);
        let fine = fold_row(
            (strings.fine_tuning.into(), strings.fine_tuning_hint.into()),
            Folding {
                id: "fine-tuning".into(),
                open: advanced,
                controls: Vec::new(),
                toggle: Box::new(self.listener(|this, _, cx| this.toggle_advanced(cx))),
                inner: Vec::new(),
                unfold: None,
            },
        );
        let mut sections = vec![
            section(Heading::Title(strings.theme_colors), self.color_rows(&sheet, theme, strings, window, cx)),
            section(Heading::Title(strings.theme_form), self.form_rows(&sheet, theme, strings, window)),
            section(Heading::Title(strings.theme_saving), self.saving_rows(&sheet, title, theme, language, window, cx)),
            section(Heading::Plain, vec![fine]),
        ];
        if advanced {
            let palette = std::iter::once(intro(strings.palette_intro, theme))
                .chain(self.palette_rows(&sheet, theme, strings, window, cx))
                .collect();
            let elements = std::iter::once(intro(strings.elements_intro, theme))
                .chain(self.group_rows(&sheet, folds, theme, language, window, cx))
                .collect();
            let fine = [
                section(Heading::Title(strings.palette), palette),
                section(Heading::Title(strings.shape), self.shape_rows(&sheet, theme, strings, window)),
                section(
                    Heading::Title(strings.windows_section),
                    self.pane_rows(&sheet, folds, theme, language, window, cx),
                ),
                section(Heading::Title(strings.elements), elements),
            ];
            sections.extend(fine.map(|block| block.under(&ADVANCED)));
        }
        sections
    }

    fn saving_rows(
        &self,
        sheet: &Sheet,
        title: &str,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        let edited = sheet.has_edits();
        let label = if self.resetting.is_some() { strings.reset_confirm } else { strings.reset };
        let reset = button("theme-reset", label, edited, theme, self.listener(|this, _, cx| this.reset_theme(cx)));
        let kept = if edited { language.theme_kept(title).into() } else { SharedString::from(strings.theme_untouched) };
        let named = !self.name.read(cx).text().trim().is_empty();
        let focused = self.name.focus_handle(cx).is_focused(window);
        let field = text_field(self.name.clone(), focused, CONTROL_WIDTH, theme);
        let save = button("theme-save", strings.save, named, theme, self.listener(|this, _, cx| this.save_theme(cx)));
        let outcome = match &self.saved {
            Some(Saved::Done(name)) => language.theme_saved(name.as_str()),
            Some(Saved::Failed) => strings.theme_save_failed.to_owned(),
            None => language.new_theme_hint(title),
        };
        vec![
            setting_row(strings.theme_changes, vec![kept], vec![reset.into_any_element()]),
            setting_row(
                strings.new_theme,
                vec![outcome.into()],
                vec![cluster([Some(field.into_any_element()), Some(save.into_any_element())])],
            ),
        ]
    }

    fn palette_rows(&self, sheet: &Sheet, theme: &Theme, strings: &Strings, window: &Window, cx: &App) -> Vec<Row> {
        sheet
            .palette()
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let slot = Slot::Palette(name.clone());
                let color = sheet.color(&slot).unwrap_or_default();
                let raw = sheet.text(&slot).unwrap_or_default();
                let mut hint = word(strings.theme_role_hints, &name).map(str::to_owned).into_iter().collect::<Vec<_>>();
                hint.push(hex_alpha(color));
                if let Some(role) = word(strings.theme_roles, raw) {
                    hint.push(role.to_owned());
                }
                let hint = hint.join(" · ");
                let controls = cluster([
                    self.reset_icon(&slot, sheet, theme),
                    Some(self.color_control(Paint::Palette(index), color, theme, window, cx)),
                ]);
                setting_row(role_label(&name, strings), vec![hint.into()], vec![controls])
            })
            .collect()
    }

    fn shape_rows(&self, sheet: &Sheet, theme: &Theme, strings: &Strings, window: &Window) -> Vec<Row> {
        let radii = RADII.into_iter().zip([strings.radius_large, strings.radius_small, strings.radius_toggle]).map(
            |(which, label)| {
                let scale = if which == "toggle" { TOGGLE_RADIUS } else { RADIUS };
                let slot = Slot::Radius(which);
                let write = move |value, cx: &mut App| set(Slot::Radius(which), number(value), cx);
                let dial = dial(format!("radius-{which}"), sheet.number(&slot), scale, write, theme);
                let controls = cluster([self.reset_icon(&slot, sheet, theme), Some(dial)]);
                setting_row(label, Vec::new(), vec![controls])
            },
        );
        let radii: Vec<Row> = radii.collect();
        let zoom = sheet.number(&Slot::Zoom);
        let zooms = ZOOMS
            .iter()
            .copied()
            .chain((!ZOOMS.contains(&zoom)).then_some(zoom))
            .map(|value| Choice { value, label: percent(value), note: None, swatch: None })
            .collect();
        let zooms = self.picker(Menu::Zoom, zooms, zoom, theme);
        let zooms = self.dropdown(Menu::Zoom, zooms, |zoom, cx| set(Slot::Zoom, number(zoom), cx), theme, window);
        let zooms = cluster([self.reset_icon(&Slot::Zoom, sheet, theme), Some(zooms.into_any_element())]);
        radii
            .into_iter()
            .chain(std::iter::once(setting_row(strings.theme_zoom, vec![strings.theme_zoom_hint.into()], vec![zooms])))
            .collect()
    }

    fn group_rows(
        &self,
        sheet: &Sheet,
        folds: Folds,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        GROUPS
            .iter()
            .enumerate()
            .map(|(group, spec)| {
                let fold = Fold::Group(group);
                let label = word(strings.theme_groups, spec.key).unwrap_or(spec.key);
                let about = word(strings.theme_group_hints, spec.key).unwrap_or_default();
                let hint = with_changes(about, sheet.edits_under(&[spec.key]), language);
                let rows = spec.fields.iter().enumerate().map(|(field, key)| {
                    let paint = Paint::Element(group, field);
                    let slot = Slot::Element(spec.key, key);
                    let color = sheet.color(&slot).unwrap_or_default();
                    let controls = cluster([
                        self.reset_icon(&slot, sheet, theme),
                        Some(self.color_control(paint, color, theme, window, cx)),
                        Some(self.role_menu(paint, sheet, theme, strings, window)),
                    ]);
                    let label = word(strings.theme_fields, key).unwrap_or(key);
                    field_row_with(label.into(), vec![controls]).id(ElementId::Name(format!("{paint:?}").into()))
                });
                self.fold(fold, (label.into(), hint), rows.collect(), folds)
            })
            .collect()
    }

    pub(super) fn fold(&self, fold: Fold, texts: (SharedString, SharedString), rows: Vec<Row>, folds: Folds) -> Row {
        let folding = Folding {
            id: ElementId::Name(format!("fold-{fold:?}").into()),
            open: self.unfolded(Unfold::Fold(fold), folds),
            controls: Vec::new(),
            toggle: Box::new(self.listener(move |this, _, cx| this.toggle_fold(fold, cx))),
            inner: rows,
            unfold: Some(Unfold::Fold(fold)),
        };
        fold_row(texts, folding)
    }

    pub(super) fn reset_icon(&self, slot: &Slot, sheet: &Sheet, theme: &Theme) -> Option<AnyElement> {
        sheet.edited(slot).then(|| {
            let slot = slot.clone();
            icon_button(ElementId::Name(format!("reset-{slot:?}").into()), assets::RESET, theme)
                .mx_0()
                .on_click(self.listener(move |this, _, cx| {
                    this.close_picker(cx);
                    edit(cx, |sheet, edits| edits.reset(&sheet.id, &slot));
                }))
                .into_any_element()
        })
    }

    pub(super) fn color_control(
        &self,
        paint: Paint,
        color: Hsla,
        theme: &Theme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let open = self.picking.as_ref().is_some_and(|picking| picking.paint == paint);
        let trigger = swatch(ElementId::Name(format!("swatch-{paint:?}").into()), color, open, theme);
        self.color_control_with(paint, trigger, theme, window, cx)
    }

    pub(super) fn color_control_with(
        &self,
        paint: Paint,
        trigger: Stateful<Div>,
        theme: &Theme,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let picking = self.picking.as_ref().filter(|picking| picking.paint == paint);
        let trigger = trigger.on_click(self.listener(move |this, _, cx| this.toggle_picker(paint, cx)));
        let hex = &self.hex;
        let picking = picking.map(|picking| Picked {
            hsv: picking.hsv,
            alpha: picking.alpha,
            field: text_field(hex.clone(), hex.focus_handle(cx).is_focused(window), PICKER_INNER, theme),
        });
        let this = self.this.clone();
        color_button(
            ColorButton { swatch: trigger, picking },
            move |pick, _, cx| {
                let _ = this.update(cx, |this, cx| this.pick(pick, cx));
            },
            self.listener(|this, _: &MouseDownEvent, cx| {
                this.close_picker(cx);
            }),
            theme,
            window,
        )
        .into_any_element()
    }

    pub(super) fn role_menu(
        &self,
        paint: Paint,
        sheet: &Sheet,
        theme: &Theme,
        strings: &Strings,
        window: &Window,
    ) -> AnyElement {
        let names = sheet.palette();
        let raw = paint_value(sheet, paint).unwrap_or_default();
        let current = match names.iter().find(|name| name.as_ref() == raw) {
            Some(name) => Role::Named(name.clone()),
            None => Role::Custom,
        };
        let custom = (current == Role::Custom).then(|| paint_color(sheet, paint)).flatten();
        let choices = names
            .iter()
            .map(|name| Choice {
                value: Role::Named(name.clone()),
                label: role_label(name, strings),
                note: None,
                swatch: sheet.paint(name),
            })
            .chain(std::iter::once(Choice {
                value: Role::Custom,
                label: strings.custom_color.into(),
                note: None,
                swatch: custom,
            }))
            .collect();
        let menu = Menu::Role(paint);
        let picker = self.picker(menu, choices, current, theme);
        let this = self.this.clone();
        self.dropdown(
            menu,
            picker,
            move |role, cx| match role {
                Role::Named(name) => set_paint(paint, name.to_string(), cx),
                Role::Custom => {
                    let _ = this.update(cx, |this, cx| this.pick_custom(paint, cx));
                }
            },
            theme,
            window,
        )
        .into_any_element()
    }
}
