use super::{Launcher, Mode, ShowFavorites, ShowRecent, view};
use crate::i18n::Language;
use crate::platform::{self, MenuAt};
use crate::settings::History;
use crate::theme::Theme;
use crate::ui::{self, Command, Go, MenuEntry, MenuIcon, MenuItem};
use gpui::{AppContext, BorrowAppContext, Context, Window};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderMenu {
    Favorites,
    Recent,
}

impl Launcher {
    pub(super) fn folder_menu_at(&self, menu: FolderMenu) -> Option<MenuAt> {
        match self.mode()? {
            mode @ Mode::Dialog(..) => Some(view::folder_menu_at(mode, menu)),
            Mode::Launcher | Mode::Explorer(_) => None,
        }
    }

    pub(super) fn press_folder_menu(&mut self, menu: FolderMenu, cx: &mut Context<Self>) {
        let Some(at) = self.folder_menu_at(menu) else { return };
        if ui::menu_shown_at(at, cx) {
            ui::close_search_menu(cx);
        } else {
            self.open_folder_menu(menu, cx);
        }
    }

    pub(super) fn show_favorites(&mut self, _: &ShowFavorites, _: &mut Window, cx: &mut Context<Self>) {
        self.open_folder_menu(FolderMenu::Favorites, cx);
    }

    pub(super) fn show_recent(&mut self, _: &ShowRecent, _: &mut Window, cx: &mut Context<Self>) {
        self.open_folder_menu(FolderMenu::Recent, cx);
    }

    fn open_folder_menu(&mut self, menu: FolderMenu, cx: &mut Context<Self>) {
        if self.folder_menu_at(menu).is_none() {
            return;
        }
        match menu {
            FolderMenu::Favorites => {
                let host = self.host();
                cx.spawn(async move |this, cx| {
                    let current = cx.background_spawn(async move { host.and_then(platform::host_folder) }).await;
                    let _ = this.update(cx, |this, cx| this.show_favorites_menu(current, cx));
                })
                .detach();
            }
            FolderMenu::Recent => {
                let visits = cx.global::<History>().visits();
                cx.spawn(async move |this, cx| {
                    let (open, recent) = cx
                        .background_spawn(async move { (platform::explorer_folders(), ui::recent_folders(visits)) })
                        .await;
                    let _ = this.update(cx, |this, cx| this.show_recent_menu(open, recent, cx));
                })
                .detach();
            }
        }
    }

    fn show_favorites_menu(&mut self, current: Option<String>, cx: &mut Context<Self>) {
        let Some(at) = self.folder_menu_at(FolderMenu::Favorites) else { return };
        let this = cx.entity().downgrade();
        let reopened = current.clone();
        let reopen: ui::Choose = Rc::new(move |cx| {
            let _ = this.update(cx, |this, cx| this.show_favorites_menu(reopened.clone(), cx));
        });
        let mut entries = ui::favorite_entries(current.as_deref(), reopen, &self.go(cx), cx);
        if let [MenuEntry::Heading { button, .. }] = entries.as_slice() {
            let strings = cx.global::<Language>().strings();
            let hint = match button {
                Some(_) => MenuEntry::Item(MenuItem::new(strings.menu_no_favorites, MenuIcon::Blank)),
                None => ui::nothing(strings),
            };
            entries.push(hint);
        }
        let chrome = view::drop_chrome(self.placed, cx.global::<Theme>());
        ui::open_menu_at(entries, at, chrome, cx);
    }

    fn show_recent_menu(&mut self, open: Vec<String>, recent: Vec<String>, cx: &mut Context<Self>) {
        let Some(at) = self.folder_menu_at(FolderMenu::Recent) else { return };
        let strings = cx.global::<Language>().strings();
        let go = self.go(cx);
        let recent = unopened(recent, &open);
        let mut entries = Vec::new();
        for (label, paths) in [(strings.open_folders, open), (strings.menu_recent, recent)] {
            if !paths.is_empty() {
                entries.push(MenuEntry::Heading { label: label.into(), button: None });
                entries.extend(ui::folder_entries(&paths, &go));
            }
        }
        if entries.is_empty() {
            entries.push(ui::nothing(strings));
        }
        let chrome = view::drop_chrome(self.placed, cx.global::<Theme>());
        ui::open_menu_at(entries, at, chrome, cx);
    }

    fn go(&self, cx: &Context<Self>) -> Go {
        let this = cx.entity().downgrade();
        Rc::new(move |path, dir, cx| {
            let _ = this.update(cx, |this, cx| this.choose_folder(path, dir, cx));
        })
    }

    fn choose_folder(&mut self, path: String, dir: bool, cx: &mut Context<Self>) {
        let Some(Mode::Dialog(host, ..)) = self.mode() else { return };
        cx.update_global::<History, _>(|history, _| history.record(&path, dir));
        let command = Command::Choose { host, path, keep_name: dir };
        self.finish(&command, cx);
        ui::perform(command, cx);
    }
}

fn unopened(recent: Vec<String>, open: &[String]) -> Vec<String> {
    recent.into_iter().filter(|folder| !open.iter().any(|shown| shown.eq_ignore_ascii_case(folder))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_folders_skip_the_ones_already_open_in_explorer() {
        let recent = vec!["C:\\Work".to_owned(), "D:\\Фото".to_owned(), "C:\\Users".to_owned()];
        let open = ["c:\\work".to_owned(), "D:\\ФОТО\\Море".to_owned()];
        assert_eq!(unopened(recent, &open), ["D:\\Фото", "C:\\Users"]);
    }
}
