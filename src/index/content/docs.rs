use crate::index::cache::Fields;
use anyhow::{Context, Result, bail, ensure};
use rustc_hash::FxHashMap;
use std::io::{BufWriter, Write};
use std::ops::Range;
use std::path::Path;

const MAGIC: &[u8; 8] = b"VSTDOCS2";
const HEADER_BYTES: usize = 8 + 8 * 9;
const DOC_BYTES: usize = 8 + 4 + 1 + 4 + 4 + 4;
const BLOCK_BYTES: usize = 8 + 4 + 4 + 4 + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Text,
    Empty,
    Unsupported,
    Encrypted,
    Denied,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Unread {
    pub(crate) denied: usize,
    pub(crate) empty: usize,
    pub(crate) binary: usize,
    pub(crate) encrypted: usize,
    pub(crate) failed: usize,
}

impl Outcome {
    pub(super) fn code(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Empty => 1,
            Self::Unsupported => 2,
            Self::Encrypted => 3,
            Self::Failed => 4,
            Self::Denied => 5,
        }
    }

    pub(super) fn of(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Text),
            1 => Some(Self::Empty),
            2 => Some(Self::Unsupported),
            3 => Some(Self::Encrypted),
            4 => Some(Self::Failed),
            5 => Some(Self::Denied),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) modified: u32,
}

impl Candidate {
    fn name(&self) -> &str {
        name_of(&self.path)
    }
}

fn name_of(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Doc {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) modified: u32,
    pub(super) outcome: Outcome,
    pub(super) blocks: Range<u32>,
}

