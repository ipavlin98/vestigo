use super::theme_editor::Unfold;
use super::{FINDER_CHORD, FINDER_KEY, Folds, Section, SettingsPage, column};
use crate::app_window::view::BOTTOM_BAR;
use crate::assets;
use crate::i18n::Language;
use crate::platform;
use crate::search::other_layout;
use crate::theme::{Theme, rem};
use crate::ui::{
    DETAIL_LINE, DETAIL_SIZE, Dismiss, GUTTER, Key, LABEL_GAP, NAME_LINE, NAME_SIZE, Tip, TipAnchor, icon_button,
    icon_tip, key_hint,
};
use gpui::{
    AnyElement, Context, Div, Focusable, IntoElement, Keystroke, MouseButton, Stateful, Window, div, prelude::*,
};
use std::ops::Range;

impl SettingsPage {
    pub(super) fn finder_bar(
        &self,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let colors = theme.field;
        let note = self.found.as_ref().map(|found| {
            div()
                .flex_none()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(colors.note)
                .child(found.note(language))
        });
        let band = TipAnchor::default();
        let trailing: Option<AnyElement> = if !self.finder.read(cx).text().is_empty() {
            let tip = Tip::keys(&Dismiss, window, cx).map(|tip| tip.along(band.clone()));
            let clear = icon_tip(icon_button("finder-clear", assets::CLOSE, theme), tip).on_click(cx.listener(
                |this, _, window, cx| {
                    this.finder.update(cx, |input, cx| input.set_text("", cx));
                    window.focus(&this.finder.focus_handle(cx), cx);
                },
            ));
            Some(clear.into_any_element())
        } else if self.finder.focus_handle(cx).is_focused(window) {
            None
        } else {
            Some(key_hint(Key::Text(FINDER_KEY), theme, window).into_any_element())
        };
        div()
            .id("finder")
            .relative()
            .h(rem(BOTTOM_BAR))
            .flex_none()
            .flex()
            .justify_center()
            .px(rem(GUTTER))
            .child(band.track())
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.finder.focus_handle(cx), cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                column()
                    .h_full()
                    .flex_row()
                    .items_center()
                    .gap(rem(LABEL_GAP))
                    .text_size(rem(NAME_SIZE))
                    .line_height(rem(NAME_LINE))
                    .child(div().flex_1().min_w_0().child(self.finder.clone()))
                    .children(note)
                    .children(trailing),
            )
    }

    pub(in crate::app_window) fn focus_finder_on(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = keystroke.modifiers;
        let Some((key, shifted)) = platform::layout_key(&keystroke.key) else { return false };
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform && !shifted && !modifiers.shift;
        let chord = modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;
        let wanted = (plain && key == FINDER_KEY && self.focus.is_focused(window)) || (chord && key == FINDER_CHORD);
        if wanted {
            window.focus(&self.finder.focus_handle(cx), cx);
            self.finder.update(cx, |input, cx| input.select_everything(cx));
        }
        wanted
    }

    pub(in crate::app_window) fn find(&mut self, window: &Window, cx: &mut Context<Self>) {
        let jump = self.found.as_ref().map_or(0, |found| found.jump);
        let text = self.finder.read(cx).text().to_owned();
        self.found =
            self.probe(&text, window, cx).map(|(needle, spots)| Finding { needle, spots, current: None, jump });
        self.land(self.section, cx);
        cx.notify();
    }

    pub(in crate::app_window) fn refind(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(found) = self.found.take() else { return };
        let text = self.finder.read(cx).text().to_owned();
        let kept = found.current_spot().map(|spot| spot.section);
        self.found = self.probe(&text, window, cx).map(|(needle, spots)| {
            let current =
                found.current.filter(|&current| kept.is_some() && spots.get(current).map(|spot| spot.section) == kept);
            Finding { needle, spots, current, jump: found.jump }
        });
        cx.notify();
    }

    fn probe(&self, text: &str, window: &Window, cx: &mut Context<Self>) -> Option<(Needle, Vec<Spot>)> {
        let needle = Needle::new(text)?;
        let spots = self.spots(&needle, window, cx);
        if !spots.is_empty() {
            return Some((needle, spots));
        }
        let swapped = other_layout(text).and_then(|other| Needle::new(&other)).map(|other| {
            let spots = self.spots(&other, window, cx);
            (other, spots)
        });
        Some(swapped.filter(|(_, spots)| !spots.is_empty()).unwrap_or((needle, spots)))
    }

    fn spots(&self, needle: &Needle, window: &Window, cx: &Context<Self>) -> Vec<Spot> {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        let mut spots = Vec::new();
        for section in Section::ALL {
            if needle.finds(section.label(language.strings()), &[] as &[&str]) {
                spots.push(Spot { section, needs: Vec::new() });
            }
            for block in self.blocks(section, Folds::All, theme, language, window, cx) {
                block.spots(section, needle, &mut spots);
            }
        }
        spots
    }

    pub(in crate::app_window) fn land(&mut self, section: Section, cx: &mut Context<Self>) {
        let Some(found) = self.found.as_mut() else { return };
        found.land(section);
        let needs = found.current_spot().map(|spot| spot.needs.clone()).unwrap_or_default();
        self.theme_editor.update(cx, |editor, cx| editor.unfold_all(&needs, cx));
    }

    pub(in crate::app_window) fn step_match(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let section = self.section;
        let Some(found) = self.found.as_mut() else { return };
        let Some(Spot { section: next, needs }) = found.step(step, section).cloned() else { return };
        if next != section {
            let finder = self.finder.focus_handle(cx);
            let typing = finder.is_focused(window);
            self.open(next, window, cx);
            if typing {
                window.focus(&finder, cx);
            }
        }
        self.theme_editor.update(cx, |editor, cx| editor.unfold_all(&needs, cx));
        cx.notify();
    }
}

