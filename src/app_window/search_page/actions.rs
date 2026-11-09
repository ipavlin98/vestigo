use super::SearchPage;
use crate::i18n::Language;
use crate::index::Hit;
use crate::settings::{BuiltinAction, History, Settings};
use crate::ui::{self, Back, Command, MenuEntry, MenuIcon, MenuItem, RunAction, ShowActions, Subject, Tool};
use gpui::{BorrowAppContext, Context, Window};

impl SearchPage {
    pub(super) fn run_action(&mut self, action: &RunAction, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.results.selected().cloned() {
            let targets = self.results.targets();
            self.act(action.index, hit, targets, cx);
        }
    }

    pub(in crate::app_window) fn run_hotkey_action(
        &mut self,
        index: usize,
        active: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hit) = self.results.selected().cloned().filter(|_| active) else { return false };
        let targets = self.results.targets();
        self.act(index, hit, targets, cx);
        true
    }

    pub(super) fn show_actions_selected(&mut self, _: &ShowActions, _: &mut Window, cx: &mut Context<Self>) {
        let targets = self.results.targets();
        self.open_actions(targets, cx);
    }

    pub(super) fn back(&mut self, _: &Back, _: &mut Window, _: &mut Context<Self>) {}

    pub(super) fn show_actions(&mut self, position: usize, cx: &mut Context<Self>) {
        let targets = self.results.targets_at(position);
        self.sync_preview(cx);
        cx.notify();
        self.open_actions(targets, cx);
    }

    fn open_actions(&mut self, targets: Vec<Hit>, cx: &mut Context<Self>) {
        let Some(hit) = self.results.selected().cloned() else { return };
        let settings = cx.global::<Settings>();
        let rows = ui::action_rows(&targets, settings, cx.global::<Language>().strings());
        let this = cx.entity().downgrade();
        let entries = rows
            .into_iter()
            .filter_map(|row| {
                let Tool::Action { index, builtin, .. } = row.tool else { return None };
                let item = MenuItem::new(row.title, MenuIcon::Glyph(row.tool.icon()));
                let shortcut = settings.actions.get(index).and_then(|action| action.shortcut.as_ref());
                let item = match shortcut {
                    Some(shortcut) => item.hint(shortcut.to_string()),
                    None => item,
                };
                let path = hit.path.clone();
                if builtin == Some(BuiltinAction::ContextMenu) {
                    return Some(MenuEntry::Item(
                        item.submenu(move |cx| ui::shell_entries(path.clone(), Vec::new(), cx)),
                    ));
                }
                let (this, hit, targets) = (this.clone(), hit.clone(), targets.clone());
                Some(MenuEntry::Item(item.on_choose(move |cx| {
                    let _ = this.update(cx, |this, cx| this.act(index, hit.clone(), targets.clone(), cx));
                })))
            })
            .collect();
        ui::open_menu(entries, cx);
    }

    fn act(&mut self, index: usize, hit: Hit, targets: Vec<Hit>, cx: &mut Context<Self>) {
        let Some(action) = cx.global::<Settings>().actions.get(index).cloned() else { return };
        let paths = ui::action_paths(&action, &hit, targets);
        let launched = match action.builtin {
            Some(BuiltinAction::Open) => Some(Command::Open(hit.path.clone())),
            Some(BuiltinAction::Reveal) => Some(Command::Reveal(hit.path.clone())),
            Some(BuiltinAction::RunAsAdmin | BuiltinAction::OpenWith) | None => None,
            Some(
                BuiltinAction::Copy
                | BuiltinAction::Cut
                | BuiltinAction::CopyPath
                | BuiltinAction::CopyName
                | BuiltinAction::Delete
                | BuiltinAction::Properties
                | BuiltinAction::ContextMenu,
            ) => {
                ui::perform_action(&action, Subject::of(paths), cx);
                return;
            }
        };
        cx.update_global::<History, _>(|history, _| history.record(&hit.path, hit.dir));
        match launched {
            Some(command) => ui::perform(command, cx),
            None => ui::perform_action(&action, Subject::of(paths), cx),
        }
    }
}
