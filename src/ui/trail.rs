use super::{Listing, ResultList};
use crate::i18n::Language;
use crate::index::Hit;
use crate::platform;
use crate::search::{self, Listed};
use crate::settings::Settings;
use gpui::{App, BorrowAppContext};
use std::path::Path;

const SHORTCUT: &str = "lnk";

#[derive(Clone, Copy)]
enum Way {
    Inside,
    Beside,
}

pub(crate) struct Entering {
    typed: String,
    listed: Listed,
    hit: Hit,
    way: Way,
}

impl Entering {
    pub(crate) fn inside(results: &ResultList, typed: &str) -> Option<Self> {
        let hit = results.selected().filter(|hit| hit.dir || is_shortcut(&hit.path))?;
        Some(Self::of(results, typed, hit, Way::Inside))
    }

    pub(crate) fn beside(results: &ResultList, typed: &str) -> Option<Self> {
        let hit = results.selected().filter(|hit| on_drive(&hit.path) && !hit.folder().is_empty())?;
        Some(Self::of(results, typed, hit, Way::Beside))
    }

    fn of(results: &ResultList, typed: &str, hit: &Hit, way: Way) -> Self {
        Self { typed: typed.to_owned(), listed: results.listing().listed(), hit: hit.clone(), way }
    }

    pub(crate) fn resolve(self) -> Option<Entered> {
        let Self { typed, listed, hit, way } = self;
        let (into, select) = match way {
            Way::Inside if hit.dir => (search::enter(&typed, listed, &hit.path, hit.name()), None),
            Way::Inside => (folder_text(&platform::shortcut_folder(&hit.path)?), None),
            Way::Beside => (folder_text(hit.folder()), Some(hit.path.clone())),
        };
        Some(Entered { from: search::origin(&typed, listed, hit.name()), row: Some(hit.path), into, select })
    }
}

fn marked(results: &ResultList) -> Option<String> {
    let folder = match results.listing() {
        Listing::Contents(_) => results.hits().first().map(Hit::folder),
        Listing::Recent
        | Listing::Folders
        | Listing::Searching
        | Listing::Found(_)
        | Listing::AtLeast(_)
        | Listing::ContentOff
        | Listing::ContentShort
        | Listing::Actions => {
            results.selected().map(|hit| if hit.dir { hit.path.trim_end_matches('\\') } else { hit.folder() })
        }
    };
    folder.filter(|folder| on_drive(folder)).map(str::to_owned)
}

pub(crate) fn set_mark(letter: char, results: &mut ResultList, cx: &mut App) {
    let Some(folder) = marked(results) else { return };
    results.announce(cx.global::<Language>().marked(letter, &folder));
    cx.update_global::<Settings, _>(|settings, _| {
        settings.marks.insert(letter, folder);
    });
}

pub(crate) fn go_to_mark(letter: char, results: &mut ResultList, typed: &str, cx: &App) -> Option<Entered> {
    let folder = cx.global::<Settings>().marks.get(&letter);
    if folder.is_none() {
        results.announce(cx.global::<Language>().unmarked(letter));
    }
    folder.map(|folder| Entered::jump(typed, folder))
}

fn is_shortcut(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(SHORTCUT))
}

fn on_drive(path: &str) -> bool {
    path.get(1..2) == Some(":")
}

fn folder_text(folder: &str) -> String {
    format!("{}\\", folder.trim_end_matches('\\'))
}

pub(crate) struct Entered {
    from: String,
    row: Option<String>,
    into: String,
    select: Option<String>,
}

impl Entered {
    pub(crate) fn jump(typed: &str, folder: &str) -> Self {
        Self { from: typed.trim().to_owned(), row: None, into: folder_text(folder), select: None }
    }
}

#[derive(Default)]
pub(crate) struct Trail(Vec<Entered>);

impl Trail {
    pub(crate) fn enter(&mut self, entered: Entered) -> (String, Option<String>) {
        let shown = (entered.into.clone(), entered.select.clone());
        self.0.push(entered);
        shown
    }

    pub(crate) fn back(&mut self, typed: &str) -> Option<(String, Option<String>)> {
        let current = folded(typed);
        if self.0.last().is_some_and(|step| !current.starts_with(folded(&step.into).trim_end_matches('\\'))) {
            self.0.clear();
        }
        match self.0.pop() {
            Some(Entered { from, row, .. }) => Some((from, row)),
            None => search::up(typed),
        }
    }
}

