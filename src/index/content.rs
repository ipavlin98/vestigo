mod docs;
mod extract;
mod grams;
mod library;
mod query;
mod store;
#[cfg(test)]
mod tests;
mod worker;

pub(crate) use extract::serve;
pub(crate) use worker::{Pipe, Spawn};

use super::volume::Volume;
use super::{Index, Update, contains};
use anyhow::Result;
pub(crate) use docs::Unread;
use docs::{Candidate, Outcome, Plan, reconcile};
use extract::Format;
use library::{Extracted, Library};
use parking_lot::{Condvar, Mutex, RwLock};
use rayon::ThreadPool;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use worker::{Extractor, Job, Step, Stop};

const CONTENT_NOISE: [&str; 16] = [
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "appdata",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "__pycache__",
    ".venv",
    "site-packages",
    ".cargo",
    ".rustup",
    "go\\pkg",
    "target",
];
const SWEEP_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_secs(60);
const SAVE_INTERVAL: Duration = Duration::from_secs(10 * 60);
const IDLE_WAIT: Duration = Duration::from_secs(60);
const BATCH_BYTES: usize = 16 << 20;
const PUBLISH_INTERVAL: Duration = Duration::from_secs(2);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
const SHOWN_QUEUE: usize = 16;
const PDF_TEXT_SHARE: f64 = 0.08;
const OFFICE_TEXT_SHARE: f64 = 0.35;
const CACHE_SHARE: f64 = 0.38;
const PDF_BYTES_PER_SECOND: f64 = 6e6;
const TEXT_BYTES_PER_SECOND: f64 = 250e6;
const BUILD_BYTES_PER_SECOND: f64 = 200e6;
const SECONDS_PER_OFFICE_FILE: f64 = 0.002;
const SECONDS_PER_TEXT_FILE: f64 = 0.0005;
const SLOW_MARGIN: u64 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentStatus {
    #[default]
    Off,
    Building {
        done: usize,
        total: usize,
        left: Option<u64>,
    },
    Ready {
        files: usize,
        built: u64,
        unread: Unread,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentEstimate {
    pub(crate) files: usize,
    pub(crate) text: u64,
    pub(crate) cache: u64,
    pub(crate) seconds: (u64, u64),
}

#[derive(Default)]
struct Signal {
    run: Option<u64>,
    runs: u64,
    touched: bool,
}

#[derive(Default)]
pub(crate) struct Content {
    folder: Option<PathBuf>,
    spawn: Option<Spawn>,
    library: RwLock<Option<Library>>,
    status: Mutex<ContentStatus>,
    signal: Mutex<Signal>,
    wake: Condvar,
    stopper: Mutex<Option<Stop>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    cache: AtomicU64,
}

impl Content {
    pub(super) fn touch(&self) {
        self.signal.lock().touched = true;
        self.wake.notify_all();
    }

    pub(super) fn note(&self, volume: &RwLock<Volume>) {
        let edits = volume.write().take_edits();
        if self.enabled() && volume.read().edits_documents(&edits, &CONTENT_NOISE, document) {
            self.touch();
        }
    }

    pub(super) fn enabled(&self) -> bool {
        self.signal.lock().run.is_some()
    }

    pub(super) fn status(&self) -> ContentStatus {
        *self.status.lock()
    }

    fn alive(&self, run: u64) -> bool {
        self.signal.lock().run == Some(run)
    }

    fn pause(&self, run: u64, time: Duration) {
        let until = Instant::now() + time;
        let mut signal = self.signal.lock();
        while signal.run == Some(run) && !self.wake.wait_until(&mut signal, until).timed_out() {}
    }

    fn wait_for_touch(&self, run: u64) -> bool {
        let until = Instant::now() + IDLE_WAIT;
        let mut signal = self.signal.lock();
        while signal.run == Some(run) && !signal.touched {
            if self.wake.wait_until(&mut signal, until).timed_out() {
                break;
            }
        }
        std::mem::take(&mut signal.touched)
    }

    fn halt(&self) {
        self.signal.lock().run = None;
        self.wake.notify_all();
        if let Some(stop) = self.stopper.lock().take() {
            stop();
        }
    }

    fn join(&self) {
        let thread = self.thread.lock().take();
        if let Some(thread) = thread
            && thread.join().is_err()
        {
            log::warn!("The content index thread panicked");
        }
    }

    pub(super) fn cache_bytes(&self) -> u64 {
        self.cache.load(Ordering::Relaxed)
    }

    fn measure(&self) {
        let entries = self.folder.as_deref().and_then(|folder| std::fs::read_dir(folder).ok());
        let bytes = entries.map_or(0, |entries| {
            entries.flatten().filter_map(|entry| entry.metadata().ok()).map(|metadata| metadata.len()).sum()
        });
        self.cache.store(bytes, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) fn read_documents(spawn: &Spawn, paths: &[String]) -> Vec<(String, String, String)> {
    let stopper = Mutex::new(None);
    let jobs = paths
        .iter()
        .map(|path| Job { candidate: Candidate { path: path.clone(), size: 0, modified: 0 }, replaces: None });
    let mut read = Vec::new();
    let mut sink = |step: Step| {
        if let Step::Done(job, outcome, text) = step {
            read.push((job.candidate.path, format!("{outcome:?}"), text));
        }
        Ok(())
    };
    Extractor::new(spawn, &stopper).run(jobs.collect(), &|| true, &mut sink).expect("the extractor ran");
    read.sort();
    read
}

fn document(lower_name: &str) -> bool {
    Format::of(lower_name).is_some()
}

fn half_pool() -> Option<ThreadPool> {
    let threads = std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).max(1));
    match rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|_| "content".into()).build() {
        Ok(pool) => Some(pool),
        Err(error) => {
            log::warn!("Couldn't start the content index threads: {error}");
            None
        }
    }
}

fn in_pool<T: Send>(pool: Option<&ThreadPool>, work: impl FnOnce() -> T + Send) -> T {
    match pool {
        Some(pool) => pool.install(work),
        None => work(),
    }
}

fn job_order(job: &Job) -> (bool, std::cmp::Reverse<u32>) {
    let lower = crate::search::fold::lowered(&job.candidate.path);
    let name = lower.rsplit('\\').next().unwrap_or_default();
    (!Format::of(name).is_some_and(Format::is_document), std::cmp::Reverse(job.candidate.modified))
}

fn jobs_of(plan: Plan) -> Vec<Job> {
    let added = plan.add.into_iter().map(|candidate| Job { candidate, replaces: None });
    let updated = plan.update.into_iter().map(|(doc, candidate)| Job { candidate, replaces: Some(doc) });
    let mut jobs: Vec<Job> = added.chain(updated).collect();
    jobs.sort_by_cached_key(job_order);
    jobs
}

fn estimate(candidates: &[Candidate]) -> ContentEstimate {
    let (mut text, mut seconds) = (0.0, 0.0);
    for candidate in candidates {
        let size = candidate.size as f64;
        let lower = crate::search::fold::lowered(&candidate.path);
        match lower.rsplit('\\').next().and_then(Format::of) {
            Some(Format::Pdf) => {
                text += size * PDF_TEXT_SHARE;
                seconds += size / PDF_BYTES_PER_SECOND;
            }
            Some(Format::Office | Format::OpenDocument) => {
                text += size * OFFICE_TEXT_SHARE;
                seconds += SECONDS_PER_OFFICE_FILE;
            }
            Some(Format::Text) | None => {
                text += size;
                seconds += SECONDS_PER_TEXT_FILE + size / TEXT_BYTES_PER_SECOND;
            }
        }
    }
    let threads = std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).max(1));
    let fastest = (seconds / threads as f64 + text / BUILD_BYTES_PER_SECOND).ceil() as u64;
    ContentEstimate {
        files: candidates.len(),
        text: text as u64,
        cache: (text * CACHE_SHARE) as u64,
        seconds: (fastest, fastest * SLOW_MARGIN),
    }
}

