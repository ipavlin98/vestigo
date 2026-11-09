use super::volume::{Family, Meta, Volume, describe};
use parking_lot::RwLock;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::time::Instant;

const CHUNK: usize = 1024;
const LOOKUP_MIN: usize = 32;
const ATTEMPTS: usize = 3;

type Found = (u32, Meta, bool);

pub(super) fn fill(volume: &RwLock<Volume>) -> bool {
    let threads = std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).max(1));
    match rayon::ThreadPoolBuilder::new().num_threads(threads).build() {
        Ok(pool) => pool.install(|| fill_in(volume)),
        Err(error) => {
            log::warn!("Reading sizes and dates without a thread pool: {error}");
            fill_in(volume)
        }
    }
}

fn fill_in(volume: &RwLock<Volume>) -> bool {
    let started = Instant::now();
    for _ in 0..ATTEMPTS {
        let family = volume.read().family();
        if read_all(volume, &family) {
            let mut volume = volume.write();
            volume.complete_details();
            log::info!("{}: sizes and dates read in {:?}", volume.letter(), started.elapsed());
            return true;
        }
    }
    false
}

struct Folder {
    path: String,
    children: Vec<(String, u32)>,
}

fn read_all(volume: &RwLock<Volume>, family: &Family) -> bool {
    for chunk in family.dirs().chunks(CHUNK) {
        let folders: Vec<Folder> = {
            let volume = volume.read();
            if volume.serial() != family.serial() {
                return false;
            }
            chunk.iter().map(|&dir| folder(&volume, family, dir)).collect()
        };
        let found: Vec<Found> = folders.par_iter().flat_map_iter(read_folder).collect();
        let mut volume = volume.write();
        if volume.serial() != family.serial() {
            return false;
        }
        for (index, meta, hidden) in found {
            volume.set_meta(index, meta, hidden);
        }
    }
    true
}

fn folder(volume: &Volume, family: &Family, dir: u32) -> Folder {
    let children = family.children(dir).iter().map(|&child| (volume.name(child).to_owned(), child)).collect();
    Folder { path: volume.path(dir), children }
}

fn read_folder(folder: &Folder) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(&folder.path) else { return Vec::new() };
    let lookup: Option<FxHashMap<&str, u32>> = (folder.children.len() > LOOKUP_MIN)
        .then(|| folder.children.iter().map(|(name, child)| (name.as_str(), *child)).collect());
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let child = match &lookup {
                Some(lookup) => lookup.get(name).copied(),
                None => folder.children.iter().find(|(known, _)| known == name).map(|&(_, child)| child),
            }?;
            let (meta, hidden) = describe(&entry.metadata().ok()?);
            Some((child, meta, hidden))
        })
        .collect()
}

pub(super) fn refresh(volume: &RwLock<Volume>) {
    let (serial, stale) = {
        let mut volume = volume.write();
        (volume.serial(), volume.take_stale())
    };
    if stale.is_empty() {
        return;
    }
    let paths: Vec<(u32, String)> = {
        let volume = volume.read();
        stale.iter().map(|&index| (index, volume.path(index))).collect()
    };
    let found: Vec<Found> = paths
        .into_iter()
        .filter_map(|(index, path)| {
            let (meta, hidden) = describe(&std::fs::symlink_metadata(path).ok()?);
            Some((index, meta, hidden))
        })
        .collect();
    let mut volume = volume.write();
    if volume.serial() == serial {
        for (index, meta, hidden) in found {
            volume.set_meta(index, meta, hidden);
        }
    }
}
