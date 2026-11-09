use super::Launcher;
use crate::ui::{Browse, Browser};
use gpui::Context;

impl Browser for Launcher {
    fn browse(&mut self) -> Option<Browse<'_>> {
        let session = self.session.as_mut()?;
        Some(Browse { input: &self.input, results: &mut self.results, trail: &mut session.trail })
    }

    fn selection_moved(&mut self, _: &mut Context<Self>) {}
}