struct Feed<'a> {
    index: &'a Index,
    pool: Option<&'a ThreadPool>,
    total: usize,
    done: usize,
    batch: Vec<Extracted>,
    bytes: usize,
    started: Instant,
    published: Instant,
    progressed: Instant,
    saved: Instant,
}

impl<'a> Feed<'a> {
    fn new(index: &'a Index, pool: Option<&'a ThreadPool>, total: usize) -> Self {
        let started = Instant::now();
        Self {
            index,
            pool,
            total,
            done: 0,
            batch: Vec::new(),
            bytes: 0,
            started,
            published: started,
            progressed: started,
            saved: started,
        }
    }

    fn take(&mut self, step: Step) -> Result<()> {
        if let Step::Done(Job { candidate, replaces }, outcome, text) = step {
            match outcome {
                Outcome::Text => {}
                Outcome::Empty | Outcome::Unsupported | Outcome::Encrypted | Outcome::Denied | Outcome::Failed => {
                    log::info!("{}: no text read ({outcome:?})", candidate.path);
                }
            }
            self.bytes += text.len();
            self.done += 1;
            self.batch.push(Extracted { candidate, replaces, outcome, text });
        }
        let due = self.published.elapsed() >= PUBLISH_INTERVAL;
        if !self.batch.is_empty() && (self.bytes >= BATCH_BYTES || due) {
            self.flush()?;
            if due {
                self.published = Instant::now();
                self.index.notify(Update::Contents);
            }
        }
        self.report();
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        let library = &self.index.content.library;
        let batch = std::mem::take(&mut self.batch);
        self.bytes = 0;
        in_pool(self.pool, || {
            library::ingest(library, batch)?;
            library::settle(library, false)
        })?;
        if self.saved.elapsed() >= SAVE_INTERVAL {
            self.saved = Instant::now();
            in_pool(self.pool, || library::save(library))?;
        }
        self.index.content.measure();
        Ok(())
    }

