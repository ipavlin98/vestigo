mod load;
mod queue;
mod skeleton;

use super::glide;
use super::snippet_text;
use super::tokens::*;
use super::widgets::{Edge, divider, edge_fades, glyph};
use crate::assets;
use crate::i18n::Language;
use crate::index::{Hit, Snippet};
use crate::platform;
use crate::settings::Settings;
use crate::theme::{Theme, rem};
use gpui::{AnyElement, Context, Div, Hsla, IntoElement, ObjectFit, ScrollHandle, div, img, prelude::*};
use load::{Early, Facts, ICON, Loaded, Visual, kind};
use queue::{Job, Kept, Key, Need, Queue};

pub(crate) const PREVIEW_WIDTH: f32 = 320.0;
const FACT_LABEL: f32 = 80.0;

enum Stage {
    Loading,
    Arrived(Loaded),
    Ready(Loaded),
}

impl Stage {
    fn settled(self) -> Self {
        match self {
            Self::Loading => Self::Loading,
            Self::Arrived(loaded) | Self::Ready(loaded) => Self::Ready(loaded),
        }
    }

    fn arrive(self, loaded: Loaded) -> Self {
        match self {
            Self::Loading | Self::Arrived(_) => Self::Arrived(loaded),
            Self::Ready(_) => Self::Ready(loaded),
        }
    }
}

struct Shown {
    hit: Hit,
    pixels: u32,
    stage: Stage,
    scroll: ScrollHandle,
    serial: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct Surface {
    pub(crate) under: Hsla,
    pub(crate) line: Hsla,
}

#[derive(Default)]
pub(crate) struct Preview {
    shown: Option<Shown>,
    kept: Kept,
    queue: Queue,
    serial: usize,
}

impl Preview {
    pub(crate) fn wanted(&self) -> Option<&Hit> {
        self.shown.as_ref().map(|shown| &shown.hit)
    }

    pub(crate) fn show<T: 'static>(
        &mut self,
        hit: Option<&Hit>,
        around: Vec<Hit>,
        ratio: f32,
        preview: fn(&mut T) -> &mut Preview,
        cx: &mut Context<T>,
    ) {
        let Some(hit) = hit else {
            self.shown = None;
            self.queue.want(Vec::new());
            return;
        };
        let pixels = (ICON * ratio).round() as u32;
        if let Some(shown) = self.shown.as_mut().filter(|shown| shown.hit.path == hit.path && shown.pixels == pixels) {
            if shown.hit.snippet != hit.snippet {
                shown.hit.snippet.clone_from(&hit.snippet);
                cx.notify();
            }
            return;
        }
        let key = Key::of(hit, pixels);
        let stage = match (self.kept.get(&key), self.shown.take()) {
            (Some(loaded), _) => Stage::Ready(loaded.clone()),
            (None, Some(shown)) if shown.hit.path == hit.path => shown.stage.settled(),
            (None, _) => Stage::Loading,
        };
        self.serial += 1;
        self.shown = Some(Shown { hit: hit.clone(), pixels, stage, scroll: ScrollHandle::new(), serial: self.serial });
        let jobs = std::iter::once((hit.clone(), Need::Shown))
            .chain(around.into_iter().map(|other| (other, Need::Ahead)))
            .map(|(hit, need)| Job { key: Key::of(&hit, pixels), hit, ratio, need })
            .filter(|job| self.kept.get(&job.key).is_none())
            .collect();
        self.queue.want(jobs);
        self.pump(preview, cx);
        cx.notify();
    }

    fn pump<T: 'static>(&mut self, preview: fn(&mut T) -> &mut Preview, cx: &mut Context<T>) {
        let replace_small = cx.global::<Settings>().replace_small_icons;
        while let Some(Job { key, hit, ratio, .. }) = self.queue.next() {
            cx.spawn(async move |this, cx| {
                let target = hit.clone();
                let loaded = match cx.background_spawn(async move { load::early(&target, ratio) }).await {
                    Early::Done(loaded) => loaded,
                    Early::Glimpse(glimpse) => {
                        let _ = this.update(cx, |this, cx| preview(this).glimpse(&key, glimpse, cx));
                        cx.background_spawn(async move { load::load(&hit, ratio, replace_small) }).await
                    }
                    Early::Nothing => cx.background_spawn(async move { load::load(&hit, ratio, replace_small) }).await,
                };
                let _ = this.update(cx, |this, cx| preview(this).arrive(key, loaded, preview, cx));
            })
            .detach();
        }
    }

    fn arrive<T: 'static>(
        &mut self,
        key: Key,
        loaded: Loaded,
        preview: fn(&mut T) -> &mut Preview,
        cx: &mut Context<T>,
    ) {
        self.queue.done(&key);
        self.glimpse(&key, loaded.clone(), cx);
        self.kept.keep(key, loaded);
        self.pump(preview, cx);
    }

    fn glimpse<T: 'static>(&mut self, key: &Key, loaded: Loaded, cx: &mut Context<T>) {
        let Some(shown) = self.shown.as_mut().filter(|shown| shown.hit.path == key.path && shown.pixels == key.pixels)
        else {
            return;
        };
        let stage = std::mem::replace(&mut shown.stage, Stage::Loading);
        shown.stage = stage.arrive(loaded);
        cx.notify();
    }

    pub(crate) fn render(&self, theme: &Theme, language: Language, surface: Surface, plain: bool) -> AnyElement {
        let Some(shown) = self.shown.as_ref() else {
            let empty = div().size_full().flex().items_center().justify_center().text_color(theme.results.empty);
            return empty.child(language.strings().preview_empty).into_any_element();
        };
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .gap(rem(GUTTER))
            .p(rem(GUTTER))
            .child(visual(shown, theme, surface))
            .child(identity(&shown.hit, plain, theme))
            .children(shown.hit.snippet.as_ref().map(|snippet| excerpt(snippet, theme)))
            .child(facts(shown, theme, language, surface))
            .into_any_element()
    }
}

