use super::volume::Volume;
use super::{Changed, Index, Job, Update, details, walk};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderStatus {
    pub(crate) path: String,
    pub(crate) entries: usize,
    pub(crate) walked: Option<SystemTime>,
    pub(crate) walking: bool,
}

pub(super) enum Wake {
    Now,
    Reschedule,
}

pub(super) struct Watched {
    pub(super) status: FolderStatus,
    pub(super) wake: Sender<Wake>,
}

impl Index {
    pub(crate) fn rewalk_every(&self, every: Option<Duration>) {
        *self.rewalk.lock() = every;
        for watched in self.watched.lock().iter() {
            let _ = watched.wake.send(Wake::Reschedule);
        }
    }

    pub(crate) fn refresh_folder(&self, path: &str) {
        if let Some(watched) = self.watched.lock().iter().find(|watched| watched.status.path == path) {
            let _ = watched.wake.send(Wake::Now);
        }
    }

    pub(crate) fn folders(&self) -> Vec<FolderStatus> {
        self.watched.lock().iter().map(|watched| watched.status.clone()).collect()
    }

    pub(super) fn index_folder(self: &Arc<Self>, folder: &str, woken: &Receiver<Wake>, changed: &Changed, job: Job) {
        let mut first = true;
        loop {
            let started = Instant::now();
            let volume = walk::scan(Volume::folder(folder), Path::new(folder));
            let entries = volume.len();
            let added = self.replace(volume, job);
            if first {
                log::info!("Indexed {folder} in {:?}", started.elapsed());
                self.scanning.fetch_sub(1, Ordering::Relaxed);
                first = false;
            }
            self.note(folder, job, |status| {
                status.entries = entries;
                status.walked = Some(SystemTime::now());
                status.walking = false;
            });
            changed(Update::Contents);
            let Some(volume) = added else { return };
            let walked = Instant::now();
            if !volume.read().details_complete() {
                details::fill(&volume);
                changed(Update::Contents);
            }
            self.content.touch();
            drop(volume);
            if !self.wait(woken, walked) || !self.current(job) {
                return;
            }
            self.note(folder, job, |status| status.walking = true);
            changed(Update::Contents);
        }
    }

    fn wait(&self, woken: &Receiver<Wake>, walked: Instant) -> bool {
        loop {
            let every = *self.rewalk.lock();
            let message = match every.map(|every| every.saturating_sub(walked.elapsed())) {
                Some(left) if left.is_zero() => return true,
                Some(left) => woken.recv_timeout(left),
                None => woken.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match message {
                Ok(Wake::Now) | Err(RecvTimeoutError::Timeout) => return true,
                Ok(Wake::Reschedule) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    fn note(&self, folder: &str, job: Job, edit: impl FnOnce(&mut FolderStatus)) {
        let mut watched = self.watched.lock();
        if !self.current(job) {
            return;
        }
        if let Some(watched) = watched.iter_mut().find(|watched| watched.status.path == folder) {
            edit(&mut watched.status);
        }
    }
}
