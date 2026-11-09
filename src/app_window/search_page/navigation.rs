use super::SearchPage;
use crate::ui::{Browse, Browser};
use gpui::Context;

impl Browser for SearchPage {
    fn browse(&mut self) -> Option<Browse<'_>> {
        Some(Browse { input: &self.input, results: &mut self.results, trail: &mut self.trail })
    }

    fn selection_moved(&mut self, cx: &mut Context<Self>) {
        self.sync_preview(cx);
    }
}