fn visual(shown: &Shown, theme: &Theme, surface: Surface) -> AnyElement {
    let area = div().relative().flex_1().min_h_0().flex().items_center().justify_center();
    match &shown.stage {
        Stage::Loading => area.child(skeleton::visual(kind(&shown.hit), theme)).into_any_element(),
        Stage::Arrived(loaded) => {
            skeleton::arriving(content(area, shown, &loaded.visual, theme, surface), ("preview-arrival", shown.serial))
        }
        Stage::Ready(loaded) => content(area, shown, &loaded.visual, theme, surface).into_any_element(),
    }
}

fn content(area: Div, shown: &Shown, visual: &Visual, theme: &Theme, surface: Surface) -> Div {
    match visual {
        Visual::Picture(picture) => area.child(img(picture.clone()).size_full().object_fit(ObjectFit::Contain)),
        Visual::Text(text) => area
            .items_start()
            .child(glide("preview", shown.scroll.clone(), None))
            .child(
                div()
                    .id("preview-text")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&shown.scroll)
                    .text_size(rem(DETAIL_SIZE))
                    .line_height(rem(DETAIL_LINE))
                    .text_color(theme.results.name)
                    .child(text.clone()),
            )
            .child(edge_fades(shown.scroll.clone(), surface.under, 0.0)),
        Visual::Icon(Some(icon)) => area.child(img(icon.clone()).size(rem(ICON)).flex_none()),
        Visual::Icon(None) => {
            let path = if shown.hit.dir { assets::FOLDER } else { assets::FILE };
            area.child(glyph(path, ICON, theme.results.icon))
        }
    }
}

fn identity(hit: &Hit, plain: bool, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(rem(FILL_GAP))
        .child(
            div()
                .text_size(rem(TITLE_SIZE))
                .line_height(rem(TITLE_LINE))
                .text_color(theme.results.name)
                .child(hit.label(plain).to_owned()),
        )
        .child(
            div()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(theme.results.path)
                .child(hit.path.clone()),
        )
}

fn excerpt(snippet: &Snippet, theme: &Theme) -> Div {
    div()
        .flex_none()
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .text_color(theme.results.name)
        .child(snippet_text(snippet, theme))
}

#[derive(Clone, Copy)]
enum Fact {
    Size,
    Created,
    Modified,
}

impl Fact {
    fn expected(hit: &Hit) -> &'static [Fact] {
        match (platform::is_store_app(&hit.path), hit.dir) {
            (true, _) => &[],
            (false, true) => &[Fact::Created, Fact::Modified],
            (false, false) => &[Fact::Size, Fact::Created, Fact::Modified],
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Size => "preview-size",
            Self::Created => "preview-created",
            Self::Modified => "preview-modified",
        }
    }

    fn label(self, language: Language) -> &'static str {
        let strings = language.strings();
        match self {
            Self::Size => strings.preview_size,
            Self::Created => strings.preview_created,
            Self::Modified => strings.preview_modified,
        }
    }

    fn value(self, facts: Facts, language: Language) -> Option<String> {
        match self {
            Self::Size => facts.size.map(|size| language.size(size)),
            Self::Created => facts.created.map(|stamp| language.date(stamp)),
            Self::Modified => facts.modified.map(|stamp| language.date(stamp)),
        }
    }

    fn placeholder(self) -> f32 {
        match self {
            Self::Size => skeleton::SIZE_BAR,
            Self::Created | Self::Modified => skeleton::DATE_BAR,
        }
    }
}

fn facts(shown: &Shown, theme: &Theme, language: Language, surface: Surface) -> Div {
    let value = |row: usize, fact: Fact| -> AnyElement {
        let text = |facts: Facts| div().child(fact.value(facts, language).unwrap_or_default());
        match &shown.stage {
            Stage::Loading => skeleton::value(fact.placeholder(), row, theme),
            Stage::Arrived(loaded) => skeleton::arriving(text(loaded.facts), (fact.id(), shown.serial)),
            Stage::Ready(loaded) => text(loaded.facts).into_any_element(),
        }
    };
    div()
        .relative()
        .flex_none()
        .flex()
        .flex_col()
        .gap(rem(FILL_GAP))
        .pt(rem(GUTTER))
        .text_size(rem(DETAIL_SIZE))
        .line_height(rem(DETAIL_LINE))
        .child(divider(Edge::Top, surface.line))
        .children(Fact::expected(&shown.hit).iter().enumerate().map(|(row, &fact)| {
            div()
                .flex()
                .child(div().w(rem(FACT_LABEL)).flex_none().text_color(theme.results.path).child(fact.label(language)))
                .child(div().flex_1().min_w_0().text_color(theme.results.name).child(value(row, fact)))
        }))
}

#[cfg(test)]
mod tests;
