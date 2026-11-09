use super::newest;
use crate::index::Hit;
use crate::platform::{self, Host, Picked, Purpose, Types};
use crate::settings::{History, Settings, Visit};
use gpui::App;
use rustc_hash::FxHashMap;
use std::cmp::Ordering;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT: usize = 3;
const CONFIDENT: f32 = 1.0;
const SELECTED: f32 = 5.0;
const CAME_FROM: f32 = 4.0;
const OPEN_WINDOW: f32 = 1.2;
const OPEN_WINDOW_FALLOFF: f32 = 0.6;
const FRESH: f32 = 3.0;
const FRESH_HALF_LIFE: u64 = 10 * 60;
const FRESH_SECONDS: u64 = 2 * 60 * 60;
const SESSION: f32 = 2.0;
const SESSION_HALF_LIFE: u64 = 30 * 60;
const HABIT: f32 = 0.6;
const HABIT_HALF_LIFE: u64 = 7 * 24 * 60 * 60;
const PICKED: f32 = 1.5;
const PICKED_FALLOFF: f32 = 0.7;
const PICKED_HALF_LIFE: u64 = 14 * 24 * 60 * 60;
const PARENT: f32 = 0.7;
const FRESH_PARENT: f32 = 0.0;
const UNTYPED_FOLDER: f32 = 0.3;
const ALL_TYPES: &str = "*";
const UNFINISHED: [&str; 5] = ["crdownload", "part", "partial", "download", "tmp"];
const LAUNCHES: [&str; 8] = ["exe", "lnk", "url", "appref-ms", "bat", "cmd", "msc", "cpl"];
const START_MENU: &str = "\\start menu\\programs";

static ANY: Types = Types::Any;

pub(crate) struct DialogSources {
    host: Host,
    came_from: Option<Host>,
    visits: Vec<Visit>,
    opening: bool,
}

impl DialogSources {
    pub(crate) fn of(host: Host, came_from: Option<Host>, cx: &App) -> Self {
        let opening = cx.global::<Settings>().suggestions.open_dialogs;
        Self { host, came_from, visits: cx.global::<History>().visits(), opening }
    }

    pub(crate) fn collect(self) -> Vec<Hit> {
        let Self { host, came_from, mut visits, opening } = self;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let ((explorer, selection, came_from), ((purpose, picked), (recent, current))) = rayon::join(
            || {
                let selection = came_from.map(platform::explorer_selection).unwrap_or_default();
                (platform::explorer_folders(), selection, came_from.and_then(platform::host_folder))
            },
            || {
                rayon::join(
                    || {
                        let purpose = platform::dialog_purpose(host);
                        let picked = platform::dialog_picks(&lists(wanted(&purpose)));
                        (purpose, picked)
                    },
                    || (platform::recent_items(), platform::host_folder(host)),
                )
            },
        );
        if !opening && matches!(purpose, Purpose::Open(_) | Purpose::Browse) {
            return Vec::new();
        }
        visits.extend(recent);
        let types = wanted(&purpose);
        let mut fresh = Vec::new();
        for folder in explorer.iter().cloned().chain(platform::landing_folders()) {
            fresh.extend(newest(&folder, LIMIT, |visit| arrived(visit, types, now)));
        }
        let situation = Situation { current, came_from, selection, explorer, fresh, visits, picked, purpose, now };
        situation.rank(|path| std::fs::metadata(path).ok().map(|metadata| metadata.is_dir()))
    }
}

struct Situation {
    purpose: Purpose,
    current: Option<String>,
    came_from: Option<String>,
    selection: Vec<String>,
    explorer: Vec<String>,
    fresh: Vec<Visit>,
    visits: Vec<Visit>,
    picked: Vec<Picked>,
    now: u64,
}

