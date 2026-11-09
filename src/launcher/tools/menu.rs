use crate::index::Hit;
use crate::platform::ShellItem;
use crate::search;
use crate::ui::{self, Extra, Shown, Tool};
use gpui::RenderImage;
use std::ops::Range;
use std::sync::Arc;

pub(in crate::launcher) struct Menu {
    hit: Hit,
    targets: Vec<Hit>,
    query: String,
    shown: Shown,
    typed: String,
    shell: Vec<ShellRow>,
    levels: Vec<Level>,
}

struct Level {
    title: String,
    rows: Vec<ShellRow>,
    shown: Shown,
    typed: String,
}

impl Menu {
    pub(super) fn new(hit: Hit, targets: Vec<Hit>, query: String, shown: Shown, shell: Vec<ShellRow>) -> Self {
        Self { hit, targets, query, shown, typed: String::new(), shell, levels: Vec::new() }
    }

    pub(super) fn hit(&self) -> &Hit {
        &self.hit
    }

    pub(super) fn targets(&self) -> &[Hit] {
        &self.targets
    }

    pub(super) fn at_top(&self) -> bool {
        self.levels.is_empty()
    }

    pub(super) fn row(&self, index: usize) -> Option<&ShellRow> {
        self.rows().get(index)
    }

    fn rows(&self) -> &[ShellRow] {
        self.levels.last().map_or(&self.shell, |level| &level.rows)
    }

    pub(in crate::launcher) fn title(&self) -> Option<&str> {
        self.levels.last().map(|level| level.title.as_str())
    }

    pub(super) fn filter(&mut self, typed: String) -> bool {
        let changed = self.typed != typed;
        self.typed = typed;
        changed
    }

    pub(super) fn descend(&mut self, title: String, rows: Vec<ShellRow>, shown: Shown) {
        let typed = std::mem::take(&mut self.typed);
        self.levels.push(Level { title, rows, shown, typed });
    }

    pub(super) fn ascend(&mut self) -> Option<(Shown, String)> {
        let Level { shown, typed, .. } = self.levels.pop()?;
        self.typed.clone_from(&typed);
        Some((shown, typed))
    }

    pub(super) fn left(self) -> (String, Shown) {
        (self.query, self.shown)
    }

    pub(super) fn listed(&self, mut builtin: Vec<Extra>) -> (Vec<Extra>, Option<usize>) {
        builtin.retain(|row| find(&row.title, &self.typed).is_some());
        let actions = builtin.len();
        let shell = self.rows().iter().enumerate().filter(|(_, row)| find(&row.title, &self.typed).is_some());
        builtin.extend(shell.map(|(index, row)| Extra {
            tool: Tool::Shell { index, submenu: row.submenu },
            title: row.title.clone(),
            detail: String::new(),
            argument: self.hit.path.clone(),
        }));
        let heading = (self.at_top() && builtin.len() > actions).then_some(actions);
        (builtin, heading)
    }
}

pub(in crate::launcher) struct ShellRow {
    pub(super) title: String,
    pub(in crate::launcher) icon: Option<(Arc<RenderImage>, u32)>,
    pub(super) submenu: bool,
    pub(super) trail: Vec<u32>,
}

impl ShellRow {
    fn of(item: ShellItem, background: f32) -> Self {
        let icon = item.icon.and_then(|mut icon| {
            ui::readable(&mut icon.bgra, background);
            Some((ui::picture(icon.width, icon.height, icon.bgra)?, icon.width))
        });
        Self { title: item.title, icon, submenu: item.submenu, trail: item.trail }
    }
}

pub(super) fn shell_rows(items: Vec<ShellItem>, background: f32) -> Vec<ShellRow> {
    items.into_iter().map(|item| ShellRow::of(item, background)).collect()
}

pub(in crate::launcher) fn find(text: &str, query: &str) -> Option<Range<usize>> {
    find_typed(text, query).or_else(|| find_typed(text, &search::other_layout(query)?))
}