impl Doc {
    pub(super) fn live(&self) -> bool {
        !self.path.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Block {
    pub(super) offset: u64,
    pub(super) frame: u32,
    pub(super) text: u32,
    pub(super) own: u32,
    pub(super) doc: u32,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    pub(super) add: Vec<Candidate>,
    pub(super) update: Vec<(u32, Candidate)>,
    pub(super) remove: Vec<u32>,
    pub(super) moved: Vec<(u32, String)>,
}

impl Plan {
    pub(super) fn is_empty(&self) -> bool {
        self.add.is_empty() && self.update.is_empty() && self.remove.is_empty() && self.moved.is_empty()
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Commit {
    pub(super) number: u64,
    pub(super) epoch: u64,
    pub(super) generation: u64,
    pub(super) store: u64,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Docs {
    pub(super) docs: Vec<Doc>,
    pub(super) blocks: Vec<Block>,
    pub(super) waste: u64,
    pub(super) built: u64,
}

impl Docs {
    pub(super) fn doc_of(&self, block: u32) -> Option<u32> {
        self.blocks.get(block as usize).map(|block| block.doc).filter(|&doc| self.get(doc).is_some_and(Doc::live))
    }

    pub(super) fn get(&self, doc: u32) -> Option<&Doc> {
        self.docs.get(doc as usize)
    }

    pub(super) fn block(&self, block: u32) -> Option<&Block> {
        self.blocks.get(block as usize)
    }

    pub(super) fn add(&mut self, candidate: Candidate, outcome: Outcome, blocks: Vec<Block>) -> u32 {
        let doc = self.docs.len() as u32;
        let first = self.blocks.len() as u32;
        let count = blocks.len() as u32;
        self.blocks.extend(blocks.into_iter().map(|block| Block { doc, ..block }));
        let Candidate { path, size, modified } = candidate;
        self.docs.push(Doc { path, size, modified, outcome, blocks: first..first + count });
        doc
    }

    pub(super) fn retire(&mut self, doc: u32) {
        let Some(found) = self.docs.get_mut(doc as usize).filter(|found| found.live()) else { return };
        found.path = String::new();
        let dead = self.blocks.get(found.blocks.start as usize..found.blocks.end as usize).unwrap_or_default();
        self.waste += dead.iter().map(|block| u64::from(block.frame)).sum::<u64>();
    }

    pub(super) fn apply(&mut self, plan: &Plan) {
        for (doc, path) in &plan.moved {
            if let Some(found) = self.docs.get_mut(*doc as usize) {
                found.path.clone_from(path);
            }
        }
        for &doc in &plan.remove {
            self.retire(doc);
        }
    }

    pub(super) fn counts(&self) -> (usize, Unread) {
        let (mut read, mut unread) = (0, Unread::default());
        for doc in self.docs.iter().filter(|doc| doc.live()) {
            match doc.outcome {
                Outcome::Text => read += 1,
                Outcome::Empty => unread.empty += 1,
                Outcome::Unsupported => unread.binary += 1,
                Outcome::Encrypted => unread.encrypted += 1,
                Outcome::Denied => unread.denied += 1,
                Outcome::Failed => unread.failed += 1,
            }
        }
        (read, unread)
    }

    pub(super) fn save(&self, path: &Path, commit: &Commit) -> Result<()> {
        let temp = path.with_extension("tmp");
        let file = std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        out.write_all(MAGIC)?;
        for number in [
            commit.number,
            commit.epoch,
            commit.generation,
            commit.store,
            self.waste,
            self.built,
            self.docs.len() as u64,
            self.blocks.len() as u64,
            self.docs.iter().map(|doc| doc.path.len() as u64).sum(),
        ] {
            out.write_all(&number.to_le_bytes())?;
        }
        for doc in &self.docs {
            out.write_all(&doc.size.to_le_bytes())?;
            out.write_all(&doc.modified.to_le_bytes())?;
            out.write_all(&[doc.outcome.code()])?;
            out.write_all(&doc.blocks.start.to_le_bytes())?;
            out.write_all(&doc.blocks.end.to_le_bytes())?;
            out.write_all(&(doc.path.len() as u32).to_le_bytes())?;
        }
        for block in &self.blocks {
            out.write_all(&block.offset.to_le_bytes())?;
            out.write_all(&block.frame.to_le_bytes())?;
            out.write_all(&block.text.to_le_bytes())?;
            out.write_all(&block.own.to_le_bytes())?;
            out.write_all(&block.doc.to_le_bytes())?;
        }
        for doc in &self.docs {
            out.write_all(doc.path.as_bytes())?;
        }
        out.into_inner().map_err(|error| error.into_error()).context("flushing the document table")?.sync_all()?;
        std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
    }

    pub(super) fn load(path: &Path) -> Result<(Self, Commit)> {
        let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let (header, rest) = data.split_first_chunk::<HEADER_BYTES>().context("the document table is truncated")?;
        let mut header = Fields(header);
        ensure!(header.take::<8>().as_ref() == Some(MAGIC), "unknown document table format");
        let mut number = || header.u64().context("the document table is truncated");
        let commit = Commit { number: number()?, epoch: number()?, generation: number()?, store: number()? };
        let (waste, built) = (number()?, number()?);
        let docs = usize::try_from(number()?)?;
        let blocks = usize::try_from(number()?)?;
        let names = usize::try_from(number()?)?;
        let doc_bytes = docs.checked_mul(DOC_BYTES).context("the document table is damaged")?;
        let block_bytes = blocks.checked_mul(BLOCK_BYTES).context("the document table is damaged")?;
        let total = doc_bytes.checked_add(block_bytes).and_then(|sum| sum.checked_add(names));
        ensure!(total == Some(rest.len()), "the document table is truncated");
        let (doc_rows, rest) = rest.split_at(doc_bytes);
        let (block_rows, paths) = rest.split_at(block_bytes);
        let paths = std::str::from_utf8(paths).context("a path in the document table is damaged")?;
        let mut table = Self { docs: Vec::with_capacity(docs), blocks: Vec::with_capacity(blocks), waste, built };
        let mut offset = 0;
        for row in doc_rows.as_chunks::<DOC_BYTES>().0 {
            let mut row = Fields(row);
            let (Some(size), Some(modified), Some([outcome]), Some(start), Some(end), Some(length)) =
                (row.u64(), row.u32(), row.take::<1>(), row.u32(), row.u32(), row.u32())
            else {
                bail!("a row in the document table is damaged")
            };
            let outcome = Outcome::of(outcome).context("a row in the document table is damaged")?;
            let stop = offset + length as usize;
            let path = paths.get(offset..stop).context("a path in the document table is damaged")?.to_owned();
            offset = stop;
            ensure!(start <= end && end as usize <= blocks, "a row in the document table is damaged");
            table.docs.push(Doc { path, size, modified, outcome, blocks: start..end });
        }
        for row in block_rows.as_chunks::<BLOCK_BYTES>().0 {
            let mut row = Fields(row);
            let (Some(offset), Some(frame), Some(text), Some(own), Some(doc)) =
                (row.u64(), row.u32(), row.u32(), row.u32(), row.u32())
            else {
                bail!("a row in the document table is damaged")
            };
            ensure!((doc as usize) < docs, "a row in the document table is damaged");
            table.blocks.push(Block { offset, frame, text, own, doc });
        }
        Ok((table, commit))
    }
}

pub(super) fn reconcile(docs: &Docs, candidates: Vec<Candidate>) -> Plan {
    let live = || docs.docs.iter().enumerate().filter(|(_, doc)| doc.live());
    let known: FxHashMap<&str, u32> = live().map(|(doc, found)| (found.path.as_str(), doc as u32)).collect();
    let mut seen = vec![false; docs.docs.len()];
    let mut plan = Plan::default();
    let mut fresh = Vec::new();
    for candidate in candidates {
        let doc = known.get(candidate.path.as_str()).copied();
        let found = doc.and_then(|doc| docs.get(doc).map(|found| (doc, found)));
        match found {
            Some((doc, found)) => {
                if let Some(slot) = seen.get_mut(doc as usize) {
                    *slot = true;
                }
                if (found.size, found.modified) != (candidate.size, candidate.modified) {
                    plan.update.push((doc, candidate));
                }
            }
            None => fresh.push(candidate),
        }
    }
    let mut gone: FxHashMap<(u64, u32), Vec<u32>> = FxHashMap::default();
    for (doc, found) in live().filter(|(doc, _)| !seen.get(*doc).copied().unwrap_or(false)) {
        gone.entry((found.size, found.modified)).or_default().push(doc as u32);
    }
    for candidate in fresh {
        let twin = gone.get_mut(&(candidate.size, candidate.modified)).and_then(|same| {
            let named = |&doc: &u32| docs.get(doc).is_some_and(|found| name_of(&found.path) == candidate.name());
            same.iter().position(named).map(|at| same.swap_remove(at))
        });
        match twin {
            Some(doc) => plan.moved.push((doc, candidate.path)),
            None => plan.add.push(candidate),
        }
    }
    plan.remove = gone.into_values().flatten().collect();
    plan.remove.sort_unstable();
    plan
}
