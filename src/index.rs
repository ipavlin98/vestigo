mod apps;
mod cache;
mod content;
mod details;
mod diagnose;
mod drive;
mod extra;
mod facets;
mod lookup;
mod ntfs;
#[cfg(test)]
mod tests;
mod volume;
mod walk;

use crate::search::{Listed, Query};
use apps::Apps;
use content::Content;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use volume::{ROOT, Volume};

pub(crate) use apps::StoreApp;
#[cfg(test)]
pub(crate) use content::read_documents;
pub(crate) use content::{ContentEstimate, ContentStatus, Pipe, Unread, serve};
pub(crate) use diagnose::Diagnosis;
pub(crate) use extra::FolderStatus;
use extra::Watched;
pub(crate) use facets::{Counts, Facets};

const SHORTLIST: usize = 400;
const MAX_DEPTH_PENALTY: usize = 12;
const NOISE_PENALTY: i32 = 10;
const HIDDEN_PENALTY: i32 = 10;
const PRIORITY: i32 = 30;
const NOISE: [&str; 4] = ["\\appdata\\local\\temp\\", "\\node_modules\\", "\\.git\\", "\\windows\\"];
const SHORTCUT_TYPES: [&str; 3] = ["lnk", "url", "appref-ms"];

pub(crate) type Boosts = FxHashMap<String, i32>;
type Changed = Arc<dyn Fn(Update) + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    Contents,
    Count,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Programs,
    Folder(String),
    Anything,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snippet {
    pub(crate) text: String,
    pub(crate) ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub(crate) path: String,
    pub(crate) name_start: usize,
    pub(crate) dir: bool,
    pub(crate) highlights: Vec<Range<usize>>,
    pub(crate) marks: Vec<Range<usize>>,
    pub(crate) size: Option<u64>,
    pub(crate) modified: Option<i64>,
    pub(crate) snippet: Option<Snippet>,
    title: Option<String>,
}

impl Hit {
    pub(crate) fn new(path: String, dir: bool) -> Self {
        let name_start = path.trim_end_matches('\\').rfind('\\').map_or(0, |slash| slash + 1);
        Self {
            path,
            name_start,
            dir,
            highlights: Vec::new(),
            marks: Vec::new(),
            size: None,
            modified: None,
            snippet: None,
            title: None,
        }
    }

    pub(crate) fn app(path: String, name: String) -> Self {
        Self { title: Some(name), ..Self::new(path, false) }
    }

    pub(crate) fn name(&self) -> &str {
        self.title.as_deref().unwrap_or_else(|| self.path.get(self.name_start..).unwrap_or(&self.path))
    }

    pub(crate) fn folder(&self) -> &str {
        self.path.get(..self.name_start).map_or("", |folder| folder.trim_end_matches('\\'))
    }

    pub(crate) fn label(&self, hide_shortcut_extensions: bool) -> &str {
        let name = self.name();
        if !hide_shortcut_extensions || self.dir {
            return name;
        }
        match name.rsplit_once('.') {
            Some((stem, extension))
                if !stem.is_empty() && SHORTCUT_TYPES.iter().any(|kind| kind.eq_ignore_ascii_case(extension)) =>
            {
                stem
            }
            Some(_) | None => name,
        }
    }
}

#[derive(Default)]
pub(crate) struct Results {
    pub(crate) hits: Vec<Hit>,
    pub(crate) total: usize,
    pub(crate) listed: Listed,
}

pub(crate) struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub(crate) fn new() -> Self {
        Self(Arc::default())
    }

    pub(crate) fn token(&self) -> Token {
        Token(Some(self.0.clone()))
    }
}

impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Default)]
pub(crate) struct Token(Option<Arc<AtomicBool>>);