    fn report(&mut self) {
        if self.total < SHOWN_QUEUE {
            return;
        }
        let Self { done, total, .. } = *self;
        let left = (done > 0).then(|| {
            let spent = self.started.elapsed().as_secs_f64() / done as f64;
            (spent * (total - done) as f64).ceil() as u64
        });
        *self.index.content.status.lock() = ContentStatus::Building { done, total, left };
        if self.progressed.elapsed() >= PROGRESS_INTERVAL {
            self.progressed = Instant::now();
            self.index.notify(Update::Count);
        }
    }
}

impl Index {
    pub(crate) fn extracting(mut self, folder: PathBuf, spawn: Spawn) -> Self {
        self.content = Content { folder: Some(folder), spawn: Some(spawn), ..Content::default() };
        self.content.measure();
        self
    }

    pub(crate) fn full_text(self: &Arc<Self>, on: bool) {
        if self.content.enabled() == on {
            return;
        }
        if !on {
            self.content.halt();
            return;
        }
        let (Some(folder), Some(spawn)) = (self.content.folder.clone(), self.content.spawn.clone()) else {
            return;
        };
        let run = {
            let mut signal = self.content.signal.lock();
            signal.runs += 1;
            signal.run = Some(signal.runs);
            signal.touched = true;
            signal.runs
        };
        let previous = self.content.thread.lock().take();
        let index = self.clone();
        let spawned = std::thread::Builder::new().name("content".into()).spawn(move || {
            if let Some(previous) = previous {
                let _ = previous.join();
            }
            index.run_content(run, &folder, &spawn);
        });
        match spawned {
            Ok(thread) => *self.content.thread.lock() = Some(thread),
            Err(error) => {
                log::warn!("Couldn't start the content index: {error}");
                self.content.signal.lock().run = None;
            }
        }
    }

    pub(crate) fn close_content(&self) {
        self.content.halt();
        self.content.join();
    }

    pub(crate) fn delete_content(self: &Arc<Self>) {
        let index = self.clone();
        let spawned = std::thread::Builder::new().name("content-delete".into()).spawn(move || {
            let enabled = index.content.enabled();
            index.close_content();
            if let Some(folder) = index.content.folder.as_deref().filter(|folder| folder.exists())
                && let Err(error) = std::fs::remove_dir_all(folder)
            {
                log::warn!("Couldn't delete the content cache: {error}");
            }
            index.content.measure();
            if enabled {
                index.full_text(true);
            }
            index.notify(Update::Contents);
        });
        if let Err(error) = spawned {
            log::warn!("Couldn't delete the content cache: {error}");
        }
    }

    pub(crate) fn content_estimate(&self) -> Option<ContentEstimate> {
        self.content_candidates().map(|candidates| estimate(&candidates))
    }

    fn notify(&self, update: Update) {
        let changed = self.launch.lock().as_ref().map(|launch| launch.changed.clone());
        if let Some(changed) = changed {
            changed(update);
        }
    }

    fn content_candidates(&self) -> Option<Vec<Candidate>> {
        let generation = self.generation.load(Ordering::Relaxed);
        if self.scanning.load(Ordering::Relaxed) > 0 {
            return None;
        }
        let volumes = self.volumes.read().clone();
        if volumes.is_empty() || !volumes.iter().all(|volume| volume.read().details_complete()) {
            return None;
        }
        let excluded = self.ranking.read().excluded.clone();
        let mut candidates = Vec::new();
        for volume in &volumes {
            let volume = volume.read();
            let wanted =
                |lower: &str, size: u64| Format::of(lower).is_some_and(|format| size > 0 && size <= format.max_file());
            volume.documents(&CONTENT_NOISE, wanted, |index, meta| {
                candidates.push(Candidate { path: volume.path(index), size: meta.size, modified: meta.modified });
            });
        }
        if !excluded.is_empty() {
            candidates.retain(|candidate| {
                let lower = crate::search::fold::lowered(&candidate.path);
                !excluded.iter().any(|folder| contains(folder, &lower))
            });
        }
        (self.generation.load(Ordering::Relaxed) == generation).then_some(candidates)
    }