pub(in crate::app_window) struct Needle {
    words: Vec<String>,
}

impl Needle {
    pub(in crate::app_window) fn new(text: &str) -> Option<Self> {
        let words: Vec<String> = text.split_whitespace().map(|word| folded(word).0).collect();
        (!words.is_empty()).then_some(Self { words })
    }

    pub(super) fn finds(&self, label: &str, hints: &[impl AsRef<str>]) -> bool {
        let texts: Vec<String> =
            std::iter::once(label).chain(hints.iter().map(AsRef::as_ref)).map(|text| folded(text).0).collect();
        self.words.iter().all(|word| texts.iter().any(|text| text.contains(word.as_str())))
    }

    pub(super) fn spans(&self, text: &str) -> Vec<Range<usize>> {
        let (lower, origins) = folded(text);
        let mut ranges: Vec<Range<usize>> = self
            .words
            .iter()
            .flat_map(|word| lower.match_indices(word.as_str()).map(|(at, word)| at..at + word.len()))
            .filter_map(|span| {
                let (from, _) = *origins.get(span.start)?;
                let (_, to) = *origins.get(span.end.checked_sub(1)?)?;
                Some(from..to)
            })
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                Some(_) | None => merged.push(range),
            }
        }
        merged
    }
}

fn folded(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut lower = String::with_capacity(text.len());
    let mut origins = Vec::with_capacity(text.len());
    for (offset, ch) in text.char_indices() {
        for lowered in ch.to_lowercase() {
            origins.extend(std::iter::repeat_n((offset, offset + ch.len_utf8()), lowered.len_utf8()));
            lower.push(lowered);
        }
    }
    (lower, origins)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Spot {
    pub(super) section: Section,
    pub(super) needs: Vec<Unfold>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app_window) enum Step {
    Next,
    Previous,
}

pub(in crate::app_window) struct Finding {
    pub(super) needle: Needle,
    pub(super) spots: Vec<Spot>,
    pub(super) current: Option<usize>,
    pub(super) jump: usize,
}

impl Finding {
    pub(in crate::app_window) fn count(&self, section: Section) -> usize {
        self.spots.iter().filter(|spot| spot.section == section).count()
    }

    pub(super) fn current_spot(&self) -> Option<&Spot> {
        self.spots.get(self.current?)
    }

    pub(super) fn land(&mut self, section: Section) {
        if self.current_spot().is_some_and(|spot| spot.section == section) {
            return;
        }
        self.current = self.spots.iter().position(|spot| spot.section == section);
        self.jump += 1;
    }

    pub(super) fn step(&mut self, step: Step, section: Section) -> Option<&Spot> {
        let count = self.spots.len();
        let next = match (self.current, step) {
            (Some(current), Step::Next) => (current + 1) % count.max(1),
            (Some(current), Step::Previous) => (current + count - 1) % count.max(1),
            (None, Step::Next) => self.spots.iter().position(|spot| spot.section >= section).unwrap_or(0),
            (None, Step::Previous) => {
                self.spots.iter().rposition(|spot| spot.section < section).unwrap_or(count.saturating_sub(1))
            }
        };
        self.spots.get(next)?;
        self.current = Some(next);
        self.jump += 1;
        self.spots.get(next)
    }

    pub(super) fn on_screen(&self, section: Section, shown: impl Fn(&[Unfold]) -> bool) -> Option<usize> {
        let current = self.current?;
        let spot = self.spots.get(current)?;
        if spot.section != section || !shown(&spot.needs) {
            return None;
        }
        let before = self.spots.get(..current)?;
        Some(before.iter().filter(|spot| spot.section == section && shown(&spot.needs)).count())
    }

    pub(super) fn note(&self, language: Language) -> String {
        match self.current {
            _ if self.spots.is_empty() => language.strings().nothing_found.to_owned(),
            Some(current) => language.nth_of(current + 1, self.spots.len()),
            None => language.matches(self.spots.len()),
        }
    }
}

#[cfg(test)]
mod tests;
