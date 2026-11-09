use super::{DIR, Edits, HIDDEN, MAX_DEPTH, Meta, ORPHAN, ROOT, Volume};

const UNKNOWN: u8 = 0;
const CLEAN: u8 = 1;
const NOISY: u8 = 2;

impl Volume {
    pub(crate) fn documents(
        &self,
        noise: &[&str],
        wanted: impl Fn(&str, u64) -> bool,
        mut visit: impl FnMut(u32, Meta),
    ) {
        let mut folders = vec![UNKNOWN; self.nodes.len()];
        let mut chain = Vec::new();
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.visible() || node.has(DIR | ORPHAN | HIDDEN) {
                continue;
            }
            let lower = self.slice(&self.lower, node.lower, node.lower_len);
            let Some(meta) = self.meta(index as u32) else { continue };
            if wanted(lower, meta.size) && !self.in_noise(node.parent, noise, &mut folders, &mut chain) {
                visit(index as u32, meta);
            }
        }
    }

    pub(crate) fn take_edits(&mut self) -> Edits {
        std::mem::take(&mut self.edits)
    }

    pub(crate) fn edits_documents(&self, edits: &Edits, noise: &[&str], document: impl Fn(&str) -> bool) -> bool {
        let now = edits.now.iter().filter_map(|&index| self.nodes.get(index as usize).copied());
        now.chain(edits.before.iter().copied()).any(|node| {
            let lower = self.slice(&self.lower, node.lower, node.lower_len);
            let holds = if node.has(DIR) { !self.names_noise(lower, node.parent, noise) } else { document(lower) };
            node.visible() && !node.has(ORPHAN) && holds && !self.noisy(node.parent, noise)
        })
    }

    fn noisy(&self, folder: u32, noise: &[&str]) -> bool {
        let mut current = folder;
        for _ in 0..MAX_DEPTH {
            let Some(node) = self.nodes.get(current as usize).filter(|_| current != ROOT) else { return false };
            if self.is_noise(current, noise) {
                return true;
            }
            current = node.parent;
        }
        false
    }

    fn in_noise(&self, folder: u32, noise: &[&str], folders: &mut [u8], chain: &mut Vec<u32>) -> bool {
        chain.clear();
        let mut current = folder;
        let mut noisy = false;
        while current != ROOT && chain.len() < MAX_DEPTH {
            match folders.get(current as usize) {
                Some(&CLEAN) => break,
                Some(&NOISY) => {
                    noisy = true;
                    break;
                }
                Some(_) | None => {}
            }
            let Some(node) = self.nodes.get(current as usize) else { break };
            chain.push(current);
            current = node.parent;
        }
        while let Some(folder) = chain.pop() {
            noisy = noisy || self.is_noise(folder, noise);
            if let Some(known) = folders.get_mut(folder as usize) {
                *known = if noisy { NOISY } else { CLEAN };
            }
        }
        noisy
    }

    fn is_noise(&self, folder: u32, noise: &[&str]) -> bool {
        self.nodes
            .get(folder as usize)
            .is_some_and(|node| self.names_noise(self.lower_name(folder), node.parent, noise))
    }

    fn names_noise(&self, name: &str, parent: u32, noise: &[&str]) -> bool {
        noise.iter().any(|entry| match entry.split_once('\\') {
            Some((above, below)) => below == name && self.lower_name(parent) == above,
            None => *entry == name,
        })
    }
}
