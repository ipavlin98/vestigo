use super::docs::Block;
use crate::search::fold;
use anyhow::{Context, Result, ensure};
use std::cell::RefCell;
use std::fs::File;
use std::os::windows::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use zstd::bulk::{Compressor, Decompressor};

pub(super) const BLOCK_TEXT: usize = 64 * 1024;
pub(super) const OVERLAP: usize = 256;
const LEVEL: i32 = 3;
const MAGIC: &[u8; 8] = b"VSTBLOCK";
const HEADER: u64 = 16;
const SCRATCH_BYTES: usize = BLOCK_TEXT + 2 * OVERLAP;

thread_local! {
    static SCRATCH: RefCell<Option<Scratch>> = const { RefCell::new(None) };
    static COMPRESSOR: RefCell<Option<Compressor<'static>>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Span {
    pub(super) start: usize,
    pub(super) own_end: usize,
    pub(super) end: usize,
}

fn boundary(text: &str, at: usize) -> usize {
    (at.min(text.len())..=text.len()).find(|&at| text.is_char_boundary(at)).unwrap_or(text.len())
}

pub(super) fn spans(text: &str) -> impl Iterator<Item = Span> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        (start < text.len()).then(|| {
            let own_end = boundary(text, start + BLOCK_TEXT);
            let span = Span { start, own_end, end: boundary(text, own_end + OVERLAP) };
            start = own_end;
            span
        })
    })
}

pub(super) fn compress(text: &[u8]) -> Result<Vec<u8>> {
    COMPRESSOR.with(|cell| {
        let mut cell = cell.borrow_mut();
        let compressor = match cell.as_mut() {
            Some(compressor) => compressor,
            None => cell.insert(Compressor::new(LEVEL).context("starting zstd")?),
        };
        compressor.compress(text).context("compressing a block")
    })
}

pub(super) struct Scratch {
    decompressor: Decompressor<'static>,
    packed: Vec<u8>,
    pub(super) raw: Vec<u8>,
    pub(super) next: Vec<u8>,
    pub(super) keys: Vec<u32>,
    lower: String,
}

impl Scratch {
    fn new() -> Result<Self> {
        Ok(Self {
            decompressor: Decompressor::new().context("starting zstd")?,
            packed: Vec::with_capacity(SCRATCH_BYTES),
            raw: Vec::with_capacity(SCRATCH_BYTES),
            next: Vec::with_capacity(SCRATCH_BYTES),
            keys: Vec::with_capacity(SCRATCH_BYTES),
            lower: String::with_capacity(SCRATCH_BYTES),
        })
    }

    pub(super) fn folded(&mut self) -> &[u8] {
        if fold::fold(&mut self.raw) {
            return &self.raw;
        }
        self.lower.clear();
        if let Ok(text) = std::str::from_utf8(&self.raw) {
            self.lower.extend(text.chars().flat_map(char::to_lowercase));
        }
        self.lower.as_bytes()
    }
}

pub(super) fn with_scratch<T>(work: impl FnOnce(&mut Scratch) -> Result<T>) -> Result<T> {
    SCRATCH.with(|cell| {
        let mut cell = cell.borrow_mut();
        let scratch = match cell.as_mut() {
            Some(scratch) => scratch,
            None => cell.insert(Scratch::new()?),
        };
        work(scratch)
    })
}

pub(super) struct Store {
    file: File,
    len: AtomicU64,
}

impl Store {
    pub(super) fn create(path: &Path, epoch: u64) -> Result<Self> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        let mut header = [0; HEADER as usize];
        let (magic, stamp) = header.split_at_mut(MAGIC.len());
        magic.copy_from_slice(MAGIC);
        stamp.copy_from_slice(&epoch.to_le_bytes());
        write_all_at(&file, &header, 0)?;
        Ok(Self { file, len: AtomicU64::new(HEADER) })
    }

    pub(super) fn open(path: &Path, epoch: u64, valid: u64) -> Result<Self> {
        let file =
            File::options().read(true).write(true).open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut header = [0; HEADER as usize];
        read_exact_at(&file, &mut header, 0)?;
        ensure!(header.starts_with(MAGIC), "unknown text store format");
        ensure!(header.get(MAGIC.len()..) == Some(&epoch.to_le_bytes()[..]), "the text store is from another build");
        let found = file.metadata().context("reading the text store size")?.len();
        ensure!(found >= valid, "the text store is shorter than recorded");
        if found > valid {
            log::info!("Dropping {} bytes written after the last save of the text store", found - valid);
            file.set_len(valid).context("cutting the text store")?;
        }
        Ok(Self { file, len: AtomicU64::new(valid) })
    }

    pub(super) fn len(&self) -> u64 {
        self.len.load(Ordering::Relaxed)
    }

    pub(super) fn append(&self, frame: &[u8]) -> Result<u64> {
        let offset = self.len();
        write_all_at(&self.file, frame, offset)?;
        self.len.store(offset + frame.len() as u64, Ordering::Relaxed);
        Ok(offset)
    }

    pub(super) fn frame(&self, block: &Block, into: &mut Vec<u8>) -> Result<()> {
        into.resize(block.frame as usize, 0);
        read_exact_at(&self.file, into, block.offset)
    }

    pub(super) fn read(&self, block: &Block, scratch: &mut Scratch) -> Result<()> {
        let Scratch { decompressor, packed, raw, .. } = scratch;
        self.frame(block, packed)?;
        raw.clear();
        raw.reserve(block.text as usize);
        decompressor.decompress_to_buffer(packed.as_slice(), raw).context("unpacking a block")?;
        Ok(())
    }

    pub(super) fn read_next(&self, block: &Block, scratch: &mut Scratch) -> Result<()> {
        let Scratch { decompressor, packed, next, .. } = scratch;
        self.frame(block, packed)?;
        next.clear();
        next.reserve(block.text as usize);
        decompressor.decompress_to_buffer(packed.as_slice(), next).context("unpacking a block")?;
        Ok(())
    }

    pub(super) fn sync(&self) -> Result<()> {
        self.file.sync_data().context("flushing the text store")
    }
}

fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> Result<()> {
    let mut done = 0;
    while let Some(rest) = buffer.get_mut(done..).filter(|rest| !rest.is_empty()) {
        let read = file.seek_read(rest, offset + done as u64).context("reading the text store")?;
        ensure!(read > 0, "the text store ended early");
        done += read;
    }
    Ok(())
}

fn write_all_at(file: &File, bytes: &[u8], offset: u64) -> Result<()> {
    let mut done = 0;
    while let Some(rest) = bytes.get(done..).filter(|rest| !rest.is_empty()) {
        done += file.seek_write(rest, offset + done as u64).context("writing the text store")?;
    }
    Ok(())
}
