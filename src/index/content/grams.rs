use super::store::OVERLAP;
use anyhow::{Context, Result, ensure};
use parking_lot::RwLock;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::os::windows::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const BASE: &str = "grams.bin";
const DELTA: &str = "grams.delta";
const MERGING: &str = "grams.tmp";
const SAVING: &str = "delta.tmp";
const RUN_PREFIX: &str = "run-";
const RUN_SUFFIX: &str = ".bin";
const MAGIC: &[u8; 8] = b"VSTGRAM1";
const TRAILER: u64 = 32;
const ENTRY: usize = 16;
const SHARD_BITS: u32 = 6;
const SHARDS: usize = 1 << SHARD_BITS;
const MAX_LISTS: usize = 12;
const MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;
const DELTA_WEIGHT: u64 = 2;
const GALLOP_START: usize = 1;

pub(super) fn key(first: char, second: char, third: char) -> u32 {
    let packed = (u64::from(first) << 42) ^ (u64::from(second) << 21) ^ u64::from(third);
    (packed.wrapping_mul(MULTIPLIER) >> 32) as u32
}

pub(super) fn keys_of(text: &str, keys: &mut Vec<u32>) {
    keys.clear();
    let mut chars = text.chars();
    let (Some(mut first), Some(mut second)) = (chars.next(), chars.next()) else { return };
    for third in chars {
        keys.push(key(first, second, third));
        (first, second) = (second, third);
    }
    keys.sort_unstable();
    keys.dedup();
}

fn shard(key: u32) -> usize {
    (key >> (u32::BITS - SHARD_BITS)) as usize
}

