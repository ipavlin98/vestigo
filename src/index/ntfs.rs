use super::volume::{Builder, Change, Checkpoint, Meta, Record as Entry, Volume};
use anyhow::{Context, Result};
use parking_lot::RwLock;
use std::ffi::c_void;
use std::ops::RangeInclusive;
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, ERROR_HANDLE_EOF, ERROR_JOURNAL_NOT_ACTIVE, GENERIC_READ, HANDLE};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAGS_AND_ATTRIBUTES, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    CREATE_USN_JOURNAL_DATA, FSCTL_CREATE_USN_JOURNAL, FSCTL_ENUM_USN_DATA, FSCTL_QUERY_USN_JOURNAL,
    FSCTL_READ_USN_JOURNAL, MFT_ENUM_DATA_V0, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0,
    USN_REASON_BASIC_INFO_CHANGE, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE, USN_REASON_DATA_TRUNCATION,
    USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE, USN_REASON_RENAME_NEW_NAME,
};
use windows::core::HSTRING;

const BUFFER: usize = 1 << 20;
const JOURNAL_SIZE: u64 = 32 << 20;
const JOURNAL_DELTA: u64 = 4 << 20;
const RECORD_VERSION: u16 = 2;
const NAMED: u32 = USN_REASON_FILE_CREATE | USN_REASON_RENAME_NEW_NAME;
const TOUCHED: u32 =
    USN_REASON_DATA_OVERWRITE | USN_REASON_DATA_EXTEND | USN_REASON_DATA_TRUNCATION | USN_REASON_BASIC_INFO_CHANGE;
const WATCHED: u32 = NAMED | TOUCHED | USN_REASON_FILE_DELETE;

struct Handle(HANDLE);

impl Handle {
    fn open(path: &str, access: u32, flags: FILE_FLAGS_AND_ATTRIBUTES) -> Result<Self> {
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                flags,
                None,
            )
        }
        .with_context(|| format!("opening {path}"))?;
        Ok(Self(handle))
    }

    fn control<T>(&self, code: u32, input: Option<&T>, output: &mut [u8]) -> windows::core::Result<usize> {
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.map(|input| input as *const T as *const c_void),
                input.map_or(0, |_| size_of::<T>() as u32),
                Some(output.as_mut_ptr().cast()),
                output.len() as u32,
                Some(&mut returned),
                None,
            )?;
        }
        Ok(returned as usize)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(super) struct Journal {
    volume: Handle,
    letter: char,
    id: u64,
    first: i64,
    next: i64,
    capacity: i64,
}

impl Journal {
    pub(super) fn open(letter: char) -> Result<Self> {
        let volume = Handle::open(&format!(r"\\.\{letter}:"), GENERIC_READ.0, FILE_FLAGS_AND_ATTRIBUTES(0))?;
        let data = query_journal(&volume)?;
        let first = data.FirstUsn.max(data.LowestValidUsn);
        let capacity = i64::try_from(data.MaximumSize).unwrap_or(i64::MAX);
        Ok(Self { volume, letter, id: data.UsnJournalID, first, next: data.NextUsn, capacity })
    }

    pub(super) fn capacity(&self) -> i64 {
        self.capacity
    }

    pub(super) fn resumes(&self, checkpoint: Checkpoint) -> bool {
        holds(checkpoint, self.id, self.first..=self.next)
    }

    pub(super) fn scan(&self) -> Result<Volume> {
        let started = Instant::now();
        let mut builder = Builder::new(self.letter, root_id(self.letter)?);
        let mut request = MFT_ENUM_DATA_V0 { StartFileReferenceNumber: 0, LowUsn: 0, HighUsn: self.next };
        let mut buffer = vec![0u8; BUFFER];
        let mut name = String::new();
        loop {
            let returned = match self.volume.control(FSCTL_ENUM_USN_DATA, Some(&request), &mut buffer) {
                Ok(returned) => returned,
                Err(error) if error.code() == ERROR_HANDLE_EOF.to_hresult() => break,
                Err(error) => return Err(error).context("enumerating the MFT"),
            };
            let Some(chunk) = buffer.get(..returned) else { break };
            let Some(next) = read_u64(chunk, 0) else { break };
            for record in Records::new(chunk) {
                name.clear();
                record.append_name(&mut name);
                builder.push_record(Entry {
                    id: record.id,
                    parent: record.parent,
                    name: &name,
                    dir: record.dir(),
                    hidden: record.hidden(),
                    meta: Meta::default(),
                });
            }
            request.StartFileReferenceNumber = next;
        }
        let read = started.elapsed();
        let volume = builder.finish(Some(Checkpoint { journal: self.id, usn: self.next }), false);
        log::info!("{}: MFT read in {read:?}, tree built in {:?}", self.letter, started.elapsed() - read);
        Ok(volume)
    }

