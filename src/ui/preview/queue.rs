use super::load::Loaded;
use crate::index::Hit;

pub(super) const KEPT: usize = 10;
const BUSY: usize = 4;
const SPARE: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub(super) path: String,
    pub(super) pixels: u32,
    size: Option<u64>,
    modified: Option<i64>,
}

impl Key {
    pub(super) fn of(hit: &Hit, pixels: u32) -> Self {
        Self { path: hit.path.clone(), pixels, size: hit.size, modified: hit.modified }
    }
}

#[derive(Default)]
pub(super) struct Kept(Vec<(Key, Loaded)>);

impl Kept {
    pub(super) fn get(&self, key: &Key) -> Option<&Loaded> {
        self.0.iter().find(|(known, _)| known == key).map(|(_, loaded)| loaded)
    }

    pub(super) fn keep(&mut self, key: Key, loaded: Loaded) {
        self.0.retain(|(known, _)| *known != key);
        if self.0.len() >= KEPT {
            self.0.remove(0);
        }
        self.0.push((key, loaded));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need {
    Shown,
    Ahead,
}

impl Need {
    fn room(self) -> usize {
        match self {
            Self::Shown => BUSY,
            Self::Ahead => SPARE,
        }
    }
}

pub(super) struct Job {
    pub(super) key: Key,
    pub(super) hit: Hit,
    pub(super) ratio: f32,
    pub(super) need: Need,
}

#[derive(Default)]
pub(super) struct Queue {
    running: Vec<Key>,
    waiting: Vec<Job>,
}

impl Queue {
    pub(super) fn want(&mut self, jobs: Vec<Job>) {
        self.waiting = jobs;
        self.waiting.retain(|job| !self.running.contains(&job.key));
    }

    pub(super) fn next(&mut self) -> Option<Job> {
        let need = self.waiting.first()?.need;
        if self.running.len() >= need.room() {
            return None;
        }
        let job = self.waiting.remove(0);
        self.running.push(job.key.clone());
        Some(job)
    }

    pub(super) fn done(&mut self, key: &Key) {
        self.running.retain(|running| running != key);
    }
}
