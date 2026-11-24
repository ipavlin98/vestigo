use crate::index::Boosts;
use crate::platform;
use crate::settings::{Activity, History, Launch, Settings, Visit};
use gpui::{App, Global, Priority};
use rustc_hash::FxHashMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAY: u64 = 24 * 60 * 60;
const SYSTEM_BOOST: f32 = 20.0;
const HALF_LIFE: u64 = 3 * DAY;
const MAX_AGE: u64 = 30 * DAY;
const FOLDER_SHARE: f32 = 0.5;
const FIRST_USE_SHARE: f32 = 0.5;
const USE_SHARE: f32 = 0.1;
const STALE: Duration = Duration::from_secs(60);
const SHORTCUTS: [&str; 3] = ["lnk", "url", "appref-ms"];

/// What Windows knows about recent use: recent files and their folders, programs started from the
/// shell. Read in the background; the programs Vestigo saw in use and its own history join it per search.
#[derive(Default)]
pub(crate) struct Recency {
    snapshot: Snapshot,
    built: Option<Instant>,
    generation: u64,
}

impl Global for Recency {}

#[derive(Default)]
struct Snapshot {
    boosts: Boosts,
    shortcuts: FxHashMap<String, Vec<String>>,
    installed: Vec<String>,
}

/// The ranking boosts of a search: launches from Vestigo, and below them what was used elsewhere.
pub(crate) fn recent_boosts(cx: &App) -> Boosts {
    let history = cx.global::<History>();
    let mut boosts = history.boosts();
    let Some(recency) = cx.try_global::<Recency>().filter(|_| cx.global::<Settings>().index.windows_usage) else {
        return boosts;
    };
    let snapshot = &recency.snapshot;
    let now = now();
    for (path, &value) in &snapshot.boosts {
        raise(&mut boosts, path, value);
    }
    programs(&mut boosts, cx.global::<Activity>().apps(), &snapshot.shortcuts, now);
    for visit in history.visited().iter().filter(|visit| !visit.dir) {
        folder_of(&mut boosts, &visit.path, visit.at, &snapshot.installed, now);
    }
    boosts
}

pub(crate) fn refresh_recency(cx: &mut App) {
    let enabled = cx.global::<Settings>().index.windows_usage;
    let recency = cx.default_global::<Recency>();
    recency.generation += 1;
    recency.built = Some(Instant::now());
    let generation = recency.generation;
    if !enabled {
        recency.snapshot = Snapshot::default();
        return;
    }
    cx.spawn(async move |cx| {
        let snapshot =
            cx.background_executor().spawn_with_priority(Priority::Low, async { read_snapshot(now()) }).await;
        cx.update_global::<Recency, _>(|recency, _| {
            if recency.generation == generation {
                recency.snapshot = snapshot;
            }
        });
    })
    .detach();
}

pub(crate) fn freshen_recency(cx: &mut App) {
    if cx.try_global::<Recency>().and_then(|recency| recency.built).is_some_and(|at| at.elapsed() < STALE) {
        return;
    }
    refresh_recency(cx);
}

fn read_snapshot(now: u64) -> Snapshot {
    let installed = platform::program_folders();
    let mut shortcuts: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for (shortcut, target) in platform::shortcut_targets() {
        shortcuts.entry(target).or_default().push(shortcut);
    }
    let mut boosts = Boosts::default();
    files(&mut boosts, &platform::recent_items(), &installed, now);
    let launches = platform::assisted_launches();
    programs(&mut boosts, &launches, &shortcuts, now);
    for launch in &launches {
        folder_of(&mut boosts, &launch.path, launch.at, &installed, now);
    }
    Snapshot { boosts, shortcuts, installed }
}

/// Recent files and the folders they were opened from. Folders Windows lists on their own are
/// left out: every folder passed while browsing lands there.
fn files(boosts: &mut Boosts, recent: &[Visit], installed: &[String], now: u64) {
    for visit in recent.iter().filter(|visit| !visit.dir) {
        lift(boosts, &visit.path, SYSTEM_BOOST * weight(visit.at, now));
        folder_of(boosts, &visit.path, visit.at, installed, now);
    }
}

fn programs(boosts: &mut Boosts, launches: &[Launch], shortcuts: &FxHashMap<String, Vec<String>>, now: u64) {
    for launch in launches {
        let uses = FIRST_USE_SHARE + USE_SHARE * launch.count.saturating_sub(1) as f32;
        let value = SYSTEM_BOOST * weight(launch.at, now) * uses.min(1.0);
        lift(boosts, &launch.path, value);
        for shortcut in shortcuts.get(&launch.path.to_lowercase()).into_iter().flatten() {
            lift(boosts, shortcut, value);
        }
    }
}