impl Situation {
    fn rank(&self, kind: impl Fn(&str) -> Option<bool>) -> Vec<Hit> {
        let types = wanted(&self.purpose);
        let opens = matches!(self.purpose, Purpose::Open(_));
        let mut evidence = Evidence { opens, ..Evidence::default() };
        let visited = match types {
            Types::Any => 1.0,
            Types::Only(_) => UNTYPED_FOLDER,
        };
        if let [selected] = self.selection.as_slice() {
            match kind(selected) {
                Some(true) => evidence.folder(selected, SELECTED),
                Some(false) if types.admit(selected) => evidence.file(selected, SELECTED),
                Some(false) | None => {}
            }
        }
        if let Some(folder) = &self.came_from {
            evidence.folder(folder, CAME_FROM);
        }
        for (place, folder) in (0..).zip(&self.explorer) {
            evidence.folder(folder, OPEN_WINDOW * OPEN_WINDOW_FALLOFF.powi(place));
        }
        for visit in self.fresh.iter().filter(|visit| arrived(visit, types, self.now)) {
            let weight = FRESH * decay(self.now.saturating_sub(visit.at), FRESH_HALF_LIFE);
            evidence.used(&visit.path, weight, FRESH_PARENT);
        }
        for visit in &self.visits {
            let age = self.now.saturating_sub(visit.at);
            let weight = SESSION * decay(age, SESSION_HALF_LIFE) + HABIT * decay(age, HABIT_HALF_LIFE);
            match visit.dir {
                true => evidence.folder(&visit.path, weight * visited),
                false if types.admit(&visit.path) => evidence.used(&visit.path, weight, PARENT),
                false => {}
            }
        }
        for list in &self.picked {
            let mut weight = PICKED * decay(self.now.saturating_sub(list.at), PICKED_HALF_LIFE);
            for path in &list.paths {
                match kind(path) {
                    Some(true) => evidence.folder(path, weight * visited),
                    Some(false) if types.admit(path) => evidence.used(path, weight, PARENT),
                    Some(false) | None => {}
                }
                weight *= PICKED_FALLOFF;
            }
        }
        evidence.best(self.current.as_deref(), kind)
    }
}

fn wanted(purpose: &Purpose) -> &Types {
    match purpose {
        Purpose::Open(types) | Purpose::Save(types) => types,
        Purpose::Browse => &ANY,
    }
}

fn lists(types: &Types) -> Vec<String> {
    match types {
        Types::Any => vec![ALL_TYPES.to_owned()],
        Types::Only(extensions) => extensions.clone(),
    }
}

fn arrived(visit: &Visit, types: &Types, now: u64) -> bool {
    !visit.dir
        && !extension_in(&visit.path, &UNFINISHED)
        && now.saturating_sub(visit.at) <= FRESH_SECONDS
        && types.admit(&visit.path)
}

fn decay(age: u64, half_life: u64) -> f32 {
    0.5f32.powf(age as f32 / half_life as f32)
}

fn extension_in(path: &str, extensions: &[&str]) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extensions.iter().any(|known| known.eq_ignore_ascii_case(extension)))
}

fn launches(path: &str) -> bool {
    extension_in(path, &LAUNCHES) || path.to_lowercase().contains(START_MENU)
}

fn key(path: &str) -> String {
    path.trim_end_matches('\\').to_lowercase()
}

#[derive(Default)]
struct Evidence {
    opens: bool,
    candidates: Vec<Candidate>,
    known: FxHashMap<String, usize>,
}

struct Candidate {
    path: String,
    dir: bool,
    score: f32,
}

impl Evidence {
    fn folder(&mut self, path: &str, weight: f32) {
        self.add(path, true, weight);
    }

    fn file(&mut self, path: &str, weight: f32) {
        self.add(path, false, weight);
    }

    fn used(&mut self, file: &str, weight: f32, parent_share: f32) {
        if launches(file) {
            return;
        }
        let share = if self.opens { parent_share } else { 1.0 };
        self.file(file, weight);
        if let Some(parent) = Path::new(file).parent().and_then(Path::to_str) {
            self.folder(parent, weight * share);
        }
    }

    fn add(&mut self, path: &str, dir: bool, weight: f32) {
        if weight <= 0.0 || launches(path) || !dir && !self.opens {
            return;
        }
        let key = key(path);
        if let Some(candidate) = self.known.get(&key).and_then(|&at| self.candidates.get_mut(at)) {
            candidate.score += weight;
            return;
        }
        self.known.insert(key, self.candidates.len());
        self.candidates.push(Candidate { path: path.to_owned(), dir, score: weight });
    }

    fn best(mut self, current: Option<&str>, kind: impl Fn(&str) -> Option<bool>) -> Vec<Hit> {
        let current = current.map(key);
        self.candidates.retain(|candidate| current.as_ref() != Some(&key(&candidate.path)));
        self.candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        self.candidates
            .into_iter()
            .take_while(|candidate| candidate.score >= CONFIDENT)
            .filter(|candidate| kind(&candidate.path) == Some(candidate.dir))
            .take(LIMIT)
            .map(|candidate| Hit::new(candidate.path, candidate.dir))
            .collect()
    }
}

#[cfg(test)]
mod tests;