    fn publish_ready(&self) {
        self.content.measure();
        let guard = self.content.library.read();
        if let Some(library) = guard.as_ref() {
            let (files, unread) = library.counts();
            *self.content.status.lock() = ContentStatus::Ready { files, built: library.docs.built, unread };
        }
    }

    fn feed(&self, jobs: Vec<Job>, run: u64, spawn: &Spawn, pool: Option<&ThreadPool>) -> Result<()> {
        let mut feed = Feed::new(self, pool, jobs.len());
        let mut extractor = Extractor::new(spawn, &self.content.stopper);
        extractor.run(jobs, &|| self.content.alive(run), &mut |step| feed.take(step))?;
        feed.flush()
    }

    fn sweep(&self, candidates: Vec<Candidate>, run: u64, spawn: &Spawn) -> Result<bool> {
        let started = Instant::now();
        let library = &self.content.library;
        let plan = match library.read().as_ref() {
            Some(found) => reconcile(&found.docs, candidates),
            None => return Ok(false),
        };
        if plan.is_empty() {
            self.publish_ready();
            self.notify(Update::Count);
            return Ok(false);
        }
        if let Some(found) = library.write().as_mut() {
            found.docs.apply(&plan);
        }
        let jobs = jobs_of(plan);
        if jobs.len() >= SHOWN_QUEUE {
            *self.content.status.lock() = ContentStatus::Building { done: 0, total: jobs.len(), left: None };
            self.notify(Update::Count);
        }
        if !jobs.is_empty() {
            let pool = half_pool();
            self.feed(jobs, run, spawn, pool.as_ref())?;
            in_pool(pool.as_ref(), || library::settle(library, true))?;
            if self.content.alive(run) {
                let first = library.write().as_mut().is_some_and(|found| {
                    let first = found.docs.built == 0;
                    if first {
                        found.docs.built = started.elapsed().as_secs().max(1);
                    }
                    first
                });
                if first {
                    in_pool(pool.as_ref(), || library::save(library))?;
                }
            }
            let compacting = library.read().as_ref().is_some_and(Library::needs_compaction);
            if compacting {
                log::info!("Compacting the content cache");
                let alive = || self.content.alive(run);
                in_pool(pool.as_ref(), || library::compact(library, &alive))?;
            }
        }
        self.publish_ready();
        self.notify(Update::Contents);
        Ok(true)
    }

    fn open_library(&self, run: u64, folder: &Path) -> bool {
        let pool = half_pool();
        let alive = || self.content.alive(run);
        let opened = in_pool(pool.as_ref(), || Library::open_or_create(folder, &alive));
        match opened {
            Ok(Some(library)) => {
                *self.content.library.write() = Some(library);
                true
            }
            Ok(None) => false,
            Err(error) => {
                log::warn!("Couldn't open the content cache: {error:#}");
                false
            }
        }
    }

    fn run_content(&self, run: u64, folder: &Path, spawn: &Spawn) {
        *self.content.status.lock() = ContentStatus::Building { done: 0, total: 0, left: None };
        self.notify(Update::Count);
        if self.open_library(run, folder) {
            self.publish_ready();
            self.notify(Update::Contents);
            self.watch_content(run, spawn);
            let library = &self.content.library;
            let pool = half_pool();
            if let Err(error) = in_pool(pool.as_ref(), || library::save(library)) {
                log::warn!("Couldn't save the content cache: {error:#}");
            }
        }
        *self.content.library.write() = None;
        self.content.measure();
        *self.content.status.lock() = ContentStatus::Off;
        self.notify(Update::Contents);
    }

    fn watch_content(&self, run: u64, spawn: &Spawn) {
        let mut saved = Instant::now();
        let mut unsaved = false;
        while self.content.alive(run) {
            if self.content.wait_for_touch(run) && self.content.alive(run) {
                let Some(candidates) = self.content_candidates() else {
                    self.content.touch();
                    self.content.pause(run, SWEEP_INTERVAL);
                    continue;
                };
                match self.sweep(candidates, run, spawn) {
                    Ok(changed) => unsaved |= changed,
                    Err(error) => {
                        log::warn!("Indexing file contents failed: {error:#}");
                        self.content.pause(run, RETRY_INTERVAL);
                    }
                }
                self.content.pause(run, SWEEP_INTERVAL);
            }
            if unsaved && saved.elapsed() >= SAVE_INTERVAL {
                let library = &self.content.library;
                match library::save(library) {
                    Ok(()) => (saved, unsaved) = (Instant::now(), false),
                    Err(error) => log::warn!("Couldn't save the content cache: {error:#}"),
                }
            }
        }
    }
}
