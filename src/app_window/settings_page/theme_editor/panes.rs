use super::super::{Folds, Row, field_row_with, setting_row};
use super::ThemeEditor;
use super::{Fold, Paint, SHADOW, Scale, Unfold, cluster, current_sheet, dial, edit, set, with_changes};
use crate::i18n::{Language, Strings};
use crate::theme::{CORNERS, Layer, Pane, Sheet, Slot, Theme, number, translucent};
use crate::ui::{button, disabled_toggle, toggle};
use gpui::{App, Context, ElementId, SharedString, Window, prelude::*};
use serde_json::{Map, Value};

const MAX_LAYERS: usize = 4;
const CORNER: Scale = Scale { start: 0.0, end: 24.0, step: 1.0, percent: false };
const OPACITY: Scale = Scale { start: 0.3, end: 1.0, step: 0.01, percent: true };
const OFFSET: Scale = Scale { start: -32.0, end: 32.0, step: 1.0, percent: false };
const BLUR: Scale = Scale { start: 0.0, end: 64.0, step: 1.0, percent: false };
const NEW_LAYER_OPACITY: f64 = 0.2;
const NEW_LAYER_Y: f64 = 4.0;
const NEW_LAYER_BLUR: f64 = 16.0;

#[derive(Clone, Copy, Debug)]
enum Part {
    X,
    Y,
    Blur,
    Spread,
}

impl Part {
    const ALL: [Self; 4] = [Self::X, Self::Y, Self::Blur, Self::Spread];

    fn label(self, strings: &Strings) -> &'static str {
        match self {
            Self::X => strings.offset_x,
            Self::Y => strings.offset_y,
            Self::Blur => strings.shadow_blur,
            Self::Spread => strings.shadow_spread,
        }
    }

    fn scale(self) -> Scale {
        match self {
            Self::X | Self::Y | Self::Spread => OFFSET,
            Self::Blur => BLUR,
        }
    }

    fn of(self, layer: &mut Layer) -> &mut f64 {
        match self {
            Self::X => &mut layer.x,
            Self::Y => &mut layer.y,
            Self::Blur => &mut layer.blur,
            Self::Spread => &mut layer.spread,
        }
    }
}

fn pane_text(pane: Pane, strings: &Strings) -> (&'static str, &'static str) {
    match pane {
        Pane::Launcher => (strings.pane_launcher, strings.pane_launcher_hint),
        Pane::Dialog => (strings.pane_dialog, strings.pane_dialog_hint),
        Pane::Explorer => (strings.pane_explorer, strings.pane_explorer_hint),
        Pane::App => (strings.pane_app, strings.pane_app_hint),
    }
}

fn corner_labels(strings: &Strings) -> [&'static str; 4] {
    [strings.corner_top_left, strings.corner_top_right, strings.corner_bottom_right, strings.corner_bottom_left]
}

fn corners_value(corners: [f64; 4]) -> Value {
    if corners.iter().all(|corner| *corner == corners[0]) {
        return number(corners[0]);
    }
    let each: Map<String, Value> =
        CORNERS.iter().zip(corners).map(|(key, radius)| ((*key).to_owned(), number(radius))).collect();
    Value::Object(each)
}

fn corners_of(sheet: &Sheet, pane: Pane) -> [f64; 4] {
    sheet.corners(pane).all().map(f64::from)
}

fn edit_layers(pane: Pane, cx: &mut App, change: impl FnOnce(&mut Vec<Layer>)) {
    edit(cx, |sheet, edits| {
        let mut layers = sheet.shadow(pane);
        change(&mut layers);
        edits.set(sheet, &Slot::Shadow(pane), serde_json::json!(layers));
    });
}

impl ThemeEditor {
    pub(super) fn pane_rows(
        &self,
        sheet: &Sheet,
        folds: Folds,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        Pane::ALL
            .into_iter()
            .map(|pane| {
                let fold = Fold::Pane(pane);
                let (label, hint) = pane_text(pane, strings);
                let hint = with_changes(hint, sheet.edits_under(&["windows", pane.key()]), language);
                let rows = if self.unfolded(Unfold::Fold(fold), folds) {
                    self.pane_fields(pane, sheet, theme, language, window, cx)
                } else {
                    Vec::new()
                };
                self.fold(fold, (label.into(), hint), rows, folds)
            })
            .collect()
    }

