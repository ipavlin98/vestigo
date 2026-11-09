use super::storage::{fit, room};
use rustc_hash::FxHashMap;

const RECORD_BITS: u32 = 48;
const RECORD_MASK: u64 = (1 << RECORD_BITS) - 1;
const DENSE_RECORDS: u64 = 1 << 23;
const EMPTY: u32 = u32::MAX;

#[derive(Default)]
pub(super) struct Ids {
    nodes: Vec<u32>,
    sequences: Vec<u16>,
    spilled: FxHashMap<u64, u32>,
}

impl Ids {
    pub(super) fn reserve(&mut self, records: usize) {
        self.nodes.reserve_exact(records);
        self.sequences.reserve_exact(records);
    }

    pub(super) fn fit(&mut self) {
        fit(&mut self.nodes);
        fit(&mut self.sequences);
    }

    pub(super) fn get(&self, id: u64) -> Option<u32> {
        let Some((record, sequence)) = dense(id) else { return self.spilled.get(&id).copied() };
        let node = *self.nodes.get(record)?;
        (node != EMPTY && self.sequences.get(record) == Some(&sequence)).then_some(node)
    }

    pub(super) fn insert(&mut self, id: u64, node: u32) -> Option<u32> {
        let Some((record, sequence)) = dense(id) else { return self.spilled.insert(id, node) };
        if record >= self.nodes.len() {
            let more = record + 1 - self.nodes.len();
            room(&mut self.nodes, more);
            room(&mut self.sequences, more);
            self.nodes.resize(record + 1, EMPTY);
            self.sequences.resize(record + 1, 0);
        }
        let (Some(slot), Some(known)) = (self.nodes.get_mut(record), self.sequences.get_mut(record)) else {
            return None;
        };
        let replaced = std::mem::replace(slot, node);
        *known = sequence;
        (replaced != EMPTY).then_some(replaced)
    }

    pub(super) fn remove(&mut self, id: u64) -> Option<u32> {
        let Some((record, _)) = dense(id) else { return self.spilled.remove(&id) };
        let node = self.get(id)?;
        if let Some(slot) = self.nodes.get_mut(record) {
            *slot = EMPTY;
        }
        Some(node)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (u64, u32)> + '_ {
        let dense = self.nodes.iter().zip(&self.sequences).enumerate().filter(|(_, (node, _))| **node != EMPTY);
        dense
            .map(|(record, (&node, &sequence))| (record as u64 | u64::from(sequence) << RECORD_BITS, node))
            .chain(self.spilled.iter().map(|(&id, &node)| (id, node)))
    }
}

fn dense(id: u64) -> Option<(usize, u16)> {
    let record = id & RECORD_MASK;
    (record < DENSE_RECORDS).then_some((record as usize, (id >> RECORD_BITS) as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(record: u64, sequence: u64) -> u64 {
        record | sequence << RECORD_BITS
    }

    #[test]
    fn a_reused_record_answers_only_to_its_new_sequence() {
        let mut ids = Ids::default();
        assert_eq!(ids.insert(id(7, 1), 3), None);
        assert_eq!(ids.insert(id(7, 2), 4), Some(3));
        assert_eq!(ids.get(id(7, 1)), None);
        assert_eq!(ids.get(id(7, 2)), Some(4));
        assert_eq!(ids.remove(id(7, 1)), None);
        assert_eq!(ids.get(id(7, 2)), Some(4));
        assert_eq!(ids.remove(id(7, 2)), Some(4));
        assert_eq!(ids.remove(id(7, 2)), None);
        assert_eq!(ids.get(id(7, 2)), None);
    }

    #[test]
    fn huge_records_still_resolve() {
        let mut ids = Ids::default();
        let far = id(DENSE_RECORDS + 5, 1);
        ids.insert(far, 9);
        ids.insert(id(2, 1), 8);
        assert_eq!(ids.get(far), Some(9));
        let mut all: Vec<(u64, u32)> = ids.iter().collect();
        all.sort_unstable();
        assert_eq!(all, [(id(2, 1), 8), (far, 9)]);
        assert_eq!(ids.remove(far), Some(9));
        assert_eq!(ids.get(far), None);
    }
}