fn folded(text: &str) -> String {
    text.trim().to_lowercase().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(from: &str, row: &str, into: &str) -> Entered {
        Entered { from: from.into(), row: Some(row.into()), into: into.into(), select: None }
    }

    #[test]
    fn going_back_retraces_each_step_to_the_first_query() {
        let mut trail = Trail::default();
        let desktop = "C:\\Users\\Alex\\Desktop\\sublime";
        let (typed, _) = trail.enter(step("sublime", desktop, "C:\\Users\\Alex\\Desktop\\sublime\\"));
        let (typed, _) = trail.enter(step(&typed, &format!("{desktop}\\tmp"), &format!("{desktop}\\tmp\\")));
        let back = trail.back(&format!("{typed}notes"));
        assert_eq!(back, Some((format!("{desktop}\\"), Some(format!("{desktop}\\tmp")))));
        assert_eq!(trail.back(&format!("{desktop}\\")), Some(("sublime".to_owned(), Some(desktop.to_owned()))));
    }

    #[test]
    fn a_new_query_forgets_the_trail_and_going_back_climbs_a_level() {
        let mut trail = Trail::default();
        trail.enter(step("sublime", "C:\\work\\sublime", "C:\\work\\sublime\\"));
        assert_eq!(
            trail.back("C:\\Users\\Alex\\"),
            Some(("C:\\Users\\".to_owned(), Some("C:\\Users\\Alex".to_owned())))
        );
        assert_eq!(
            trail.back("C:\\work\\sublime\\"),
            Some(("C:\\work\\".to_owned(), Some("C:\\work\\sublime".to_owned())))
        );
        assert_eq!(trail.back("\\sfs\\src\\"), Some(("\\sfs\\".to_owned(), None)));
        assert_eq!(trail.back("\\sfs\\"), None);
        assert_eq!(trail.back("C:\\"), None);
        assert_eq!(trail.back("sublime"), None);
    }

    fn listed(paths: &[(&str, bool)], listing: Listing, selected: usize) -> ResultList {
        let mut list = ResultList::new(1.0);
        list.present(paths.iter().map(|&(path, dir)| Hit::new(path.to_owned(), dir)).collect(), listing);
        list.select(selected);
        list
    }

    #[test]
    fn a_mark_is_the_folder_being_viewed_or_the_one_holding_the_selection() {
        let inside = listed(&[("C:\\work\\src", true), ("C:\\work\\a.txt", false)], Listing::Contents(2), 1);
        assert_eq!(marked(&inside).as_deref(), Some("C:\\work"));
        let found = listed(&[("C:\\work\\src", true), ("D:\\notes\\a.txt", false)], Listing::Found(2), 1);
        assert_eq!(marked(&found).as_deref(), Some("D:\\notes"));
        let folder = listed(&[("C:\\work\\src", true)], Listing::Found(1), 0);
        assert_eq!(marked(&folder).as_deref(), Some("C:\\work\\src"));
        let app = listed(&[("shell:AppsFolder\\Calculator", false)], Listing::Found(1), 0);
        assert_eq!(marked(&app), None);
    }

    #[test]
    fn entering_beside_a_file_opens_its_folder_with_the_file_selected() {
        let found = listed(&[("C:\\work\\src\\main.rs", false)], Listing::Found(1), 0);
        let entered = Entering::beside(&found, "main").and_then(Entering::resolve).expect("a file has a folder");
        let mut trail = Trail::default();
        let shown = trail.enter(entered);
        assert_eq!(shown, ("C:\\work\\src\\".to_owned(), Some("C:\\work\\src\\main.rs".to_owned())));
        assert_eq!(trail.back("C:\\work\\src\\"), Some(("main".to_owned(), Some("C:\\work\\src\\main.rs".to_owned()))));
    }

    #[test]
    fn only_folders_and_shortcuts_can_be_entered() {
        assert!(is_shortcut("C:\\Recent\\sublime.LNK"));
        assert!(!is_shortcut("C:\\Recent\\sublime.txt"));
        assert!(!is_shortcut(".lnk"));
    }
}
