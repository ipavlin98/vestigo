use super::{Choose, MenuEntry, MenuIcon, MenuItem};
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::settings::{self, Favorite, Settings, Visit};
use gpui::{App, AppContext, BorrowAppContext, Task};
use std::cmp::Reverse;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;

const RECENT_FOLDERS: usize = 12;
const LISTED_LIMIT: usize = 300;
const HIDDEN_ATTRIBUTE: u32 = 0x2;

pub(crate) type Go = Rc<dyn Fn(String, bool, &mut App)>;

struct Child {
    name: String,
    path: String,
    dir: bool,
}

pub(crate) fn recent_folders(visits: Vec<Visit>) -> Vec<String> {
    let folders = settings::recent_folders(visits).into_iter();
    folders.filter(|folder| Path::new(folder).is_dir()).take(RECENT_FOLDERS).collect()
}

pub(crate) fn favorite_entries(current: Option<&str>, reopen: Choose, go: &Go, cx: &App) -> Vec<MenuEntry> {
    let strings = cx.global::<Language>().strings();
    let favorites: Vec<&Favorite> =
        cx.global::<Settings>().menu.favorites.iter().filter(|favorite| !favorite.path.trim().is_empty()).collect();
    let toggle = current.map(|current| {
        let known = favorites.iter().any(|favorite| favorite.path.eq_ignore_ascii_case(current));
        let path = current.to_owned();
        let toggle: Choose = Rc::new(move |cx: &mut App| {
            toggle_favorite(&path, cx);
            reopen(cx);
        });
        (if known { assets::STAR_FILLED } else { assets::STAR }, toggle)
    });
    let mut entries = vec![MenuEntry::Heading { label: strings.menu_favorites.into(), button: toggle }];
    entries.extend(favorites.into_iter().map(|favorite| MenuEntry::Item(folder(label(favorite), &favorite.path, go))));
    entries
}

pub(crate) fn folder_entries(paths: &[String], go: &Go) -> Vec<MenuEntry> {
    paths.iter().map(|path| MenuEntry::Item(folder(name(path), path, go))).collect()
}

pub(crate) fn nothing(strings: &Strings) -> MenuEntry {
    MenuEntry::Item(MenuItem::new(strings.menu_nothing, MenuIcon::Blank))
}

fn toggle_favorite(path: &str, cx: &mut App) {
    let name = name(path);
    let path = path.to_owned();
    cx.update_global::<Settings, _>(|settings, _| {
        let favorites = &mut settings.menu.favorites;
        let before = favorites.len();
        favorites.retain(|favorite| !favorite.path.eq_ignore_ascii_case(&path));
        if favorites.len() == before {
            favorites.push(Favorite { name, path });
        }
    });
}

fn folder(label: String, path: &str, go: &Go) -> MenuItem {
    let (chosen, listed, go, inner) = (path.to_owned(), path.to_owned(), go.clone(), go.clone());
    MenuItem::new(label, MenuIcon::Path { path: path.to_owned(), dir: true })
        .on_choose(move |cx| go(chosen.clone(), true, cx))
        .submenu(move |cx| contents(listed.clone(), inner.clone(), cx))
}

fn contents(folder_path: String, go: Go, cx: &mut App) -> Task<Vec<MenuEntry>> {
    let empty = cx.global::<Language>().strings().empty_folder;
    let listed = cx.background_spawn(async move { children(&folder_path) });
    cx.spawn(async move |_| {
        let children = listed.await;
        if children.is_empty() {
            return vec![MenuEntry::Item(MenuItem::new(empty, MenuIcon::Blank))];
        }
        children
            .into_iter()
            .map(|Child { name, path, dir }| {
                MenuEntry::Item(if dir {
                    folder(name, &path, &go)
                } else {
                    let go = go.clone();
                    MenuItem::new(name, MenuIcon::Path { path: path.clone(), dir: false })
                        .on_choose(move |cx| go(path.clone(), false, cx))
                })
            })
            .collect()
    })
}

fn children(folder: &str) -> Vec<Child> {
    let Ok(listing) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut children: Vec<Child> = listing
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if metadata.file_attributes() & HIDDEN_ATTRIBUTE != 0 {
                return None;
            }
            let name = entry.file_name().into_string().ok()?;
            let path = entry.path().into_os_string().into_string().ok()?;
            Some(Child { name, path, dir: metadata.is_dir() })
        })
        .collect();
    children.sort_by_cached_key(|child| (Reverse(child.dir), child.name.to_lowercase()));
    children.truncate(LISTED_LIMIT);
    children
}

fn label(favorite: &Favorite) -> String {
    let name = favorite.name.trim();
    if name.is_empty() { favorite.path.clone() } else { name.to_owned() }
}

fn name(path: &str) -> String {
    Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_owned()
}
