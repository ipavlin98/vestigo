mod menu;

use menu::shell_rows;
pub(super) use menu::{Menu, ShellRow, find};

use super::{Launcher, Mode};
use crate::i18n::Language;
use crate::index::Hit;
use crate::platform;
use crate::settings::{BuiltinAction, History, Settings};
use crate::theme::Theme;
use crate::ui::{self, Back, Command, Entry, Extra, Listing, RunAction, ShowActions, Subject, Tool};
use gpui::{AppContext, BorrowAppContext, Context, Window};

impl Launcher {
    pub(super) fn show_actions_selected(&mut self, _: &ShowActions, _: &mut Window, cx: &mut Context<Self>) {
        let position = self.results.selected_position();
        match self.results.entry(position) {
            Some(Entry::Extra(Extra { tool: Tool::Shell { index, submenu: true }, .. })) => {
                let index = *index;
                self.open_submenu(index, cx);
            }
            Some(Entry::File(_)) => {
                let targets = self.results.targets();
                self.show_actions(position, targets, cx);
            }
            Some(Entry::Extra(_) | Entry::Heading(_)) | None => {}
        }
    }

    pub(super) fn prepare_menu(&mut self) {
        let (Some(hit), Some(session)) = (self.results.selected(), self.session.as_mut()) else { return };
        if session.prepared.as_ref() != Some(&hit.path) {
            session.prepared = Some(hit.path.clone());
            platform::prepare_shell_menu(hit.path.clone());
        }
    }

    pub(super) fn show_actions(&mut self, position: usize, targets: Vec<Hit>, cx: &mut Context<Self>) {
        let Some(hit) = self.results.hit(position).cloned() else { return };
        self.forget_prepared();
        if targets.len() > 1 {
            self.enter_menu(hit, targets, Vec::new(), cx);
            return;
        }
        let items = platform::load_shell_menu(hit.path.clone(), Vec::new());
        let background = cx.global::<Theme>().launcher.background.l;
        self.filling = Some(cx.spawn(async move |this, cx| {
            let shell =
                cx.background_spawn(async move { shell_rows(items.await.unwrap_or_default(), background) }).await;
            let _ = this.update(cx, |this, cx| this.enter_menu(hit, targets, shell, cx));
        }));
    }

    fn enter_menu(&mut self, hit: Hit, targets: Vec<Hit>, shell: Vec<ShellRow>, cx: &mut Context<Self>) {
        let builtin = ui::action_rows(&targets, cx.global::<Settings>(), cx.global::<Language>().strings());
        if builtin.is_empty() && shell.is_empty() {
            return;
        }
        let query = self.input.read(cx).text().to_owned();
        let shown = self.results.shown();
        self.menu = Some(Menu::new(hit, targets, query, shown, shell));
        self.apply_placeholder(cx);
        self.present_actions(cx);
        self.input.update(cx, |input, cx| input.set_text("", cx));
    }

