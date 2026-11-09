use super::{ACCENT, BACKGROUND, HUNDRED, Paint, Picking, SHADOW, TEXT, edit};
use crate::theme::{
    GROUPS, Sheet, Slot, ThemeEdits, accent, accent_chosen, color_slots, colors, hex, parse_hex, text_for,
};
use gpui::{App, Hsla};
use serde_json::Value;

pub(super) fn element_slot(group: usize, field: usize) -> Option<Slot> {
    let group = GROUPS.get(group)?;
    Some(Slot::Element(group.key, group.fields.get(field)?))
}

pub(super) fn paint_value(sheet: &Sheet, paint: Paint) -> Option<String> {
    match paint {
        Paint::Accent => Some(sheet.text(&Slot::Palette(ACCENT.into()))?.to_owned()),
        Paint::Background => Some(sheet.text(&Slot::Palette(BACKGROUND.into()))?.to_owned()),
        Paint::Text => Some(sheet.text(&Slot::Palette(TEXT.into()))?.to_owned()),
        Paint::Palette(index) => Some(sheet.text(&Slot::Palette(sheet.palette().get(index)?.clone()))?.to_owned()),
        Paint::Element(group, field) => Some(sheet.text(&element_slot(group, field)?)?.to_owned()),
        Paint::Layer(pane, index) => Some(sheet.shadow(pane).get(index)?.color.clone()),
    }
}

fn paint_alpha(sheet: &Sheet, paint: Paint) -> Option<f32> {
    match paint {
        Paint::Layer(pane, index) => Some(sheet.shadow(pane).get(index)?.opacity as f32),
        Paint::Accent | Paint::Background | Paint::Text | Paint::Palette(_) | Paint::Element(..) => None,
    }
}

pub(super) fn paint_color(sheet: &Sheet, paint: Paint) -> Option<Hsla> {
    let color = sheet.paint(&paint_value(sheet, paint)?)?;
    Some(paint_alpha(sheet, paint).map_or(color, |alpha| color.opacity(alpha)))
}

pub(super) fn translucent(sheet: &Sheet, paint: Paint) -> bool {
    match paint {
        Paint::Layer(..) => true,
        Paint::Palette(index) => sheet.palette().get(index).is_some_and(|name| name == SHADOW),
        Paint::Accent | Paint::Background | Paint::Text | Paint::Element(..) => false,
    }
}

pub(super) fn store(picking: &Picking, shade: bool, faded: bool, cx: &mut App) {
    match picking.paint {
        Paint::Layer(pane, index) => {
            if shade {
                set_paint(picking.paint, hex(picking.hsv.color()), cx);
            }
            if let Some(alpha) = picking.alpha.filter(|_| faded) {
                edit(cx, |sheet, edits| {
                    let mut layers = sheet.shadow(pane);
                    if let Some(layer) = layers.get_mut(index) {
                        layer.opacity = (f64::from(alpha) * HUNDRED).round() / HUNDRED;
                        edits.set(sheet, &Slot::Shadow(pane), serde_json::json!(layers));
                    }
                });
            }
        }
        Paint::Accent | Paint::Background | Paint::Text | Paint::Palette(_) | Paint::Element(..) => {
            set_paint(picking.paint, picking.field(), cx)
        }
    }
}

pub(super) fn set_paint(paint: Paint, value: String, cx: &mut App) {
    edit(cx, |sheet, edits| match paint {
        Paint::Accent => edits.apply(sheet, &accent(&value)),
        Paint::Background => {
            if let Some(background) = parse_hex(&value) {
                edits.apply(sheet, &colors(sheet, background, text_for(background)));
            }
        }
        Paint::Text => {
            if let (Some(background), Some(text)) = (sheet.color(&Slot::Palette(BACKGROUND.into())), parse_hex(&value))
            {
                edits.apply(sheet, &colors(sheet, background, text));
            }
        }
        Paint::Palette(index) => {
            if let Some(name) = sheet.palette().get(index) {
                edits.set(sheet, &Slot::Palette(name.clone()), Value::String(value));
            }
        }
        Paint::Element(group, field) => {
            if let Some(slot) = element_slot(group, field) {
                edits.set(sheet, &slot, Value::String(value));
            }
        }
        Paint::Layer(pane, index) => {
            let mut layers = sheet.shadow(pane);
            if let Some(layer) = layers.get_mut(index) {
                layer.color = value;
                edits.set(sheet, &Slot::Shadow(pane), serde_json::json!(layers));
            }
        }
    });
}

pub(super) fn follow_background(sheet: &Sheet, edits: &mut ThemeEdits) {
    match sheet.color(&Slot::Palette(BACKGROUND.into())) {
        Some(background) if sheet.edited(&Slot::Palette(BACKGROUND.into())) => {
            edits.apply(sheet, &colors(sheet, background, text_for(background)));
        }
        Some(_) | None => reset_colors(sheet, edits),
    }
}

pub(super) fn reset_colors(sheet: &Sheet, edits: &mut ThemeEdits) {
    edits.reset_all(&sheet.id, &color_slots(sheet));
    if !accent_chosen(sheet) {
        edits.reset(&sheet.id, &Slot::Palette(ACCENT.into()));
    }
}
