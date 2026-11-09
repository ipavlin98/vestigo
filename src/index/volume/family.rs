use super::{DEAD, DIR, Meta, Volume};
use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::time::UNIX_EPOCH;

const HIDDEN_ATTRIBUTE: u32 = 0x2;
const SYSTEM_ATTRIBUTE: u32 = 0x4;

pub(crate) fn describe(metadata: &Metadata) -> (Meta, bool) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| u32::try_from(since.as_secs()).unwrap_or(u32::MAX));
    let size = if metadata.is_dir() { 0 } else { metadata.len() };
    let hidden = metadata.file_attributes() & (HIDDEN_ATTRIBUTE | SYSTEM_ATTRIBUTE) != 0;
    (Meta { size, modified }, hidden)
}

pub(crate) struct Family {
    serial: u64,
    order: Vec<u32>,
    starts: Vec<u32>,
    dirs: Vec<u32>,
}

impl Family {
    pub(crate) fn serial(&self) -> u64 {
        self.serial
    }

    pub(crate) fn dirs(&self) -> &[u32] {
        &self.dirs
    }

    pub(crate) fn children(&self, dir: u32) -> &[u32] {
        let start = self.starts.get(dir as usize).copied().unwrap_or(0) as usize;
        let end = self.starts.get(dir as usize + 1).copied().unwrap_or(0) as usize;
        self.order.get(start..end).unwrap_or_default()
    }
}

impl Volume {
    pub(crate) fn family(&self) -> Family {
        let count = self.nodes.len();
        let mut starts = vec![0u32; count + 1];
        let live = || self.nodes.iter().enumerate().skip(1).filter(|(_, node)| !node.has(DEAD));
        for (_, node) in live() {
            if let Some(slot) = starts.get_mut(node.parent as usize + 1) {
                *slot += 1;
            }
        }
        for position in 1..starts.len() {
            let before = starts.get(position - 1).copied().unwrap_or(0);
            if let Some(slot) = starts.get_mut(position) {
                *slot += before;
            }
        }
        let mut next = starts.clone();
        let mut order = vec![0u32; starts.last().copied().unwrap_or(0) as usize];
        for (index, node) in live() {
            if let Some(cursor) = next.get_mut(node.parent as usize)
                && let Some(slot) = order.get_mut(*cursor as usize)
            {
                *slot = index as u32;
                *cursor += 1;
            }
        }
        let dirs = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.has(DIR) && node.visible())
            .map(|(index, _)| index as u32)
            .collect();
        Family { serial: self.serial, order, starts, dirs }
    }
}