    fn open_submenu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = &self.menu else { return };
        let Some(row) = menu.row(index).filter(|row| row.submenu) else { return };
        let title = row.title.clone();
        let items = platform::load_shell_menu(menu.hit().path.clone(), row.trail.clone());
        let background = cx.global::<Theme>().launcher.background.l;
        self.filling = Some(cx.spawn(async move |this, cx| {
            let rows =
                cx.background_spawn(async move { shell_rows(items.await.unwrap_or_default(), background) }).await;
            let _ = this.update(cx, |this, cx| this.enter_level(title, rows, cx));
        }));
    }

    fn enter_level(&mut self, title: String, rows: Vec<ShellRow>, cx: &mut Context<Self>) {
        let shown = self.results.shown();
        let Some(menu) = self.menu.as_mut().filter(|_| !rows.is_empty()) else { return };
        menu.descend(title, rows, shown);
        self.present_actions(cx);
        self.input.update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn filter_actions(&mut self, cx: &mut Context<Self>) -> bool {
        let typed = self.input.read(cx).text().to_owned();
        let Some(menu) = self.menu.as_mut() else { return false };
        if menu.filter(typed) {
            self.present_actions(cx);
        }
        true
    }

    fn present_actions(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = &self.menu else { return };
        let strings = cx.global::<Language>().strings();
        let builtin =
            if menu.at_top() { ui::action_rows(menu.targets(), cx.global::<Settings>(), strings) } else { Vec::new() };
        let (rows, heading) = menu.listed(builtin);
        let shape = self.shape(cx);
        self.filling = None;
        self.results.present_with(rows, Vec::new(), Vec::new(), Listing::Actions);
        if let Some(at) = heading {
            self.results.head(at, strings.context_menu.to_owned());
        }
        self.reshape(shape, cx);
    }

    pub(super) fn shell_row(&self, index: usize) -> Option<&ShellRow> {
        self.menu.as_ref()?.row(index)
    }

    pub(super) fn back(&mut self, _: &Back, _: &mut Window, cx: &mut Context<Self>) {
        self.step_back(cx);
    }

    pub(super) fn step_back(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((shown, typed)) = self.menu.as_mut().and_then(Menu::ascend) else { return self.leave_menu(cx) };
        let shape = self.shape(cx);
        self.results.show_again(shown);
        self.reshape(shape, cx);
        if self.input.read(cx).text() != typed {
            self.input.update(cx, |input, cx| input.set_text(&typed, cx));
        }
        true
    }

    fn leave_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let shape = self.shape(cx);
        let left = self.restore_menu(cx);
        if left {
            self.reshape(shape, cx);
        }
        left
    }

    pub(super) fn restore_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.menu.take() else { return false };
        let (query, shown) = menu.left();
        self.results.show_again(shown);
        self.searched.clone_from(&query);
        self.apply_placeholder(cx);
        if self.input.read(cx).text() != query {
            self.input.update(cx, |input, cx| input.set_text(&query, cx));
        }
        true
    }
    pub(super) fn clear_query(&mut self, cx: &mut Context<Self>) {
        self.leave_menu(cx);
        self.input.update(cx, |input, cx| input.set_text("", cx));
    }

    pub(super) fn run_action(&mut self, action: &RunAction, _: &mut Window, cx: &mut Context<Self>) {
        let (hit, targets) = match self.results.selected_entry() {
            Some(Entry::File(hit)) => (hit.clone(), self.results.targets()),
            Some(Entry::Extra(Extra { tool: Tool::Action { dir, .. }, argument, .. })) => {
                let hit = Hit::new(argument.clone(), *dir);
                (hit.clone(), self.menu_targets(hit))
            }
            Some(Entry::Extra(_) | Entry::Heading(_)) | None => return,
        };
        self.act(action.index, hit, targets, cx);
    }

    pub(crate) fn run_hotkey_action(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        let Some(hit) = self.results.selected().cloned().filter(|_| self.active) else { return false };
        let targets = self.results.targets();
        self.act(index, hit, targets, cx);
        true
    }

    fn menu_targets(&self, hit: Hit) -> Vec<Hit> {
        self.menu.as_ref().map_or_else(|| vec![hit], |menu| menu.targets().to_vec())
    }

    fn act(&mut self, index: usize, hit: Hit, targets: Vec<Hit>, cx: &mut Context<Self>) {
        let Some(action) = cx.global::<Settings>().actions.get(index).cloned() else { return };
        let paths = ui::action_paths(&action, &hit, targets);
        match action.builtin {
            Some(BuiltinAction::Open) => return self.launch(hit, false, cx),
            Some(BuiltinAction::Reveal) => return self.launch(hit, true, cx),
            Some(BuiltinAction::RunAsAdmin | BuiltinAction::OpenWith) | None => {
                cx.update_global::<History, _>(|history, _| history.record(&hit.path, hit.dir));
            }
            Some(
                BuiltinAction::Copy
                | BuiltinAction::Cut
                | BuiltinAction::CopyPath
                | BuiltinAction::CopyName
                | BuiltinAction::Delete
                | BuiltinAction::Properties
                | BuiltinAction::ContextMenu,
            ) => {}
        }
        self.leave(cx);
        ui::perform_action(&action, Subject::of(paths), cx);
    }

    pub(super) fn run_tool(&mut self, extra: Extra, cx: &mut Context<Self>) {
        let Extra { tool, argument, .. } = extra;
        match tool {
            Tool::Action { index, dir, .. } => {
                let hit = Hit::new(argument, dir);
                let targets = self.menu_targets(hit.clone());
                self.act(index, hit, targets, cx);
            }
            Tool::Shell { index, submenu: true } => self.open_submenu(index, cx),
            Tool::Shell { index, submenu: false } => {
                let Some(trail) = self.shell_row(index).map(|row| row.trail.clone()) else { return };
                self.leave(cx);
                cx.spawn(async move |_, _| platform::run_shell_item(argument, trail)).detach();
            }
            Tool::Web(index) => {
                let Some(web) = cx.global::<Settings>().web.get(index) else { return };
                let link = web.link(&argument);
                self.leave(cx);
                ui::perform(Command::Open(link), cx);
            }
            Tool::Command(index) => {
                let Some(command) = cx.global::<Settings>().commands.get(index).cloned() else { return };
                let host = self.host();
                self.leave(cx);
                cx.spawn(async move |this, cx| {
                    let folder = cx.background_spawn(async move { platform::current_folder(host) }).await;
                    let subject = Subject { folder, query: argument, paths: Vec::new() };
                    let _ = this.update(cx, |_, cx| ui::perform_command(&command, subject, host, cx));
                })
                .detach();
            }
        }
    }

    fn forget_prepared(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.prepared = None;
        }
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.forget_prepared();
        match self.mode() {
            Some(Mode::Dialog(..)) => self.clear_query(cx),
            Some(Mode::Launcher | Mode::Explorer(_)) | None => self.close(cx),
        }
    }
}
