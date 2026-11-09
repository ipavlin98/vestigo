use super::volume::{Checkpoint, Volume};
use super::{Changed, Drive, Index, Job, Update, cache, details, ntfs, walk};
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const SAVE_SHARE: i64 = 2;
const RESCAN_PAUSE: Duration = Duration::from_secs(60);
const START_MENU_ROOTS: [&str; 2] = ["ProgramData", "APPDATA"];
const START_MENU_PROGRAMS: &str = "Microsoft\\Windows\\Start Menu\\Programs";

impl Index {
    pub(super) fn index_drive(self: &Arc<Self>, drive: Drive, changed: Changed, job: Job) {
        let started = Instant::now();
        if drive.journal {
            match self.index_ntfs(drive.letter, started, &changed, job) {
                Ok(()) => return,
                Err(error) => log::warn!("Reading the MFT of {}: failed ({error:#}), walking instead", drive.letter),
            }
        }
        let root = format!("{}:\\", drive.letter);
        let volume = self.add(walk::scan(Volume::new(drive.letter), Path::new(&root)), job);
        self.finish_scan(drive.letter, started, &*changed);
        if let Some(volume) = volume {
            self.read_details(volume, changed, job);
        }
    }

    fn read_details(self: &Arc<Self>, volume: Arc<RwLock<Volume>>, changed: Changed, job: Job) {
        if volume.read().details_complete() {
            return;
        }
        let index = self.clone();
        let letter = volume.read().letter();
        let spawned = std::thread::Builder::new().name(format!("details-{letter}")).spawn(move || {
            if details::fill(&volume) && index.current(job) {
                index.save_shared(&volume);
                changed(Update::Contents);
            }
        });
        if let Err(error) = spawned {
            log::warn!("Couldn't start reading sizes and dates of {letter}: {error}");
        }
    }

    fn index_ntfs(self: &Arc<Self>, letter: char, started: Instant, changed: &Changed, job: Job) -> anyhow::Result<()> {
        let journal = ntfs::Journal::open(letter)?;
        let cached = self.cache_file(letter).filter(|_| !job.fresh).and_then(|path| match cache::load(&path, letter) {
            Ok(volume) => Some(volume),
            Err(error) => {
                log::info!("{letter}: no usable index cache ({error:#})");
                None
            }
        });
        let mut volume = match cached.filter(|volume| volume.checkpoint().is_some_and(|point| journal.resumes(point))) {
            Some(volume) => volume,
            None => {
                let volume = journal.scan()?;
                if self.current(job) {
                    self.save_volume(&volume);
                }
                volume
            }
        };
        let start_menus = start_menus(letter);
        volume.fill(&start_menus);
        let Some(checkpoint) = volume.checkpoint() else { anyhow::bail!("the index has no journal position") };
        let Some(volume) = self.add(volume, job) else {
            self.scanning.fetch_sub(1, Ordering::Relaxed);
            return Ok(());
        };
        self.finish_scan(letter, started, &**changed);
        self.read_details(volume.clone(), changed.clone(), job);
        self.follow(journal, checkpoint, &volume, &start_menus, changed, job);
        Ok(())
    }

    fn follow(
        self: &Arc<Self>,
        mut journal: ntfs::Journal,
        mut checkpoint: Checkpoint,
        volume: &Arc<RwLock<Volume>>,
        start_menus: &[PathBuf],
        changed: &Changed,
        job: Job,
    ) {
        let letter = volume.read().letter();
        let mut rescanned: Option<Instant> = None;
        loop {
            let Err(error) = self.watch(journal, checkpoint, volume, start_menus, changed, job) else { return };
            if !self.current(job) {
                return;
            }
            log::warn!("Watching changes on {letter}: lost the journal ({error:#}), reading the MFT again");
            if let Some(at) = rescanned {
                std::thread::sleep(RESCAN_PAUSE.saturating_sub(at.elapsed()));
            }
            rescanned = Some(Instant::now());
            let scanned = ntfs::Journal::open(letter).and_then(|journal| Ok((journal.scan()?, journal)));
            let (mut scanned, reopened) = match scanned {
                Ok(scanned) => scanned,
                Err(error) => {
                    log::warn!("Watching changes on {letter}: stopped ({error:#})");
                    return;
                }
            };
            let Some(point) = scanned.checkpoint() else { return };
            if !self.current(job) {
                return;
            }
            scanned.fill(start_menus);
            *volume.write() = scanned;
            self.save_shared(volume);
            self.content.touch();
            changed(Update::Contents);
            self.read_details(volume.clone(), changed.clone(), job);
            (journal, checkpoint) = (reopened, point);
        }
    }

    fn watch(
        &self,
        journal: ntfs::Journal,
        checkpoint: Checkpoint,
        volume: &RwLock<Volume>,
        start_menus: &[PathBuf],
        changed: &Changed,
        job: Job,
    ) -> anyhow::Result<()> {
        let due = journal.capacity() / SAVE_SHARE;
        let mut saved = checkpoint.usn;
        journal.watch(checkpoint, volume, |volume| {
            if !self.current(job) {
                return false;
            }
            details::refresh(volume);
            self.content.note(volume);
            let needs_compaction = volume.read().needs_compaction();
            if needs_compaction {
                let mut compacted = volume.read().compacted();
                compacted.fill(start_menus);
                *volume.write() = compacted;
                log::info!("{}: index compacted", volume.read().letter());
            }
            changed(Update::Contents);
            let reached = volume.read().checkpoint().map_or(saved, |point| point.usn);
            if reached - saved >= due {
                self.save_shared(volume);
                saved = reached;
            }
            true
        })
    }
}

pub(super) fn start_menus(letter: char) -> Vec<PathBuf> {
    START_MENU_ROOTS
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join(START_MENU_PROGRAMS))
        .filter(|folder| {
            folder
                .to_str()
                .and_then(|path| path.chars().next())
                .is_some_and(|drive| drive.eq_ignore_ascii_case(&letter))
        })
        .collect()
}
