use super::view::panel_height;
use super::{Launcher, Mode};
use crate::i18n::Language;
use crate::index::Hit;
use crate::platform::{self, CardLayout};
use crate::settings::Settings;
use crate::theme::{Theme, rem};
use crate::ui::{self, INSET, NAME_LINE, NAME_SIZE, PREVIEW_WIDTH, Preview, Surface, TogglePreview, window_shape};
use gpui::{App, BorrowAppContext, Context, IntoElement, Render, Subscription, Window, div, prelude::*};

pub(crate) struct Card {
    preview: Preview,
    ratio: f32,
    _subscriptions: Vec<Subscription>,
}

impl Card {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.observe_window_bounds(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Theme>(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global_in::<Settings>(window, |this, window, cx| this.rescale(window, cx)),
            cx.observe_global::<Language>(|_, cx| cx.notify()),
        ];
        ui::fit_rem(window, cx);
        Self { preview: Preview::default(), ratio: ui::pixel_ratio(window), _subscriptions: subscriptions }
    }

    fn show(&mut self, hit: Option<&Hit>, around: Vec<Hit>, cx: &mut Context<Self>) {
        self.preview.show(hit, around, self.ratio, |this: &mut Self| &mut this.preview, cx);
    }

    fn rescale(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        ui::fit_rem(window, cx);
        self.ratio = ui::pixel_ratio(window);
        let hit = self.preview.wanted().cloned();
        self.show(hit.as_ref(), Vec::new(), cx);
    }
}

impl Launcher {
    pub(super) fn toggle_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode() == Some(Mode::Launcher) {
            cx.update_global::<Settings, _>(|settings, _| settings.preview.launcher = !settings.preview.launcher);
            cx.notify();
        }
    }

    pub(super) fn sync_card(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.card.clone() else { return };
        let shown = self.mode() == Some(Mode::Launcher) && cx.global::<Settings>().preview.launcher;
        let kept = || card.read(cx).preview.wanted().cloned().filter(|_| !self.results.hits().is_empty());
        let hit = self.results.selected().cloned().or_else(kept).filter(|_| shown);
        let around = if hit.is_some() { self.results.around_selected() } else { Vec::new() };
        card.update(cx, |card, cx| card.show(hit.as_ref(), around, cx));
        let Some(session) = self.session.as_mut() else { return };
        let layout = hit.is_some().then(|| layout(cx));
        if session.card_layout != layout {
            session.card_layout = layout.clone();
            cx.spawn(async move |_, _| platform::place_card(layout)).detach();
        }
    }
}

fn layout(cx: &App) -> CardLayout {
    let chrome = cx.global::<Theme>().windows.launcher.clone();
    CardLayout { width: PREVIEW_WIDTH, height: panel_height(), gap: INSET, zoom: ui::zoom(cx), chrome }
}

impl Render for Card {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        let surface = Surface { under: theme.launcher.background, line: theme.launcher.divider };
        window_shape(div(), theme.windows.launcher.corners)
            .size_full()
            .overflow_hidden()
            .bg(theme.launcher.background)
            .font_family(theme.font.clone())
            .text_size(rem(NAME_SIZE))
            .line_height(rem(NAME_LINE))
            .text_color(theme.results.name)
            .child(self.preview.render(theme, language, surface, cx.global::<Settings>().hide_shortcut_extensions))
    }
}