fn put(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn encode(blocks: &[u32], out: &mut Vec<u8>) {
    let mut last = 0;
    for &block in blocks {
        put(block - last, out);
        last = block;
    }
}

fn decode(bytes: &[u8], out: &mut Vec<u32>) {
    let (mut last, mut value, mut shift) = (0u32, 0u32, 0);
    for &byte in bytes {
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 != 0 {
            shift += 7;
        } else {
            last = last.wrapping_add(value);
            out.push(last);
            (value, shift) = (0, 0);
        }
    }
}

fn intersect(left: &mut Vec<u32>, right: &[u32]) {
    let mut from = 0;
    left.retain(|block| {
        let mut step = GALLOP_START;
        while right.get(from + step).is_some_and(|found| found < block) {
            from += step;
            step *= 2;
        }
        let window = right.get(from..).unwrap_or_default();
        let window = window.get(..(step + 1).min(window.len())).unwrap_or_default();
        from += window.partition_point(|found| found < block);
        right.get(from) == Some(block)
    });
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    key: u32,
    len: u32,
    offset: u64,
}

struct Segment {
    path: PathBuf,
    file: File,
    table: Vec<Entry>,
}

impl Segment {
    fn open(path: &Path) -> Result<(Self, u64)> {
        let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let total = file.metadata().context("reading the index size")?.len();
        let tail = total.checked_sub(TRAILER).context("the gram index is truncated")?;
        let mut trailer = [0; TRAILER as usize];
        read_at(&file, &mut trailer, tail)?;
        let (fields, magic) = trailer.split_last_chunk::<8>().context("the gram index is truncated")?;
        ensure!(magic == MAGIC, "unknown gram index format");
        let number = |at: usize| -> Result<u64> {
            let field = fields.get(at..at + 8).and_then(|bytes| bytes.try_into().ok());
            Ok(u64::from_le_bytes(field.context("the gram index is truncated")?))
        };
        let (start, keys, stamp) = (number(0)?, number(8)?, number(16)?);
        ensure!(start.checked_add(keys * ENTRY as u64) == Some(tail), "the gram index is truncated");
        let mut bytes = vec![0; usize::try_from(tail - start)?];
        read_at(&file, &mut bytes, start)?;
        let table = bytes
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .filter_map(|row| {
                let (key, rest) = row.split_first_chunk::<4>()?;
                let (len, offset) = rest.split_first_chunk::<4>()?;
                let offset = u64::from_le_bytes(*offset.first_chunk::<8>()?);
                Some(Entry { key: u32::from_le_bytes(*key), len: u32::from_le_bytes(*len), offset })
            })
            .collect();
        Ok((Self { path: path.to_owned(), file, table }, stamp))
    }

    fn find(&self, key: u32) -> Option<&Entry> {
        let at = self.table.binary_search_by_key(&key, |entry| entry.key).ok()?;
        self.table.get(at)
    }

    fn read(&self, entry: &Entry, bytes: &mut Vec<u8>) -> Result<()> {
        bytes.resize(entry.len as usize, 0);
        read_at(&self.file, bytes, entry.offset)
    }

    fn shard(&self, shard: usize) -> &[Entry] {
        let first = self.table.partition_point(|entry| self::shard(entry.key) < shard);
        let end = self.table.partition_point(|entry| self::shard(entry.key) <= shard);
        self.table.get(first..end).unwrap_or_default()
    }
}

fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> Result<()> {
    let mut done = 0;
    while let Some(rest) = buffer.get_mut(done..).filter(|rest| !rest.is_empty()) {
        let read = file.seek_read(rest, offset + done as u64).context("reading the gram index")?;
        ensure!(read > 0, "the gram index ended early");
        done += read;
    }
    Ok(())
}

struct Writer {
    out: BufWriter<File>,
    table: Vec<Entry>,
    offset: u64,
}

impl Writer {
    fn create(path: &Path) -> Result<Self> {
        let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
        Ok(Self { out: BufWriter::with_capacity(1 << 20, file), table: Vec::new(), offset: 0 })
    }

    fn add(&mut self, encoded: Encoded) -> Result<()> {
        self.out.write_all(&encoded.bytes)?;
        let base = self.offset;
        self.table.extend(encoded.table.iter().map(|entry| Entry { offset: base + entry.offset, ..*entry }));
        self.offset += encoded.bytes.len() as u64;
        Ok(())
    }

    fn finish(mut self, stamp: u64) -> Result<()> {
        for entry in &self.table {
            self.out.write_all(&entry.key.to_le_bytes())?;
            self.out.write_all(&entry.len.to_le_bytes())?;
            self.out.write_all(&entry.offset.to_le_bytes())?;
        }
        self.out.write_all(&self.offset.to_le_bytes())?;
        self.out.write_all(&(self.table.len() as u64).to_le_bytes())?;
        self.out.write_all(&stamp.to_le_bytes())?;
        self.out.write_all(MAGIC)?;
        self.out.into_inner().map_err(|error| error.into_error()).context("flushing the gram index")?.sync_data()?;
        Ok(())
    }
}

#[derive(Default)]
struct Encoded {
    bytes: Vec<u8>,
    table: Vec<Entry>,
}

impl Encoded {
    fn seal(&mut self, key: u32, start: usize) {
        let len = (self.bytes.len() - start) as u32;
        self.table.push(Entry { key, len, offset: start as u64 });
    }
}

type Lists = FxHashMap<u32, Vec<u32>>;

fn encode_shard(lists: &Lists) -> Encoded {
    let mut keys: Vec<u32> = lists.keys().copied().collect();
    keys.sort_unstable();
    let mut encoded = Encoded::default();
    for key in keys {
        let start = encoded.bytes.len();
        encode(lists.get(&key).map_or(&[], Vec::as_slice), &mut encoded.bytes);
        encoded.seal(key, start);
    }
    encoded
}

fn merge_shard(segments: &[&Segment], shard: usize) -> Result<Encoded> {
    let mut parts: Vec<(&[Entry], Vec<u8>)> = Vec::with_capacity(segments.len());
    let mut rows: Vec<(u32, usize, usize)> = Vec::new();
    for (position, segment) in segments.iter().enumerate() {
        let table = segment.shard(shard);
        let start = table.first().map_or(0, |entry| entry.offset);
        let end = table.last().map_or(0, |entry| entry.offset + u64::from(entry.len));
        let mut bytes = vec![0; usize::try_from(end - start)?];
        read_at(&segment.file, &mut bytes, start)?;
        rows.extend(table.iter().enumerate().map(|(row, entry)| (entry.key, position, row)));
        parts.push((table, bytes));
    }
    rows.sort_unstable();
    let list = |position: usize, row: usize| -> Option<&[u8]> {
        let (table, bytes) = parts.get(position)?;
        let entry = table.get(row)?;
        let start = (entry.offset - table.first()?.offset) as usize;
        bytes.get(start..start + entry.len as usize)
    };
    let mut encoded = Encoded::default();
    let mut blocks = Vec::new();
    for group in rows.chunk_by(|left, right| left.0 == right.0) {
        let Some(&(key, ..)) = group.first() else { continue };
        let start = encoded.bytes.len();
        if let [(_, position, row)] = group {
            encoded.bytes.extend_from_slice(list(*position, *row).unwrap_or_default());
        } else {
            blocks.clear();
            for &(_, position, row) in group {
                decode(list(position, row).unwrap_or_default(), &mut blocks);
            }
            encode(&blocks, &mut encoded.bytes);
        }
        encoded.seal(key, start);
    }
    Ok(encoded)
}

pub(super) struct Merged {
    generation: u64,
}

pub(super) struct Grams {
    folder: PathBuf,
    generation: u64,
    base: Option<Segment>,
    runs: Vec<Segment>,
    delta: Vec<RwLock<Lists>>,
    postings: AtomicUsize,
    next_run: u64,
}

impl Grams {
    pub(super) fn empty(folder: &Path) -> Self {
        Self {
            folder: folder.to_owned(),
            generation: 0,
            base: None,
            runs: Vec::new(),
            delta: (0..SHARDS).map(|_| RwLock::default()).collect(),
            postings: AtomicUsize::new(0),
            next_run: 0,
        }
    }

    pub(super) fn reopened(folder: &Path, generation: u64) -> Result<Self> {
        let mut grams = Self::empty(folder);
        grams.generation = generation;
        if generation != 0 {
            let (base, stamp) = Segment::open(&folder.join(BASE))?;
            ensure!(stamp == generation, "the gram index is from another build");
            grams.base = Some(base);
        }
        Ok(grams)
    }

    pub(super) fn load(folder: &Path, generation: u64, commit: u64) -> Result<Self> {
        let mut grams = Self::reopened(folder, generation)?;
        let (delta, stamp) = Segment::open(&folder.join(DELTA))?;
        ensure!(stamp == commit, "the saved changes are from another build");
        let mut bytes = Vec::new();
        for entry in &delta.table {
            delta.read(entry, &mut bytes)?;
            if let Some(shard) = grams.delta.get_mut(shard(entry.key)) {
                let list = shard.get_mut().entry(entry.key).or_default();
                decode(&bytes, list);
                *grams.postings.get_mut() += list.len();
            }
        }
        Ok(grams)
    }

    pub(super) fn clear_leftovers(folder: &Path) {
        let Ok(entries) = std::fs::read_dir(folder) else { return };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let run = name.starts_with(RUN_PREFIX) && name.ends_with(RUN_SUFFIX);
            if (run || name == MERGING || name == SAVING)
                && let Err(error) = std::fs::remove_file(entry.path())
            {
                log::warn!("Couldn't remove {}: {error}", entry.path().display());
            }
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn postings(&self) -> usize {
        self.postings.load(Ordering::Relaxed)
    }

    pub(super) fn runs(&self) -> usize {
        self.runs.len()
    }

    pub(super) fn add(&self, blocks: &[(u32, Vec<u32>)]) {
        self.delta.par_iter().enumerate().for_each(|(position, shard)| {
            let mut added = 0;
            let mut lists = shard.write();
            for (block, keys) in blocks {
                let start = keys.partition_point(|&key| self::shard(key) < position);
                let end = keys.partition_point(|&key| self::shard(key) <= position);
                let keys = keys.get(start..end).unwrap_or_default();
                added += keys.len();
                for &key in keys {
                    lists.entry(key).or_default().push(*block);
                }
            }
            self.postings.fetch_add(added, Ordering::Relaxed);
        });
    }

    fn segments(&self) -> impl Iterator<Item = &Segment> {
        self.base.iter().chain(&self.runs)
    }

    fn weight(&self, key: u32) -> u64 {
        let stored: u64 =
            self.segments().filter_map(|segment| segment.find(key)).map(|entry| u64::from(entry.len)).sum();
        let delta = self.delta.get(shard(key)).and_then(|lists| lists.read().get(&key).map(Vec::len)).unwrap_or(0);
        stored + delta as u64 * DELTA_WEIGHT
    }

    fn list(&self, key: u32, bytes: &mut Vec<u8>, out: &mut Vec<u32>) -> Result<()> {
        out.clear();
        for segment in self.segments() {
            if let Some(entry) = segment.find(key) {
                segment.read(entry, bytes)?;
                decode(bytes, out);
            }
        }
        if let Some(lists) = self.delta.get(shard(key))
            && let Some(list) = lists.read().get(&key)
        {
            out.extend_from_slice(list);
        }
        Ok(())
    }

    pub(super) fn candidates(&self, term: &str) -> Result<Vec<u32>> {
        let mut end = term.len().min(OVERLAP);
        while !term.is_char_boundary(end) {
            end -= 1;
        }
        let mut keys = Vec::new();
        keys_of(term.get(..end).unwrap_or(term), &mut keys);
        let mut weighted: Vec<(u64, u32)> = keys.into_iter().map(|key| (self.weight(key), key)).collect();
        weighted.sort_unstable();
        if weighted.first().is_none_or(|&(weight, _)| weight == 0) {
            return Ok(Vec::new());
        }
        let (mut found, mut next, mut bytes) = (Vec::new(), Vec::new(), Vec::new());
        for (position, &(_, key)) in weighted.iter().take(MAX_LISTS).enumerate() {
            if position == 0 {
                self.list(key, &mut bytes, &mut found)?;
            } else {
                self.list(key, &mut bytes, &mut next)?;
                intersect(&mut found, &next);
            }
            if found.is_empty() {
                break;
            }
        }
        Ok(found)
    }

    pub(super) fn spill(&mut self) -> Result<()> {
        let run = self.spilled()?;
        self.attach_pending(run);
        Ok(())
    }

    pub(super) fn spilled(&self) -> Result<PendingRun> {
        let path = self.folder.join(format!("{RUN_PREFIX}{}{RUN_SUFFIX}", self.next_run + 1));
        self.write_delta(&path, 0)?;
        Ok(PendingRun(Segment::open(&path)?.0))
    }

    pub(super) fn attach_pending(&mut self, run: PendingRun) {
        self.next_run += 1;
        self.runs.push(run.0);
        for shard in &mut self.delta {
            shard.get_mut().clear();
        }
        *self.postings.get_mut() = 0;
    }

    fn write_delta(&self, path: &Path, stamp: u64) -> Result<()> {
        let parts: Vec<Encoded> = self.delta.par_iter().map(|lists| encode_shard(&lists.read())).collect();
        let mut writer = Writer::create(path)?;
        for encoded in parts {
            writer.add(encoded)?;
        }
        writer.finish(stamp)
    }

    pub(super) fn save_delta(&self, commit: u64) -> Result<()> {
        let saving = self.folder.join(SAVING);
        self.write_delta(&saving, commit)?;
        std::fs::rename(&saving, self.folder.join(DELTA)).context("replacing the saved changes")
    }

    pub(super) fn merged(&self) -> Result<Merged> {
        let segments: Vec<&Segment> = self.segments().collect();
        let generation = self.generation + 1;
        let threads = rayon::current_num_threads().max(1);
        let shards: Vec<usize> = (0..SHARDS).collect();
        let mut writer = Writer::create(&self.folder.join(MERGING))?;
        for group in shards.chunks(threads) {
            let parts: Vec<Encoded> =
                group.par_iter().map(|&shard| merge_shard(&segments, shard)).collect::<Result<_>>()?;
            for encoded in parts {
                writer.add(encoded)?;
            }
        }
        writer.finish(generation)?;
        Ok(Merged { generation })
    }

    pub(super) fn install(&mut self, merged: Merged) -> Result<()> {
        let old: Vec<PathBuf> = self.runs.drain(..).map(|run| run.path).collect();
        self.base = None;
        let path = self.folder.join(BASE);
        std::fs::rename(self.folder.join(MERGING), &path).context("replacing the gram index")?;
        self.base = Some(Segment::open(&path)?.0);
        self.generation = merged.generation;
        for run in old {
            if let Err(error) = std::fs::remove_file(&run) {
                log::warn!("Couldn't remove {}: {error}", run.display());
            }
        }
        Ok(())
    }

    pub(super) fn merge(&mut self) -> Result<()> {
        let merged = self.merged()?;
        self.install(merged)
    }
}

pub(super) struct PendingRun(Segment);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_survive_encoding() {
        let blocks = [0, 1, 2, 300, 70_000, 4_000_000_000];
        let mut bytes = Vec::new();
        encode(&blocks, &mut bytes);
        let mut decoded = Vec::new();
        decode(&bytes, &mut decoded);
        assert_eq!(decoded, blocks);
    }

    #[test]
    fn intersection_keeps_only_shared_blocks() {
        let mut left = vec![2, 3, 500, 900, 1000];
        let right: Vec<u32> = (0..1000).filter(|value| value % 3 == 0 || *value == 500).collect();
        intersect(&mut left, &right);
        assert_eq!(left, [3, 500, 900]);
        let mut left = vec![5];
        intersect(&mut left, &[]);
        assert!(left.is_empty());
    }

    #[test]
    fn the_keys_of_a_text_are_sorted_and_unique() {
        let mut keys = Vec::new();
        keys_of("аааа", &mut keys);
        assert_eq!(keys, [key('а', 'а', 'а')]);
        keys_of("ab", &mut keys);
        assert!(keys.is_empty());
    }
}