    pub(super) fn watch(
        mut self,
        from: Checkpoint,
        volume: &RwLock<Volume>,
        mut changed: impl FnMut(&RwLock<Volume>) -> bool,
    ) -> Result<()> {
        self.next = from.usn;
        let mut buffer = vec![0u8; BUFFER];
        let mut name = String::new();
        let mut changes = Vec::new();
        loop {
            let request = READ_USN_JOURNAL_DATA_V0 {
                StartUsn: self.next,
                ReasonMask: WATCHED,
                ReturnOnlyOnClose: 0,
                Timeout: 0,
                BytesToWaitFor: 1,
                UsnJournalID: self.id,
            };
            let returned = self
                .volume
                .control(FSCTL_READ_USN_JOURNAL, Some(&request), &mut buffer)
                .context("reading the journal")?;
            let Some(chunk) = buffer.get(..returned) else { continue };
            let Some(next) = read_u64(chunk, 0) else { continue };
            self.next = next as i64;
            changes.clear();
            for record in Records::new(chunk) {
                name.clear();
                record.append_name(&mut name);
                changes.push(record.change(&name));
            }
            {
                let mut volume = volume.write();
                volume.apply(&changes);
                volume.set_checkpoint(Checkpoint { journal: self.id, usn: self.next });
            }
            if !changes.is_empty() && !changed(volume) {
                return Ok(());
            }
        }
    }
}

pub(super) fn holds(checkpoint: Checkpoint, journal: u64, readable: RangeInclusive<i64>) -> bool {
    checkpoint.journal == journal && readable.contains(&checkpoint.usn)
}

fn query_journal(volume: &Handle) -> Result<USN_JOURNAL_DATA_V0> {
    let mut data = USN_JOURNAL_DATA_V0::default();
    let output = unsafe {
        std::slice::from_raw_parts_mut((&mut data as *mut USN_JOURNAL_DATA_V0).cast::<u8>(), size_of_val(&data))
    };
    match volume.control::<()>(FSCTL_QUERY_USN_JOURNAL, None, output) {
        Ok(_) => Ok(data),
        Err(error) if error.code() == ERROR_JOURNAL_NOT_ACTIVE.to_hresult() => {
            let create = CREATE_USN_JOURNAL_DATA { MaximumSize: JOURNAL_SIZE, AllocationDelta: JOURNAL_DELTA };
            volume.control(FSCTL_CREATE_USN_JOURNAL, Some(&create), &mut []).context("creating the USN journal")?;
            query_journal(volume)
        }
        Err(error) => Err(error).context("querying the USN journal"),
    }
}

fn root_id(letter: char) -> Result<u64> {
    let root = Handle::open(&format!("{letter}:\\"), FILE_READ_ATTRIBUTES.0, FILE_FLAG_BACKUP_SEMANTICS)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(root.0, &mut info) }.context("reading the root folder id")?;
    Ok((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
}

struct Record<'a> {
    id: u64,
    parent: u64,
    reason: u32,
    attributes: u32,
    name: &'a [u8],
}

impl Record<'_> {
    fn dir(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0
    }

    fn hidden(&self) -> bool {
        self.attributes & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0
    }

    fn change(&self, name: &str) -> Change {
        if self.reason & USN_REASON_FILE_DELETE != 0 {
            Change::Remove(self.id)
        } else if self.reason & NAMED != 0 {
            let (id, parent, dir, hidden) = (self.id, self.parent, self.dir(), self.hidden());
            Change::Upsert { id, parent, name: name.to_owned(), dir, hidden }
        } else {
            Change::Touch(self.id)
        }
    }

    fn append_name(&self, out: &mut String) {
        let units = self.name.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair));
        out.extend(char::decode_utf16(units).map(|ch| ch.unwrap_or(char::REPLACEMENT_CHARACTER)));
    }
}

struct Records<'a> {
    buffer: &'a [u8],
    offset: usize,
}

impl<'a> Records<'a> {
    fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, offset: size_of::<u64>() }
    }

    fn parse(&self, offset: usize, length: usize) -> Option<Record<'a>> {
        let record = self.buffer.get(offset..offset + length)?;
        let version = u16::from_le_bytes(record.get(4..6)?.try_into().ok()?);
        if version != RECORD_VERSION {
            return None;
        }
        let name_length = usize::from(u16::from_le_bytes(record.get(56..58)?.try_into().ok()?));
        let name_offset = usize::from(u16::from_le_bytes(record.get(58..60)?.try_into().ok()?));
        Some(Record {
            id: read_u64(record, 8)?,
            parent: read_u64(record, 16)?,
            reason: read_u32(record, 40)?,
            attributes: read_u32(record, 52)?,
            name: record.get(name_offset..name_offset + name_length)?,
        })
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Record<'a>;

    fn next(&mut self) -> Option<Record<'a>> {
        loop {
            let length = read_u32(self.buffer, self.offset)? as usize;
            if length == 0 {
                return None;
            }
            let offset = self.offset;
            self.offset += length;
            if let Some(record) = self.parse(offset, length) {
                return Some(record);
            }
        }
    }
}

fn read_u64(buffer: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buffer.get(offset..offset + 8)?.try_into().ok()?))
}

fn read_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buffer.get(offset..offset + 4)?.try_into().ok()?))
}
