mod builder;
mod documents;
mod family;
mod folders;
mod ids;
mod scan;
mod storage;

pub(crate) use builder::Builder;
pub(crate) use family::{Family, describe};

use crate::search;
use ids::Ids;
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use storage::{fit, room};

pub(crate) const ROOT: u32 = 0;
const NONE: u32 = u32::MAX;
const MAX_DEPTH: usize = 256;
const SCAN_CHUNK: usize = 16 * 1024;
const COMPACT_MIN_WASTE: usize = 8 << 20;
const COMPACT_SHARE: usize = 4;
const HEADROOM: usize = 8;
const MIN_ROOM: usize = 4096;
const LOWER_GROWTH: usize = 2;

const DIR: u8 = 1;
const DEAD: u8 = 2;
const EXCLUDED: u8 = 4;
const ORPHAN: u8 = 8;
const RECYCLED: u8 = 16;
const PROGRAMS: u8 = 32;
const ASCII: u8 = 64;
const HIDDEN: u8 = 128;
const INHERITED: u8 = EXCLUDED | RECYCLED | PROGRAMS;
const RECYCLE_BIN: &str = "$Recycle.Bin";
pub(super) const FOCUS_BONUS: i32 = 1_000;
const FOLDER_LETTER: char = '\\';

static SERIALS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Prefer {
    Programs,
    Within(u32),
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Area<'a> {
    Everywhere,
    Children(&'a [u32]),
    Subtree(&'a [u32]),
}

#[derive(Clone, Copy)]
struct Node {
    parent: u32,
    name: u32,
    lower: u32,
    name_len: u16,
    lower_len: u16,
    flags: u8,
}

impl Node {
    fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn visible(self) -> bool {
        self.flags & (DEAD | EXCLUDED) == 0
    }

    fn persistent(self) -> bool {
        !self.has(DEAD | ORPHAN) && (!self.has(EXCLUDED) || self.has(RECYCLED))
    }

    fn arena_bytes(self) -> usize {
        usize::from(self.name_len) + usize::from(self.lower_len)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hidden {
    Shown,
    System,
    Recycled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    pub(crate) journal: u64,
    pub(crate) usn: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Meta {
    pub(crate) size: u64,
    pub(crate) modified: u32,
}

impl Meta {
    pub(crate) fn known(self) -> bool {
        self.modified != 0
    }
}

pub(crate) struct Record<'a> {
    pub(crate) id: u64,
    pub(crate) parent: u64,
    pub(crate) name: &'a str,
    pub(crate) dir: bool,
    pub(crate) hidden: bool,
    pub(crate) meta: Meta,
}

pub(crate) enum Change {
    Remove(u64),
    Upsert { id: u64, parent: u64, name: String, dir: bool, hidden: bool },
    Touch(u64),
}

#[derive(Default)]
pub(crate) struct Edits {
    now: Vec<u32>,
    before: Vec<Node>,
}

#[derive(Default)]
struct Details {
    sizes: Vec<u64>,
    times: Vec<u32>,
    complete: bool,
    stale: Vec<u32>,
}

pub(crate) struct Volume {
    letter: char,
    folder: bool,
    serial: u64,
    nodes: Vec<Node>,
    letters: Vec<u32>,
    names: String,
    lower: String,
    ids: Ids,
    root_id: u64,
    checkpoint: Option<Checkpoint>,
    details: Details,
    live: usize,
    waste: usize,
    located: Mutex<FxHashMap<String, Option<u32>>>,
    edits: Edits,
}

impl Volume {
    pub(crate) fn new(letter: char) -> Self {
        Self::rooted(letter, &format!("{letter}:"), false)
    }

    pub(crate) fn folder(path: &str) -> Self {
        let drive = path.chars().next().filter(|_| path.get(1..2) == Some(":"));
        Self::rooted(
            drive.map_or(FOLDER_LETTER, |letter| letter.to_ascii_uppercase()),
            path.trim_end_matches('\\'),
            true,
        )
    }

    fn rooted(letter: char, root: &str, folder: bool) -> Self {
        let mut volume = Self {
            letter,
            folder,
            serial: SERIALS.fetch_add(1, Ordering::Relaxed),
            nodes: Vec::new(),
            letters: Vec::new(),
            names: String::new(),
            lower: String::new(),
            ids: Ids::default(),
            root_id: 0,
            checkpoint: None,
            details: Details::default(),
            live: 0,
            waste: 0,
            located: Mutex::default(),
            edits: Edits::default(),
        };
        let root = volume.node(NONE, root, true, false);
        volume.place(root, Meta::default());
        volume
    }

    fn reserve(&mut self, records: usize, name_bytes: usize) {
        let (records, name_bytes) = (records + records / HEADROOM, name_bytes + name_bytes / HEADROOM);
        self.nodes.reserve_exact(records);
        self.details.sizes.reserve_exact(records);
        self.details.times.reserve_exact(records);
        self.ids.reserve(records);
        self.names.reserve_exact(name_bytes);
        self.lower.reserve_exact(name_bytes);
    }

    fn fit(&mut self) {
        fit(&mut self.nodes);
        fit(&mut self.letters);
        fit(&mut self.details.sizes);
        fit(&mut self.details.times);
        fit(&mut self.names);
        fit(&mut self.lower);
        self.ids.fit();
    }

    fn make_room(&mut self, entries: usize, name_bytes: usize) {
        room(&mut self.nodes, entries);
        room(&mut self.letters, entries);
        room(&mut self.details.sizes, entries);
        room(&mut self.details.times, entries);
        room(&mut self.names, name_bytes);
        room(&mut self.lower, name_bytes * LOWER_GROWTH);
    }

    fn push(&mut self, node: Node, meta: Meta) -> u32 {
        let index = self.nodes.len() as u32;
        self.nodes.push(node);
        self.details.sizes.push(meta.size);
        self.details.times.push(meta.modified);
        index
    }

    pub(crate) fn letter(&self) -> char {
        self.letter
    }

    pub(crate) fn is_folder(&self) -> bool {
        self.folder
    }

    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    pub(crate) fn len(&self) -> usize {
        self.live
    }

    pub(super) fn name_bytes(&self) -> usize {
        self.names.len()
    }

    pub(super) fn footprint(&self) -> usize {
        self.nodes.len() * size_of::<Node>() + self.names.len() + self.lower.len()
    }

    pub(crate) fn root_id(&self) -> u64 {
        self.root_id
    }

    pub(crate) fn checkpoint(&self) -> Option<Checkpoint> {
        self.checkpoint
    }

    pub(crate) fn set_checkpoint(&mut self, checkpoint: Checkpoint) {
        self.checkpoint = Some(checkpoint);
    }

    pub(crate) fn details_complete(&self) -> bool {
        self.details.complete
    }

    pub(crate) fn complete_details(&mut self) {
        self.details.complete = true;
    }

    pub(crate) fn meta(&self, index: u32) -> Option<Meta> {
        let size = *self.details.sizes.get(index as usize)?;
        let modified = *self.details.times.get(index as usize)?;
        Some(Meta { size, modified }).filter(|meta| meta.known())
    }

    pub(crate) fn set_meta(&mut self, index: u32, meta: Meta, hidden: bool) {
        if let (Some(size), Some(time)) =
            (self.details.sizes.get_mut(index as usize), self.details.times.get_mut(index as usize))
        {
            *size = meta.size;
            *time = meta.modified;
        }
        if let Some(node) = self.nodes.get_mut(index as usize) {
            node.flags = if hidden { node.flags | HIDDEN } else { node.flags & !HIDDEN };
        }
    }

    pub(crate) fn take_stale(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.details.stale)
    }

    pub(crate) fn for_each_record(&self, mut visit: impl FnMut(Record)) {
        let mut ids = vec![0u64; self.nodes.len()];
        for (id, index) in self.ids.iter() {
            if let Some(slot) = ids.get_mut(index as usize) {
                *slot = id;
            }
        }
        for (index, node) in self.nodes.iter().enumerate().skip(1) {
            let (Some(&id), Some(&parent)) = (ids.get(index), ids.get(node.parent as usize)) else { continue };
            if node.persistent() && id != 0 {
                visit(Record {
                    id,
                    parent,
                    name: self.slice(&self.names, node.name, node.name_len),
                    dir: node.has(DIR),
                    hidden: node.has(HIDDEN),
                    meta: Meta {
                        size: self.details.sizes.get(index).copied().unwrap_or(0),
                        modified: self.details.times.get(index).copied().unwrap_or(0),
                    },
                });
            }
        }
    }

    pub(crate) fn needs_compaction(&self) -> bool {
        self.waste >= COMPACT_MIN_WASTE && self.waste * COMPACT_SHARE >= self.footprint()
    }

    pub(crate) fn compacted(&self) -> Volume {
        let mut builder = Builder::new(self.letter, self.root_id);
        self.for_each_record(|record| builder.push_record(record));
        builder.finish(self.checkpoint, self.details.complete)
    }

    pub(crate) fn insert(&mut self, parent: u32, name: &str, dir: bool) -> u32 {
        self.insert_entry(parent, name, dir, false, Meta::default())
    }

    pub(crate) fn insert_entry(&mut self, parent: u32, name: &str, dir: bool, hidden: bool, meta: Meta) -> u32 {
        self.make_room(1, name.len());
        let mut node = self.node(parent, name, dir, hidden);
        node.flags |= self.inherited(parent, name);
        if dir {
            self.forget_missing();
        }
        self.live += 1;
        self.place(node, meta)
    }

    pub(crate) fn apply(&mut self, changes: &[Change]) {
        let mut moved = Vec::new();
        for change in changes {
            match change {
                Change::Remove(id) => self.remove(*id),
                Change::Upsert { id, parent, name, dir, hidden } => {
                    moved.extend(self.upsert(*id, *parent, name, *dir, *hidden))
                }
                Change::Touch(id) => {
                    if let Some(index) = self.ids.get(*id) {
                        self.details.stale.push(index);
                        if !self.is_dir(index) {
                            self.edits.now.push(index);
                        }
                    }
                }
            }
        }
        if !moved.is_empty() {
            self.reclassify(&moved);
        }
    }

    fn upsert(&mut self, id: u64, parent_id: u64, name: &str, dir: bool, hidden: bool) -> Option<u32> {
        let parent = self.ids.get(parent_id)?;
        let Some(index) = self.ids.get(id) else {
            let index = self.insert_entry(parent, name, dir, hidden, Meta::default());
            if let Some(reused) = self.ids.insert(id, index) {
                if let Some(&node) = self.nodes.get(reused as usize) {
                    self.edits.before.push(node);
                }
                self.bury(reused);
            }
            self.details.stale.push(index);
            if !dir {
                self.edits.now.push(index);
            }
            return None;
        };
        let old = *self.nodes.get(index as usize)?;
        let same_name = self.slice(&self.names, old.name, old.name_len) == name;
        if same_name && old.parent == parent && old.has(DIR) == dir && !old.has(DEAD) {
            if old.has(HIDDEN) != hidden
                && let Some(node) = self.nodes.get_mut(index as usize)
            {
                node.flags ^= HIDDEN;
                self.edits.now.push(index);
            }
            return None;
        }
        self.edits.before.push(old);
        self.edits.now.push(index);
        self.make_room(0, name.len());
        let mut node = self.node(parent, name, dir, hidden);
        node.flags |= self.inherited(parent, name);
        let letters = self.letters_of(node);
        let (Some(slot), Some(known)) = (self.nodes.get_mut(index as usize), self.letters.get_mut(index as usize))
        else {
            return None;
        };
        (*slot, *known) = (node, letters);
        if dir {
            self.forget_missing();
        }
        if old.has(DEAD) {
            self.live += 1;
            self.details.stale.push(index);
        }
        self.waste += old.arena_bytes();
        let affects_children = (old.flags ^ node.flags) & INHERITED != 0 || old.parent == ROOT || parent == ROOT;
        (dir && affects_children).then_some(index)
    }

    fn remove(&mut self, id: u64) {
        if let Some(index) = self.ids.remove(id) {
            if let Some(&node) = self.nodes.get(index as usize) {
                self.edits.before.push(node);
            }
            self.bury(index);
        }
    }

    fn bury(&mut self, index: u32) {
        if let Some(node) = self.nodes.get_mut(index as usize)
            && !node.has(DEAD)
        {
            node.flags |= DEAD;
            self.live -= 1;
            self.waste += size_of::<Node>() + node.arena_bytes();
        }
    }

    pub(crate) fn lower_name(&self, index: u32) -> &str {
        self.nodes.get(index as usize).map_or("", |node| self.slice(&self.lower, node.lower, node.lower_len))
    }

    pub(crate) fn name(&self, index: u32) -> &str {
        self.nodes.get(index as usize).map_or("", |node| self.slice(&self.names, node.name, node.name_len))
    }

    pub(crate) fn is_dir(&self, index: u32) -> bool {
        self.nodes.get(index as usize).is_some_and(|node| node.has(DIR))
    }

    pub(crate) fn is_hidden(&self, index: u32) -> bool {
        self.nodes.get(index as usize).is_some_and(|node| node.has(HIDDEN))
    }

    pub(crate) fn path(&self, index: u32) -> String {
        let mut parts = Vec::new();
        let mut current = index;
        while current != NONE && parts.len() < MAX_DEPTH {
            let Some(node) = self.nodes.get(current as usize) else { break };
            parts.push(self.name(current));
            current = node.parent;
        }
        parts.reverse();
        if parts.len() == 1 { format!("{}\\", parts.concat()) } else { parts.join("\\") }
    }

    pub(crate) fn depth(&self, index: u32) -> usize {
        let mut depth = 0;
        let mut current = index;
        while let Some(node) = self.nodes.get(current as usize)
            && node.parent != NONE
            && depth < MAX_DEPTH
        {
            depth += 1;
            current = node.parent;
        }
        depth
    }

    fn place(&mut self, node: Node, meta: Meta) -> u32 {
        let letters = self.letters_of(node);
        self.letters.push(letters);
        self.push(node, meta)
    }

    fn spell_out(&mut self) {
        let mut letters = Vec::with_capacity(self.nodes.len() + self.nodes.len() / HEADROOM);
        letters.par_extend(self.nodes.par_iter().map(|&node| self.letters_of(node)));
        self.letters = letters;
    }

    fn letters_of(&self, node: Node) -> u32 {
        search::alphabet(self.slice(&self.lower, node.lower, node.lower_len).as_bytes())
    }

    fn slice<'a>(&self, arena: &'a str, start: u32, len: u16) -> &'a str {
        let start = start as usize;
        arena.get(start..start + len as usize).unwrap_or("")
    }

    fn node(&mut self, parent: u32, name: &str, dir: bool, hidden: bool) -> Node {
        let name_start = self.names.len() as u32;
        self.names.push_str(name);
        let lower_start = self.lower.len() as u32;
        let ascii = name.is_ascii();
        if ascii {
            self.lower.push_str(name);
            if let Some(added) = self.lower.get_mut(lower_start as usize..) {
                added.make_ascii_lowercase();
            }
        } else {
            self.lower.extend(name.chars().flat_map(char::to_lowercase));
        }
        Node {
            parent,
            name: name_start,
            lower: lower_start,
            name_len: (self.names.len() as u32 - name_start) as u16,
            lower_len: (self.lower.len() as u32 - lower_start) as u16,
            flags: if dir { DIR } else { 0 } | if ascii { ASCII } else { 0 } | if hidden { HIDDEN } else { 0 },
        }
    }

    fn inherited(&self, parent: u32, name: &str) -> u8 {
        let Some(node) = self.nodes.get(parent as usize) else {
            return EXCLUDED;
        };
        let own = own_flags(parent == ROOT, node.parent == ROOT, self.name(parent), name);
        if parent == ROOT { own } else { node.flags & INHERITED | own }
    }

    pub(crate) fn hidden_by_rules(names: &[&str]) -> Hidden {
        let flags = names.iter().enumerate().fold(0, |flags, (depth, &name)| {
            let parent = depth.checked_sub(1).and_then(|parent| names.get(parent)).copied().unwrap_or_default();
            flags | own_flags(depth == 0, depth == 1, parent, name)
        });
        match (flags & RECYCLED != 0, flags & EXCLUDED != 0) {
            (true, _) => Hidden::Recycled,
            (false, true) => Hidden::System,
            (false, false) => Hidden::Shown,
        }
    }

    fn mark_inherited(&mut self) {
        self.top_down(|volume, index| {
            let Some(node) = volume.nodes.get(index).copied() else { return };
            if node.has(ORPHAN) {
                return;
            }
            let inherited = volume.inherited(node.parent, volume.name(index as u32));
            if let Some(node) = volume.nodes.get_mut(index) {
                node.flags |= inherited;
            }
        });
    }

    fn reclassify(&mut self, moved: &[u32]) {
        let mut inside = vec![false; self.nodes.len()];
        for &index in moved {
            if let Some(flag) = inside.get_mut(index as usize) {
                *flag = true;
            }
        }
        self.top_down(|volume, index| {
            let Some(node) = volume.nodes.get(index).copied() else { return };
            let below_moved = inside.get(node.parent as usize).copied().unwrap_or(false);
            let Some(here) = inside.get_mut(index) else { return };
            if !*here && !below_moved {
                return;
            }
            *here = true;
            let inherited = volume.inherited(node.parent, volume.name(index as u32));
            if let Some(node) = volume.nodes.get_mut(index) {
                node.flags = node.flags & !INHERITED | inherited;
            }
        });
    }

    fn top_down(&mut self, mut visit: impl FnMut(&mut Self, usize)) {
        let mut resolved = vec![false; self.nodes.len()];
        if let Some(root) = resolved.first_mut() {
            *root = true;
        }
        let mut chain = Vec::new();
        for start in 1..self.nodes.len() {
            let mut current = start;
            while let Some(false) = resolved.get(current).copied() {
                if chain.len() > MAX_DEPTH {
                    break;
                }
                chain.push(current);
                current = self.nodes.get(current).map_or(0, |node| node.parent as usize);
            }
            while let Some(index) = chain.pop() {
                visit(self, index);
                if let Some(done) = resolved.get_mut(index) {
                    *done = true;
                }
            }
        }
    }
}

fn own_flags(parent_is_root: bool, grandparent_is_root: bool, parent: &str, name: &str) -> u8 {
    if parent_is_root {
        if name.eq_ignore_ascii_case(RECYCLE_BIN) {
            return EXCLUDED | RECYCLED;
        }
        let system = name.starts_with('$')
            || name.eq_ignore_ascii_case("System Volume Information")
            || name.eq_ignore_ascii_case("Config.Msi");
        return if system { EXCLUDED } else { 0 };
    }
    let component_store =
        grandparent_is_root && parent.eq_ignore_ascii_case("Windows") && name.eq_ignore_ascii_case("WinSxS");
    let start_menu = name.eq_ignore_ascii_case("Programs") && parent.eq_ignore_ascii_case("Start Menu");
    (if component_store { EXCLUDED } else { 0 }) | if start_menu { PROGRAMS } else { 0 }
}