impl Token {
    pub(crate) fn cancelled(&self) -> bool {
        self.0.as_ref().is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Status {
    pub(crate) scanning: bool,
    pub(crate) entries: usize,
    pub(crate) details: bool,
    pub(crate) content: ContentStatus,
    pub(crate) content_cache: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ranking {
    excluded: Vec<String>,
    high: Vec<String>,
    low: Vec<String>,
    hidden_low: bool,
}

impl Default for Ranking {
    fn default() -> Self {
        Self { excluded: Vec::new(), high: Vec::new(), low: Vec::new(), hidden_low: true }
    }
}

impl Ranking {
    pub(crate) fn new(excluded: &[String], high: &[String], low: &[String], hidden_low: bool) -> Self {
        let folders = |folders: &[String]| folders.iter().filter_map(|folder| normalized(folder)).collect();
        Self { excluded: folders(excluded), high: folders(high), low: folders(low), hidden_low }
    }

    fn score(&self, lower: &str, hidden: bool) -> i32 {
        let inside = |folders: &[String]| folders.iter().any(|folder| contains(folder, lower));
        let mut score = 0;
        if inside(&self.high) {
            score += PRIORITY;
        }
        if inside(&self.low) {
            score -= PRIORITY;
        }
        if hidden && self.hidden_low {
            score -= HIDDEN_PENALTY;
        }
        score
    }
}

fn normalized(folder: &str) -> Option<String> {
    let folder = folder.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    (!folder.is_empty()).then_some(folder)
}

fn contains(folder: &str, lower: &str) -> bool {
    lower.strip_prefix(folder).is_some_and(|rest| rest.is_empty() || rest.starts_with('\\'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DriveStatus {
    pub(crate) letter: char,
    pub(crate) entries: usize,
}

#[derive(Clone)]
struct Launch {
    drives: Vec<Drive>,
    folders: Vec<String>,
    changed: Changed,
}

#[derive(Default)]
pub(crate) struct Index {
    volumes: RwLock<Vec<Arc<RwLock<Volume>>>>,
    scanning: AtomicUsize,
    cache: Option<PathBuf>,
    generation: AtomicU64,
    launch: parking_lot::Mutex<Option<Launch>>,
    ranking: RwLock<Ranking>,
    apps: RwLock<Apps>,
    watched: parking_lot::Mutex<Vec<Watched>>,
    rewalk: parking_lot::Mutex<Option<Duration>>,
    content: Content,
}

impl Index {
    pub(crate) fn with_cache(cache: PathBuf) -> Self {
        Self { cache: Some(cache), ..Self::default() }
    }

    pub(crate) fn save(&self) {
        for volume in self.volumes.read().iter().filter(|volume| !volume.read().is_folder()) {
            self.save_shared(volume);
        }
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        drives: Vec<Drive>,
        folders: Vec<String>,
        changed: impl Fn(Update) + Send + Sync + 'static,
    ) {
        let launch = Launch { drives, folders, changed: Arc::new(changed) };
        *self.launch.lock() = Some(launch.clone());
        self.launch(launch, false);
    }

    pub(crate) fn rebuild(self: &Arc<Self>) {
        let Some(launch) = self.launch.lock().clone() else { return };
        self.relaunch(launch, true);
    }

    pub(crate) fn restart(self: &Arc<Self>, drives: Vec<Drive>, folders: Vec<String>) {
        let Some(launch) = self.launch.lock().as_mut().map(|launch| {
            launch.drives = drives;
            launch.folders = folders;
            launch.clone()
        }) else {
            return;
        };
        self.save();
        self.relaunch(launch, false);
    }

    pub(crate) fn rank(&self, ranking: Ranking) {
        *self.ranking.write() = ranking;
        self.content.touch();
    }

    pub(crate) fn list_apps(&self, apps: Vec<StoreApp>) {
        if self.apps.read().lists(&apps) {
            return;
        }
        *self.apps.write() = Apps::new(apps);
        let changed = self.launch.lock().as_ref().map(|launch| launch.changed.clone());
        if let Some(changed) = changed {
            changed(Update::Contents);
        }
    }

    fn relaunch(self: &Arc<Self>, launch: Launch, fresh: bool) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.watched.lock().clear();
        self.volumes.write().clear();
        (launch.changed)(Update::Contents);
        self.launch(launch, fresh);
    }

    fn launch(self: &Arc<Self>, launch: Launch, fresh: bool) {
        let Launch { drives, folders, changed } = launch;
        let generation = self.generation.load(Ordering::Relaxed);
        let folders: Vec<String> =
            folders.into_iter().filter(|folder| !drives.iter().any(|drive| folder_on(folder, drive.letter))).collect();
        self.scanning.fetch_add(drives.len() + folders.len(), Ordering::Relaxed);
        for folder in folders {
            let (wake, woken) = mpsc::channel();
            let status = FolderStatus { path: folder.clone(), entries: 0, walked: None, walking: true };
            self.watched.lock().push(Watched { status, wake });
            let index = self.clone();
            let changed = changed.clone();
            let path = folder.clone();
            let spawned = std::thread::Builder::new()
                .name("index-folder".into())
                .spawn(move || index.index_folder(&path, &woken, &changed, Job { generation, fresh }));
            if let Err(error) = spawned {
                log::error!("Couldn't start indexing a folder: {error}");
                self.watched.lock().retain(|watched| watched.status.path != folder);
                self.scanning.fetch_sub(1, Ordering::Relaxed);
            }
        }
        for drive in drives {
            let index = self.clone();
            let changed = changed.clone();
            let job = Job { generation, fresh };
            let spawned = std::thread::Builder::new()
                .name(format!("index-{}", drive.letter))
                .spawn(move || index.index_drive(drive, changed, job));
            if let Err(error) = spawned {
                log::error!("Couldn't start indexing: {error}");
                self.scanning.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn drives(&self) -> Vec<DriveStatus> {
        let volumes = self.volumes.read();
        let drives = volumes.iter().map(|volume| volume.read()).filter(|volume| !volume.is_folder());
        drives.map(|volume| DriveStatus { letter: volume.letter(), entries: volume.len() }).collect()
    }

    pub(crate) fn status(&self) -> Status {
        let volumes = self.volumes.read();
        let entries = volumes.iter().map(|volume| volume.read().len()).sum();
        let details = volumes.iter().all(|volume| volume.read().details_complete());
        Status {
            scanning: self.scanning.load(Ordering::Relaxed) > 0,
            entries,
            details,
            content: self.content.status(),
            content_cache: self.content.cache_bytes(),
        }
    }

    pub(crate) fn search(&self, query: &Query, boosts: &Boosts, focus: &Focus, limit: usize) -> Results {
        self.search_with(query, boosts, focus, limit, &Facets::default()).0
    }

    pub(crate) fn search_with(
        &self,
        query: &Query,
        boosts: &Boosts,
        focus: &Focus,
        limit: usize,
        facets: &Facets,
    ) -> (Results, Counts) {
        self.search_until(query, boosts, focus, limit, facets, &Token::default()).unwrap_or_default()
    }

    fn current(&self, job: Job) -> bool {
        self.generation.load(Ordering::Relaxed) == job.generation
    }

    fn cache_file(&self, letter: char) -> Option<PathBuf> {
        self.cache.as_deref().map(|dir| cache::file(dir, letter))
    }

    fn save_volume(&self, volume: &Volume) {
        self.commit_cache(volume.letter(), |path| cache::write(path, volume));
    }

    fn save_shared(&self, volume: &RwLock<Volume>) {
        let letter = volume.read().letter();
        self.commit_cache(letter, |path| cache::write(path, &volume.read()));
    }

    fn commit_cache(&self, letter: char, write: impl FnOnce(&Path) -> anyhow::Result<Option<cache::Written>>) {
        let Some(path) = self.cache_file(letter) else { return };
        let saved = write(&path).and_then(|written| written.map_or(Ok(()), cache::Written::commit));
        if let Err(error) = saved {
            log::warn!("{letter}: couldn't save the index cache ({error:#})");
        }
    }

    fn replace(&self, volume: Volume, job: Job) -> Option<Arc<RwLock<Volume>>> {
        let root = volume.path(ROOT);
        let mut volumes = self.volumes.write();
        volumes.retain(|known| {
            let known = known.read();
            !(known.is_folder() && known.path(ROOT) == root)
        });
        self.add_to(&mut volumes, volume, job)
    }

    fn add(&self, volume: Volume, job: Job) -> Option<Arc<RwLock<Volume>>> {
        self.add_to(&mut self.volumes.write(), volume, job)
    }

    fn add_to(&self, volumes: &mut Vec<Arc<RwLock<Volume>>>, volume: Volume, job: Job) -> Option<Arc<RwLock<Volume>>> {
        let volume = Arc::new(RwLock::new(volume));
        if !self.current(job) {
            return None;
        }
        volumes.push(volume.clone());
        volumes.sort_by_key(|volume| volume.read().letter());
        Some(volume)
    }

    fn finish_scan(&self, letter: char, started: Instant, changed: &(dyn Fn(Update) + Send + Sync)) {
        log::info!("Indexed {letter}: in {:?}", started.elapsed());
        self.scanning.fetch_sub(1, Ordering::Relaxed);
        changed(Update::Contents);
    }
}

#[derive(Clone, Copy, Debug)]
struct Job {
    generation: u64,
    fresh: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Drive {
    pub(crate) letter: char,
    pub(crate) journal: bool,
}

fn folder_on(folder: &str, letter: char) -> bool {
    let mut chars = folder.chars();
    chars.next().is_some_and(|drive| drive.eq_ignore_ascii_case(&letter)) && chars.next() == Some(':')
}

fn context_score(path: &str, depth: usize, hidden: bool, boosts: &Boosts, ranking: &Ranking) -> i32 {
    let lower = path.to_lowercase();
    let mut score = -(depth.min(MAX_DEPTH_PENALTY) as i32) + ranking.score(&lower, hidden);
    if NOISE.iter().any(|noise| lower.contains(noise)) {
        score -= NOISE_PENALTY;
    }
    score + boosts.get(&lower).copied().unwrap_or(0)
}