/// The folder a file or program was opened from, unless it is a shortcut, a Store app or an
/// installed program: those say nothing about a folder the user worked in.
fn folder_of(boosts: &mut Boosts, path: &str, at: u64, installed: &[String], now: u64) {
    let lower = path.to_lowercase();
    let shortcut = Path::new(&lower)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SHORTCUTS.contains(&extension));
    if shortcut || platform::is_store_app(&lower) || installed.iter().any(|root| lower.starts_with(root.as_str())) {
        return;
    }
    if let Some(folder) = Path::new(&lower).parent().and_then(Path::to_str).filter(|folder| !folder.is_empty()) {
        lift(boosts, folder, SYSTEM_BOOST * FOLDER_SHARE * weight(at, now));
    }
}

fn lift(boosts: &mut Boosts, path: &str, value: f32) {
    let value = value.round() as i32;
    if value > 0 {
        raise(boosts, &path.to_lowercase(), value);
    }
}

fn raise(boosts: &mut Boosts, path: &str, value: i32) {
    match boosts.get_mut(path) {
        Some(boost) => *boost = (*boost).max(value),
        None => {
            boosts.insert(path.to_owned(), value);
        }
    }
}

fn weight(at: u64, now: u64) -> f32 {
    let age = now.saturating_sub(at);
    if age > MAX_AGE { 0.0 } else { 0.5f32.powf(age as f32 / HALF_LIFE as f32) }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;
    const INSTALLED: [&str; 1] = ["c:\\program files\\"];

    fn installed() -> Vec<String> {
        INSTALLED.map(str::to_owned).to_vec()
    }

    fn file(path: &str, age: u64) -> Visit {
        Visit { path: path.into(), dir: false, at: NOW - age }
    }

    fn launch(path: &str, age: u64, count: u32) -> Launch {
        Launch { path: path.to_lowercase(), at: NOW - age, count }
    }

    #[test]
    fn a_recent_file_lifts_itself_and_half_as_much_its_folder() {
        let mut boosts = Boosts::default();
        files(&mut boosts, &[file("D:\\Работа\\Отчёт.docx", 0)], &installed(), NOW);
        assert_eq!(boosts.get("d:\\работа\\отчёт.docx"), Some(&20));
        assert_eq!(boosts.get("d:\\работа"), Some(&10));
        assert_eq!(boosts.len(), 2);
    }

    #[test]
    fn folders_passed_while_browsing_get_nothing() {
        let mut boosts = Boosts::default();
        let browsed = Visit { path: "D:\\Работа\\Архив".into(), dir: true, at: NOW };
        files(&mut boosts, &[browsed], &installed(), NOW);
        assert!(boosts.is_empty());
    }

    #[test]
    fn the_boost_halves_every_three_days_and_ends_after_a_month() {
        assert_eq!(weight(NOW, NOW), 1.0);
        assert_eq!(weight(NOW - HALF_LIFE, NOW), 0.5);
        assert_eq!(weight(NOW - 31 * DAY, NOW), 0.0);
        assert_eq!(weight(NOW + DAY, NOW), 1.0);
    }

    #[test]
    fn a_program_lifts_its_start_menu_shortcuts_and_grows_with_its_uses() {
        let shortcut = "c:\\programdata\\start menu\\programs\\code.lnk".to_owned();
        let shortcuts = FxHashMap::from_iter([("d:\\tools\\code.exe".to_owned(), vec![shortcut.clone()])]);
        let mut boosts = Boosts::default();
        programs(&mut boosts, &[launch("D:\\Tools\\Code.exe", 0, 1)], &shortcuts, NOW);
        assert_eq!(boosts.get(&shortcut), Some(&10));
        programs(&mut boosts, &[launch("D:\\Tools\\Code.exe", 0, 9)], &shortcuts, NOW);
        assert_eq!((boosts.get("d:\\tools\\code.exe"), boosts.get(&shortcut)), (Some(&20), Some(&20)));
    }

    #[test]
    fn shortcuts_store_apps_and_installed_programs_mark_no_folder() {
        let mut boosts = Boosts::default();
        let opened = [
            "C:\\Users\\Алекс\\Start Menu\\Programs\\Code.lnk",
            "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App",
            "C:\\Program Files\\Tool\\tool.exe",
            "D:\\Portable\\tool.exe",
        ];
        for path in opened {
            folder_of(&mut boosts, path, NOW, &installed(), NOW);
        }
        assert_eq!(boosts.keys().map(String::as_str).collect::<Vec<_>>(), ["d:\\portable"]);
    }

    #[test]
    fn a_launch_from_vestigo_keeps_its_bigger_boost() {
        let mut boosts = Boosts::from_iter([("d:\\работа\\отчёт.docx".to_owned(), 55)]);
        files(&mut boosts, &[file("D:\\Работа\\Отчёт.docx", 0)], &installed(), NOW);
        assert_eq!(boosts.get("d:\\работа\\отчёт.docx"), Some(&55));
    }
}
