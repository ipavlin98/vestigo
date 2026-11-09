use super::docs::{Block, Candidate, Commit, Docs, Outcome, Unread};
use super::grams::{Grams, keys_of};
use super::store::{Store, compress, spans, with_scratch};
use anyhow::{Context, Result};
use parking_lot::RwLock;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

const DOCS: &str = "docs.bin";
const BLOCKS: &str = "blocks.bin";
const COMPACT: &str = "compact";
const GRAMS: &str = "grams.bin";
const COMPACT_MIN_WASTE: u64 = 8 << 20;
const COMPACT_SHARE: u64 = 4;
const SPILL_POSTINGS: usize = 8_000_000;
const MAX_RUNS: usize = 16;
const REBUILD_CHUNK: usize = 1024;

pub(super) struct Library {
    folder: PathBuf,
    pub(super) docs: Docs,
    pub(super) grams: Grams,
    pub(super) store: Store,
    epoch: u64,
    commit: u64,
}

pub(super) struct Extracted {
    pub(super) candidate: Candidate,
    pub(super) replaces: Option<u32>,
    pub(super) outcome: Outcome,
    pub(super) text: String,
}

struct Packed {
    frame: Vec<u8>,
    keys: Vec<u32>,
    text: u32,
    own: u32,
}

fn pack(text: &str) -> Result<Vec<Packed>> {
    spans(text)
        .map(|span| {
            let stored = text.get(span.start..span.end).context("a block cut a character")?;
            let frame = compress(stored.as_bytes())?;
            let keys = with_scratch(|scratch| {
                scratch.raw.clear();
                scratch.raw.extend_from_slice(stored.as_bytes());
                Ok(block_keys(scratch))
            })?;
            Ok(Packed { frame, keys, text: stored.len() as u32, own: (span.own_end - span.start) as u32 })
        })
        .collect()
}

fn block_keys(scratch: &mut super::store::Scratch) -> Vec<u32> {
    let mut keys = std::mem::take(&mut scratch.keys);
    keys_of(std::str::from_utf8(scratch.folded()).unwrap_or_default(), &mut keys);
    let exact = keys.as_slice().to_vec();
    scratch.keys = keys;
    exact
}

fn rebuild(docs: &Docs, store: &Store, grams: &mut Grams, alive: &(dyn Fn() -> bool + Sync)) -> Result<bool> {
    let live: Vec<u32> = (0..docs.blocks.len() as u32).filter(|&block| docs.doc_of(block).is_some()).collect();
    for chunk in live.chunks(REBUILD_CHUNK) {
        if !alive() {
            return Ok(false);
        }
        let lists: Vec<(u32, Vec<u32>)> = chunk
            .par_iter()
            .filter_map(|&id| docs.block(id).map(|block| (id, block)))
            .map(|(id, block)| {
                with_scratch(|scratch| {
                    store.read(block, scratch)?;
                    Ok((id, block_keys(scratch)))
                })
            })
            .collect::<Result<_>>()?;
        grams.add(&lists);
        if grams.postings() >= SPILL_POSTINGS {
            grams.spill()?;
        }
    }
    if grams.postings() > 0 {
        grams.spill()?;
    }
    grams.merge()?;
    Ok(true)
}

impl Library {
    fn create(folder: &Path) -> Result<Self> {
        match std::fs::remove_dir_all(folder) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("clearing {}", folder.display())),
        }
        std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
        let epoch = 1;
        Ok(Self {
            folder: folder.to_owned(),
            docs: Docs::default(),
            grams: Grams::empty(folder),
            store: Store::create(&folder.join(BLOCKS), epoch)?,
            epoch,
            commit: 0,
        })
    }

    fn open(folder: &Path, alive: &(dyn Fn() -> bool + Sync)) -> Result<Option<Self>> {
        Grams::clear_leftovers(folder);
        let unfinished = folder.join(COMPACT);
        if unfinished.exists() {
            std::fs::remove_dir_all(&unfinished).context("clearing an unfinished compaction")?;
        }
        let (docs, commit) = Docs::load(&folder.join(DOCS))?;
        let Commit { number, epoch, generation, store } = commit;
        let store = Store::open(&folder.join(BLOCKS), epoch, store)?;
        let grams = match Grams::load(folder, generation, number) {
            Ok(grams) => grams,
            Err(error) => {
                log::info!("Rebuilding the gram index from the stored text ({error:#})");
                let mut grams = Grams::empty(folder);
                if !rebuild(&docs, &store, &mut grams, alive)? {
                    return Ok(None);
                }
                grams
            }
        };
        Ok(Some(Self { folder: folder.to_owned(), docs, grams, store, epoch, commit: number }))
    }

    pub(super) fn open_or_create(folder: &Path, alive: &(dyn Fn() -> bool + Sync)) -> Result<Option<Self>> {
        if folder.join(DOCS).exists() {
            match Self::open(folder, alive) {
                Ok(found) => return Ok(found),
                Err(error) => log::warn!("The content cache can't be used, indexing anew: {error:#}"),
            }
        }
        Self::create(folder).map(Some)
    }

    pub(super) fn needs_compaction(&self) -> bool {
        self.docs.waste >= COMPACT_MIN_WASTE && self.docs.waste * COMPACT_SHARE >= self.store.len()
    }

    pub(super) fn counts(&self) -> (usize, Unread) {
        self.docs.counts()
    }

    fn save(&self, commit: u64) -> Result<()> {
        self.store.sync()?;
        self.grams.save_delta(commit)?;
        let stamp =
            Commit { number: commit, epoch: self.epoch, generation: self.grams.generation(), store: self.store.len() };
        self.docs.save(&self.folder.join(DOCS), &stamp)
    }
}

