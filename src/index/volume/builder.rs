use super::{Checkpoint, EXCLUDED, NONE, ORPHAN, ROOT, Record, Volume};

pub(crate) struct Builder {
    volume: Volume,
    parents: Vec<u64>,
    root: u64,
}

impl Builder {
    pub(crate) fn new(letter: char, root: u64) -> Self {
        Self::with_capacity(letter, root, 0, 0)
    }

    pub(crate) fn with_capacity(letter: char, root: u64, records: usize, name_bytes: usize) -> Self {
        let mut volume = Volume::new(letter);
        volume.reserve(records, name_bytes);
        volume.ids.insert(root, ROOT);
        volume.root_id = root;
        let mut parents = Vec::with_capacity(records + 1);
        parents.push(root);
        Self { volume, parents, root }
    }

    #[cfg(test)]
    pub(crate) fn push(&mut self, id: u64, parent: u64, name: &str, dir: bool) {
        self.push_record(Record { id, parent, name, dir, hidden: false, meta: super::Meta::default() });
    }

    pub(crate) fn push_record(&mut self, record: Record) {
        if record.id == self.root {
            return;
        }
        let node = self.volume.node(NONE, record.name, record.dir, record.hidden);
        let index = self.volume.push(node, record.meta);
        self.volume.ids.insert(record.id, index);
        self.parents.push(record.parent);
    }

    pub(crate) fn finish(self, checkpoint: Option<Checkpoint>, complete: bool) -> Volume {
        let Self { mut volume, parents, .. } = self;
        volume.checkpoint = checkpoint;
        volume.details.complete = complete;
        for (index, parent) in parents.iter().enumerate().skip(1) {
            let resolved = volume.ids.get(*parent);
            if let Some(node) = volume.nodes.get_mut(index) {
                match resolved {
                    Some(parent) => node.parent = parent,
                    None => {
                        node.parent = ROOT;
                        node.flags |= EXCLUDED | ORPHAN;
                    }
                }
            }
        }
        volume.spell_out();
        volume.fit();
        volume.mark_inherited();
        volume.live = volume.nodes.len() - 1;
        volume
    }
}
