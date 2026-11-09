use super::volume::{Builder, Checkpoint, Meta, Record, Volume};
use anyhow::{Context, Result, bail, ensure};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAGIC: &[u8; 8] = b"VESTIGO4";
const DIR: u8 = 1;
const HIDDEN: u8 = 2;
const HEADER_BYTES: usize = 8 + 4 + 8 + 8 + 8 + 1;
const ROW_BYTES: usize = 8 + 8 + 8 + 4 + 2 + 1;
const TRAILER_BYTES: usize = 8 + 8;

pub(super) fn file(dir: &Path, letter: char) -> PathBuf {
    dir.join(format!("{letter}.bin"))
}

pub(super) struct Written {
    file: std::fs::File,
    temp: PathBuf,
    path: PathBuf,
    letter: char,
    started: Instant,
}

impl Written {
    pub(super) fn commit(self) -> Result<()> {
        self.file.sync_all().context("flushing the index cache")?;
        drop(self.file);
        std::fs::rename(&self.temp, &self.path).with_context(|| format!("replacing {}", self.path.display()))?;
        log::info!("{}: index cache saved in {:?}", self.letter, self.started.elapsed());
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn save(path: &Path, volume: &Volume) -> Result<()> {
    write(path, volume)?.map_or(Ok(()), Written::commit)
}

pub(super) fn write(path: &Path, volume: &Volume) -> Result<Option<Written>> {
    let Some(checkpoint) = volume.checkpoint() else { return Ok(None) };
    let started = Instant::now();
    let folder = path.parent().context("the cache file has no folder")?;
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let temp = path.with_extension("tmp");
    let file = std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    out.write_all(MAGIC)?;
    out.write_all(&u32::from(volume.letter()).to_le_bytes())?;
    out.write_all(&checkpoint.journal.to_le_bytes())?;
    out.write_all(&checkpoint.usn.to_le_bytes())?;
    out.write_all(&volume.root_id().to_le_bytes())?;
    out.write_all(&[u8::from(volume.details_complete())])?;
    let mut names = Vec::with_capacity(volume.name_bytes());
    let mut rows: u64 = 0;
    let mut failure = None;
    volume.for_each_record(|Record { id, parent, name, dir, hidden, meta }| {
        if failure.is_some() {
            return;
        }
        let length = u16::try_from(name.len()).unwrap_or(0);
        let flags = if dir { DIR } else { 0 } | if hidden { HIDDEN } else { 0 };
        let written = out
            .write_all(&id.to_le_bytes())
            .and_then(|()| out.write_all(&parent.to_le_bytes()))
            .and_then(|()| out.write_all(&meta.size.to_le_bytes()))
            .and_then(|()| out.write_all(&meta.modified.to_le_bytes()))
            .and_then(|()| out.write_all(&length.to_le_bytes()))
            .and_then(|()| out.write_all(&[flags]));
        names.extend_from_slice(name.as_bytes().get(..usize::from(length)).unwrap_or_default());
        rows += 1;
        failure = written.err();
    });
    if let Some(error) = failure {
        return Err(error).context("writing the index cache");
    }
    out.write_all(&names)?;
    out.write_all(&rows.to_le_bytes())?;
    out.write_all(&(names.len() as u64).to_le_bytes())?;
    let file = out.into_inner().map_err(|error| error.into_error()).context("flushing the index cache")?;
    Ok(Some(Written { file, temp, path: path.to_owned(), letter: volume.letter(), started }))
}

pub(super) fn load(path: &Path, letter: char) -> Result<Volume> {
    let started = Instant::now();
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let (header, rest) = data.split_first_chunk::<HEADER_BYTES>().context("the cache is truncated")?;
    let (body, trailer) = rest.split_last_chunk::<TRAILER_BYTES>().context("the cache is truncated")?;
    let mut header = Fields(header);
    ensure!(header.take::<8>().as_ref() == Some(MAGIC), "unknown cache format");
    ensure!(header.u32() == Some(u32::from(letter)), "the cache belongs to another drive");
    let (Some(journal), Some(usn), Some(root), Some([complete])) =
        (header.u64(), header.u64(), header.u64(), header.take::<1>())
    else {
        bail!("the cache header is damaged")
    };
    let mut trailer = Fields(trailer);
    let (Some(rows), Some(name_bytes)) = (trailer.u64(), trailer.u64()) else { bail!("the cache is truncated") };
    let rows = usize::try_from(rows)?;
    let row_bytes = rows.checked_mul(ROW_BYTES).context("the cache is damaged")?;
    ensure!(row_bytes.checked_add(usize::try_from(name_bytes)?) == Some(body.len()), "the cache is truncated");
    let (table, names) = body.split_at(row_bytes);
    let names = std::str::from_utf8(names).context("a name in the cache is damaged")?;
    let mut builder = Builder::with_capacity(letter, root, rows, names.len());
    let mut offset = 0;
    for row in table.as_chunks::<ROW_BYTES>().0 {
        let mut row = Fields(row);
        let (Some(id), Some(parent), Some(size), Some(modified), Some(length), Some([flags])) =
            (row.u64(), row.u64(), row.u64(), row.u32(), row.u16(), row.take::<1>())
        else {
            bail!("a row in the cache is damaged")
        };
        let end = offset + usize::from(length);
        let name = names.get(offset..end).context("a name in the cache is damaged")?;
        offset = end;
        let (dir, hidden) = (flags & DIR != 0, flags & HIDDEN != 0);
        builder.push_record(Record { id, parent, name, dir, hidden, meta: Meta { size, modified } });
    }
    ensure!(offset == names.len(), "the cache names don't add up");
    let volume = builder.finish(Some(Checkpoint { journal, usn: usn as i64 }), complete != 0);
    log::info!("{letter}: index loaded from the cache in {:?}", started.elapsed());
    Ok(volume)
}

pub(super) struct Fields<'a>(pub(super) &'a [u8]);

impl Fields<'_> {
    pub(super) fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (field, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*field)
    }

    fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    pub(super) fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    pub(super) fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }
}
