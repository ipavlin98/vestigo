mod dialog;

pub(crate) use dialog::DialogSources;

use crate::index::Hit;
use crate::platform::{self, Reach};
use crate::settings::{History, Settings, Suggestions, Visit};
use gpui::App;
use rustc_hash::FxHashSet;
use std::cmp::Reverse;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

const HIDDEN_ATTRIBUTES: u32 = 0x2 | 0x4;

type Folder = fn() -> Option<String>;

pub(crate) struct Sources {
    history: Vec<Visit>,
    suggestions: Suggestions,
}

impl Sources {
    pub(crate) fn of(cx: &App) -> Option<Self> {
        let settings = cx.global::<Settings>();
        let history = if settings.suggestions.history { cx.global::<History>().visits() } else { Vec::new() };
        settings.recent.then(|| Self { history, suggestions: settings.suggestions.clone() })
    }

    pub(crate) fn collect(self, limit: usize) -> Vec<Hit> {
        let Self { mut history, suggestions } = self;
        if suggestions.windows_recent {
            history.extend(platform::recent_items());
        }
        let folders: [(bool, Folder); 2] =
            [(suggestions.downloads, platform::downloads_folder), (suggestions.desktop, platform::desktop_folder)];
        for (wanted, folder) in folders {
            if wanted && let Some(folder) = folder() {
                history.extend(newest(&folder, limit, |_| true));
            }
        }
        pick(history, &suggestions, limit, found)
    }
}

fn found(visit: Visit) -> Option<Hit> {
    if platform::is_store_app(&visit.path) {
        return platform::store_app_name(&visit.path).map(|name| Hit::app(visit.path, name));
    }
    let exists = match platform::reach(&visit.path) {
        Reach::Quick => Path::new(&visit.path).exists(),
        Reach::Slow => true,
        Reach::Gone => false,
    };
    exists.then(|| Hit::new(visit.path, visit.dir))
}

fn pick(
    mut visits: Vec<Visit>,
    suggestions: &Suggestions,
    limit: usize,
    found: impl Fn(Visit) -> Option<Hit>,
) -> Vec<Hit> {
    visits.sort_by_key(|visit| Reverse(visit.at));
    let mut seen = FxHashSet::default();
    let mut hits = Vec::new();
    for visit in visits {
        if hits.len() >= limit {
            break;
        }
        if suggestions.allows(&visit.path)
            && seen.insert(visit.path.to_lowercase())
            && let Some(hit) = found(visit)
        {
            hits.push(hit);
        }
    }
    hits
}

fn newest(folder: &str, limit: usize, wanted: impl Fn(&Visit) -> bool) -> Vec<Visit> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut visits: Vec<Visit> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if metadata.file_attributes() & HIDDEN_ATTRIBUTES != 0 {
                return None;
            }
            let at = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            let path = entry.path().into_os_string().into_string().ok()?;
            Some(Visit { path, dir: metadata.is_dir(), at }).filter(&wanted)
        })
        .collect();
    visits.sort_by_key(|visit| Reverse(visit.at));
    visits.truncate(limit);
    visits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visit(path: &str, at: u64) -> Visit {
        Visit { path: path.into(), dir: false, at }
    }

    #[test]
    fn suggestions_merge_newest_first_without_repeats_or_hidden_files() {
        let suggestions = Suggestions { hidden: vec!["C:\\Temp\\secret.txt".into()], ..Suggestions::default() };
        let visits = vec![
            visit("C:\\Work\\plan.txt", 10),
            visit("C:\\Downloads\\Отчёт.pdf", 30),
            visit("c:\\work\\PLAN.txt", 5),
            visit("C:\\Temp\\secret.txt", 40),
            visit("C:\\Gone\\old.txt", 20),
            visit("C:\\Work\\notes.md", 1),
        ];
        let hits = pick(visits, &suggestions, 2, |visit| {
            (!visit.path.starts_with("C:\\Gone")).then(|| Hit::new(visit.path, visit.dir))
        });
        let paths: Vec<&str> = hits.iter().map(|hit| hit.path.as_str()).collect();
        assert_eq!(paths, ["C:\\Downloads\\Отчёт.pdf", "C:\\Work\\plan.txt"]);
    }
}