    fn pane_fields(
        &self,
        pane: Pane,
        sheet: &Sheet,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        let corners = corners_of(sheet, pane);
        let uniform = corners.iter().all(|corner| *corner == corners[0]);
        let apart = !uniform || self.split.contains(&pane);
        let slot = Slot::Corners(pane);
        let mut rows = Vec::new();
        if apart {
            for (position, label) in corner_labels(strings).into_iter().enumerate() {
                let value = corners.get(position).copied().unwrap_or_default();
                let write = move |value, cx: &mut App| {
                    let Some(sheet) = current_sheet(cx) else { return };
                    let mut corners = corners_of(&sheet, pane);
                    if let Some(corner) = corners.get_mut(position) {
                        *corner = value;
                    }
                    set(Slot::Corners(pane), corners_value(corners), cx);
                };
                let dial = dial(format!("corner-{pane:?}-{position}"), value, CORNER, write, theme);
                let reset = (position == 0).then(|| self.reset_icon(&slot, sheet, theme)).flatten();
                rows.push(field_row_with(label.into(), vec![cluster([reset, Some(dial)])]));
            }
        } else {
            let write = move |value, cx: &mut App| set(Slot::Corners(pane), number(value), cx);
            let dial = dial(format!("corners-{pane:?}"), corners[0], CORNER, write, theme);
            let controls = cluster([self.reset_icon(&slot, sheet, theme), Some(dial)]);
            rows.push(field_row_with(strings.corners.into(), vec![controls]));
        }
        let split = toggle(ElementId::Name(format!("apart-{pane:?}").into()), apart, theme)
            .on_click(self.listener(move |this, _, cx| this.split_corners(pane, cx)));
        rows.push(field_row_with(strings.corners_apart.into(), vec![split.into_any_element()]));

        let opacity = Slot::Opacity(pane);
        let write = move |value, cx: &mut App| set(Slot::Opacity(pane), number(value), cx);
        let dial = dial(format!("opacity-{pane:?}"), sheet.number(&opacity), OPACITY, write, theme);
        let controls = cluster([self.reset_icon(&opacity, sheet, theme), Some(dial)]);
        rows.push(field_row_with(strings.opacity.into(), vec![controls]));

        let blur = Slot::Blur(pane);
        let on = sheet.flag(&blur);
        let id = ElementId::Name(format!("blur-{pane:?}").into());
        let switch = if translucent(sheet, pane) {
            toggle(id, on, theme).on_click(move |_, _, cx| set(Slot::Blur(pane), Value::Bool(!on), cx))
        } else {
            disabled_toggle(id, theme)
        };
        let controls = cluster([self.reset_icon(&blur, sheet, theme), Some(switch.into_any_element())]);
        rows.push(field_row_with(strings.blur.into(), vec![controls]));

        rows.extend(self.shadow_rows(pane, sheet, theme, language, window, cx));
        rows
    }

    fn shadow_rows(
        &self,
        pane: Pane,
        sheet: &Sheet,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        let slot = Slot::Shadow(pane);
        let layers = sheet.shadow(pane);
        let add = button(
            ElementId::Name(format!("add-layer-{pane:?}").into()),
            strings.add,
            layers.len() < MAX_LAYERS,
            theme,
            move |_, _, cx| {
                edit_layers(pane, cx, |layers| {
                    if layers.len() < MAX_LAYERS {
                        layers.push(Layer {
                            color: SHADOW.to_owned(),
                            opacity: NEW_LAYER_OPACITY,
                            x: 0.0,
                            y: NEW_LAYER_Y,
                            blur: NEW_LAYER_BLUR,
                            spread: 0.0,
                        });
                    }
                })
            },
        );
        let controls = cluster([self.reset_icon(&slot, sheet, theme), Some(add.into_any_element())]);
        let mut rows = vec![setting_row(strings.shadow, vec![language.layers(layers.len()).into()], vec![controls])];
        for (index, layer) in layers.iter().enumerate() {
            rows.extend(self.layer_rows(pane, index, layer, sheet, theme, language, window, cx));
        }
        rows
    }

    #[expect(clippy::too_many_arguments, reason = "a layer row needs the layer, the sheet and the render context")]
    fn layer_rows(
        &self,
        pane: Pane,
        index: usize,
        layer: &Layer,
        sheet: &Sheet,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &App,
    ) -> Vec<Row> {
        let strings = language.strings();
        let mut rows = Vec::new();
        let paint = Paint::Layer(pane, index);
        let remove = button(
            ElementId::Name(format!("remove-{paint:?}").into()),
            strings.delete,
            true,
            theme,
            self.listener(move |this, _, cx| this.remove_layer(pane, index, cx)),
        );
        rows.push(field_row_with(SharedString::from(language.layer(index + 1)), vec![remove.into_any_element()]));
        let color = sheet.paint(&layer.color).unwrap_or_default().opacity(layer.opacity as f32);
        let controls = cluster([
            Some(self.color_control(paint, color, theme, window, cx)),
            Some(self.role_menu(paint, sheet, theme, strings, window)),
        ]);
        rows.push(
            field_row_with(strings.shadow_color.into(), vec![controls])
                .id(ElementId::Name(format!("{paint:?}").into())),
        );
        for part in Part::ALL {
            let value = *part.of(&mut layer.clone());
            let write = move |value, cx: &mut App| {
                edit_layers(pane, cx, |layers| {
                    if let Some(layer) = layers.get_mut(index) {
                        *part.of(layer) = value;
                    }
                })
            };
            let dial = dial(format!("{paint:?}-{part:?}"), value, part.scale(), write, theme);
            rows.push(field_row_with(part.label(strings).into(), vec![dial]));
        }
        rows
    }

    fn split_corners(&mut self, pane: Pane, cx: &mut Context<Self>) {
        let Some(sheet) = current_sheet(cx) else { return };
        let corners = corners_of(&sheet, pane);
        let uniform = corners.iter().all(|corner| *corner == corners[0]);
        if uniform && !self.split.contains(&pane) {
            self.split.push(pane);
        } else {
            self.split.retain(|split| *split != pane);
            if !uniform {
                let widest = corners.into_iter().fold(0.0, f64::max);
                set(Slot::Corners(pane), number(widest), cx);
            }
        }
        cx.notify();
    }

    fn remove_layer(&mut self, pane: Pane, index: usize, cx: &mut Context<Self>) {
        if matches!(self.picking.as_ref().map(|picking| picking.paint), Some(Paint::Layer(shown, _)) if shown == pane) {
            self.close_picker(cx);
        }
        edit_layers(pane, cx, |layers| {
            if index < layers.len() {
                layers.remove(index);
            }
        });
        cx.notify();
    }
}