fn find_typed(text: &str, query: &str) -> Option<Range<usize>> {
    let query: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(0..0);
    }
    text.char_indices().find_map(|(start, _)| {
        let mut lowered = Vec::with_capacity(query.len());
        let mut end = start;
        for ch in text.get(start..)?.chars() {
            if lowered.len() >= query.len() {
                break;
            }
            lowered.extend(ch.to_lowercase());
            end += ch.len_utf8();
        }
        (lowered == query).then_some(start..end)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{Listing, ResultList};

    const PATH: &str = "C:\\Документы\\отчёт.txt";

    fn shown(query_rows: usize) -> Shown {
        let mut list = ResultList::new(1.0);
        let hits = (0..query_rows).map(|row| Hit::new(format!("C:\\{row}.txt"), false)).collect();
        list.present_with(Vec::new(), hits, Vec::new(), Listing::Found(query_rows));
        list.shown()
    }

    fn shell(titles: &[&str]) -> Vec<ShellRow> {
        titles
            .iter()
            .map(|title| ShellRow { title: (*title).to_owned(), icon: None, submenu: false, trail: Vec::new() })
            .collect()
    }

    fn menu(titles: &[&str]) -> Menu {
        let hit = Hit::new(PATH.to_owned(), false);
        Menu::new(hit.clone(), vec![hit], "отчёт".to_owned(), shown(3), shell(titles))
    }

    fn action(title: &str) -> Extra {
        Extra {
            tool: Tool::Action { index: 0, dir: false, builtin: None },
            title: title.to_owned(),
            detail: String::new(),
            argument: PATH.to_owned(),
        }
    }

    fn titles(rows: &[Extra]) -> Vec<&str> {
        rows.iter().map(|row| row.title.as_str()).collect()
    }

    #[test]
    fn the_context_menu_gets_a_heading_after_the_actions_only_at_the_top() {
        let mut menu = menu(&["Отправить", "Сжать"]);
        let (rows, heading) = menu.listed(vec![action("Открыть"), action("Копировать")]);
        assert_eq!(titles(&rows), ["Открыть", "Копировать", "Отправить", "Сжать"]);
        assert_eq!(heading, Some(2));
        menu.descend("Отправить".to_owned(), shell(&["Почта"]), shown(4));
        let (rows, heading) = menu.listed(Vec::new());
        assert_eq!(titles(&rows), ["Почта"]);
        assert_eq!(heading, None);
    }

    #[test]
    fn typing_narrows_actions_and_shell_rows_and_keeps_their_indexes() {
        let mut menu = menu(&["Отправить", "Сжать"]);
        assert!(menu.filter("сжа".to_owned()));
        assert!(!menu.filter("сжа".to_owned()), "the same text changes nothing");
        let (rows, heading) = menu.listed(vec![action("Открыть")]);
        assert_eq!(titles(&rows), ["Сжать"]);
        assert_eq!(heading, Some(0));
        assert_eq!(rows.first().map(|row| row.tool), Some(Tool::Shell { index: 1, submenu: false }));
    }

    #[test]
    fn a_heading_needs_a_shell_row_below_it() {
        let mut menu = menu(&["Отправить"]);
        menu.filter("откр".to_owned());
        assert_eq!(menu.listed(vec![action("Открыть")]).1, None);
    }

    #[test]
    fn a_submenu_starts_unfiltered_and_going_back_restores_what_was_typed() {
        let mut menu = menu(&["Отправить"]);
        menu.filter("отпр".to_owned());
        menu.descend("Отправить".to_owned(), shell(&["Почта"]), shown(4));
        assert_eq!(menu.title(), Some("Отправить"));
        assert_eq!(titles(&menu.listed(Vec::new()).0), ["Почта"]);
        let (_, typed) = menu.ascend().expect("a level");
        assert_eq!(typed, "отпр");
        assert_eq!(menu.title(), None);
        assert!(menu.at_top());
        assert!(!menu.filter("отпр".to_owned()));
        assert!(menu.ascend().is_none());
    }

    #[test]
    fn leaving_returns_the_query_the_menu_was_opened_from() {
        assert_eq!(menu(&[]).left().0, "отчёт");
    }

    #[test]
    fn actions_match_anywhere_in_the_title_ignoring_case() {
        assert_eq!(find("Открыть папку", "от"), Some(0..4));
        assert_eq!(find("Копировать путь", " ПУТЬ "), Some(21..29));
        assert_eq!(find("Copy name", "NAME"), Some(5..9));
        assert_eq!(find("Свойства", ""), Some(0..0));
        assert_eq!(find("Свойства", "путь"), None);
        assert_eq!(find("Cut", "cuts"), None);
    }

    #[test]
    fn actions_match_a_query_typed_in_the_other_layout() {
        assert_eq!(find("Открыть", "jnr"), Some(0..6));
        assert_eq!(find("Copy path", "сщзн"), Some(0..4));
    }
}
