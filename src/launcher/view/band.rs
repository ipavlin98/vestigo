use super::{DOCKED, Growth, growth};
use crate::i18n::Language;
use crate::index::Status;
use crate::launcher::{Launcher, Mode};
use crate::theme::{Theme, rem};
use crate::ui::{DETAIL_LINE, DETAIL_SIZE, Edge, Entry, Tip, divider, faded, path_crumbs};
use gpui::{AnyElement, Bounds, Pixels, SharedString, div, prelude::*};

impl Launcher {
    pub(super) fn path_band(
        &self,
        mode: Mode,
        area: Bounds<Pixels>,
        theme: &Theme,
        language: Language,
        status: Status,
    ) -> AnyElement {
        let edge = match growth(mode) {
            Growth::Down => Edge::Top,
            Growth::Up => Edge::Bottom,
        };
        let line = match self.selected_folder() {
            Some(folder) => {
                let folder = SharedString::from(folder.to_owned());
                path_crumbs("crumbs", folder.clone(), Tip::new(folder).inside(area), DETAIL_LINE, theme.status)
            }
            None => {
                let status = self.results.status(status, language);
                faded("status", status.clone(), Tip::new(status).inside(area), DETAIL_LINE, theme.launcher.background)
            }
        };
        div()
            .relative()
            .h(rem(DOCKED.footer))
            .flex_none()
            .flex()
            .items_center()
            .px(rem(DOCKED.gutter()))
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(theme.status.text)
            .child(divider(edge, theme.launcher.divider))
            .child(line.flex_1().min_w_0())
            .into_any_element()
    }

    fn selected_folder(&self) -> Option<&str> {
        if self.results.notice().is_some() {
            return None;
        }
        match self.results.entry(self.results.selected_position())? {
            Entry::File(hit) if hit.dir => Some(hit.path.as_str()),
            Entry::File(hit) => Some(hit.folder()).filter(|folder| !folder.is_empty()),
            Entry::Extra(_) | Entry::Heading(_) => None,
        }
    }
}