pub(super) fn ingest(library: &RwLock<Option<Library>>, batch: Vec<Extracted>) -> Result<()> {
    let packed: Vec<Vec<Packed>> = batch
        .par_iter()
        .map(|extracted| match extracted.outcome {
            Outcome::Text => pack(&extracted.text),
            Outcome::Empty | Outcome::Unsupported | Outcome::Encrypted | Outcome::Denied | Outcome::Failed => {
                Ok(Vec::new())
            }
        })
        .collect::<Result<_>>()?;
    let mut placed = Vec::with_capacity(batch.len());
    {
        let guard = library.read();
        let Some(library) = guard.as_ref() else { return Ok(()) };
        let mut next = library.docs.blocks.len() as u32;
        let mut lists = Vec::new();
        for blocks in packed {
            let mut row = Vec::with_capacity(blocks.len());
            for Packed { frame, keys, text, own } in blocks {
                let offset = library.store.append(&frame)?;
                row.push(Block { offset, frame: frame.len() as u32, text, own, doc: 0 });
                lists.push((next, keys));
                next += 1;
            }
            placed.push(row);
        }
        library.grams.add(&lists);
    }
    let mut guard = library.write();
    let Some(library) = guard.as_mut() else { return Ok(()) };
    for (extracted, blocks) in batch.into_iter().zip(placed) {
        let Extracted { candidate, replaces, outcome, .. } = extracted;
        if let Some(old) = replaces {
            library.docs.retire(old);
        }
        library.docs.add(candidate, outcome, blocks);
    }
    Ok(())
}

pub(super) fn settle(library: &RwLock<Option<Library>>, finished: bool) -> Result<()> {
    let spill = {
        let guard = library.read();
        let Some(found) = guard.as_ref() else { return Ok(()) };
        let grams = &found.grams;
        let settling = finished && (grams.runs() > 0 || grams.generation() == 0);
        (grams.postings() >= SPILL_POSTINGS || settling && grams.postings() > 0).then(|| grams.spilled()).transpose()?
    };
    if let Some(run) = spill
        && let Some(found) = library.write().as_mut()
    {
        found.grams.attach_pending(run);
    }
    let merged = {
        let guard = library.read();
        let Some(found) = guard.as_ref() else { return Ok(()) };
        let grams = &found.grams;
        (grams.runs() >= MAX_RUNS || finished && grams.runs() > 0).then(|| grams.merged()).transpose()?
    };
    if let Some(merged) = merged
        && let Some(found) = library.write().as_mut()
    {
        found.grams.install(merged)?;
    }
    Ok(())
}

pub(super) fn save(library: &RwLock<Option<Library>>) -> Result<()> {
    settle(library, true)?;
    let guard = library.read();
    let Some(found) = guard.as_ref() else { return Ok(()) };
    let commit = found.commit + 1;
    found.save(commit)?;
    drop(guard);
    if let Some(found) = library.write().as_mut() {
        found.commit = commit;
    }
    Ok(())
}

pub(super) fn compact(library: &RwLock<Option<Library>>, alive: &(dyn Fn() -> bool + Sync)) -> Result<()> {
    let guard = library.read();
    let Some(old) = guard.as_ref() else { return Ok(()) };
    let folder = old.folder.clone();
    let spare = folder.join(COMPACT);
    if spare.exists() {
        std::fs::remove_dir_all(&spare).context("clearing an unfinished compaction")?;
    }
    std::fs::create_dir_all(&spare).context("preparing the compaction")?;
    let epoch = old.epoch + 1;
    let store = Store::create(&spare.join(BLOCKS), epoch)?;
    let mut docs = Docs { built: old.docs.built, ..Docs::default() };
    let mut frame = Vec::new();
    for doc in old.docs.docs.iter().filter(|doc| doc.live()) {
        let mut blocks = Vec::with_capacity(doc.blocks.len());
        for id in doc.blocks.clone() {
            let block = old.docs.block(id).context("a document points past the block table")?;
            old.store.frame(block, &mut frame)?;
            blocks.push(Block { offset: store.append(&frame)?, ..*block });
        }
        let candidate = Candidate { path: doc.path.clone(), size: doc.size, modified: doc.modified };
        docs.add(candidate, doc.outcome, blocks);
    }
    let mut grams = Grams::empty(&spare);
    if !rebuild(&docs, &store, &mut grams, alive)? {
        drop(guard);
        std::fs::remove_dir_all(&spare).context("clearing a stopped compaction")?;
        return Ok(());
    }
    let (generation, length, commit) = (grams.generation(), store.len(), old.commit);
    store.sync()?;
    drop((grams, store, guard));
    let mut guard = library.write();
    *guard = None;
    std::fs::rename(spare.join(BLOCKS), folder.join(BLOCKS)).context("replacing the text store")?;
    std::fs::rename(spare.join(GRAMS), folder.join(GRAMS)).context("replacing the gram index")?;
    std::fs::remove_dir_all(&spare).context("clearing the compaction")?;
    *guard = Some(Library {
        docs,
        grams: Grams::reopened(&folder, generation)?,
        store: Store::open(&folder.join(BLOCKS), epoch, length)?,
        folder,
        epoch,
        commit,
    });
    drop(guard);
    save(library)
}
