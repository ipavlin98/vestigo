use super::{DEAD, DIR, PROGRAMS, ROOT, Volume, describe};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

const MAX_FILL_DEPTH: usize = 8;
const LOCATED_LIMIT: usize = 64;

impl Volume {
    pub(crate) fn fill(&mut self, folders: &[PathBuf]) {
        let mut known: FxHashMap<(u32, String), u32> = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.has(PROGRAMS) && !node.has(DEAD))
            .map(|(index, node)| {
                ((node.parent, self.slice(&self.lower, node.lower, node.lower_len).to_owned()), index as u32)
            })
            .collect();
        for folder in folders {
            if let Some(node) = folder.to_str().and_then(|path| self.locate(path)) {
                self.fill_folder(node, folder, &mut known, 0);
            }
        }
    }

    fn fill_folder(&mut self, node: u32, path: &Path, known: &mut FxHashMap<(u32, String), u32>, depth: usize) {
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for entry in entries.flatten() {
            let (Ok(kind), Some(name)) = (entry.file_type(), entry.file_name().to_str().map(str::to_owned)) else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let key = (node, name.chars().flat_map(char::to_lowercase).collect::<String>());
            let child = match known.get(&key) {
                Some(&child) => child,
                None => {
                    let (meta, hidden) = entry.metadata().map(|metadata| describe(&metadata)).unwrap_or_default();
                    let child = self.insert_entry(node, &name, kind.is_dir(), hidden, meta);
                    known.insert(key, child);
                    child
                }
            };
            if kind.is_dir() && depth < MAX_FILL_DEPTH {
                self.fill_folder(child, &entry.path(), known, depth + 1);
            }
        }
    }

    pub(crate) fn below_root<'a>(&self, lower: &'a str) -> Option<Vec<&'a str>> {
        let root = self.nodes.first().map_or("", |root| self.slice(&self.lower, root.lower, root.lower_len));
        let rest = lower.strip_prefix(root)?;
        let rest = if rest.is_empty() { rest } else { rest.strip_prefix('\\')? };
        Some(rest.split('\\').filter(|part| !part.is_empty()).collect())
    }

    pub(crate) fn entry(&self, lower: &str) -> Option<u32> {
        let wanted = self.below_root(lower)?;
        let Some(&name) = wanted.last() else { return Some(ROOT) };
        (1..self.nodes.len() as u32).into_par_iter().find_any(|&index| {
            self.nodes
                .get(index as usize)
                .is_some_and(|node| !node.has(DEAD) && self.slice(&self.lower, node.lower, node.lower_len) == name)
                && self.ancestors_are(index, &wanted)
        })
    }

    pub(crate) fn locate(&self, path: &str) -> Option<u32> {
        let lower: String = path.trim_end_matches('\\').chars().flat_map(char::to_lowercase).collect();
        let wanted = self.below_root(&lower)?;
        let Some(&name) = wanted.last() else { return Some(ROOT) };
        match self.located.lock().get(&lower).copied() {
            Some(Some(index)) if self.is_folder_at(index, &wanted) => return Some(index),
            Some(None) => return None,
            Some(Some(_)) | None => {}
        }
        let found = (1..self.nodes.len() as u32).into_par_iter().find_any(|&index| {
            self.nodes
                .get(index as usize)
                .is_some_and(|node| self.slice(&self.lower, node.lower, node.lower_len) == name)
                && self.is_folder_at(index, &wanted)
        });
        let mut located = self.located.lock();
        if located.len() >= LOCATED_LIMIT {
            located.clear();
        }
        located.insert(lower, found);
        found
    }

    pub(super) fn forget_missing(&mut self) {
        self.located.get_mut().retain(|_, found| found.is_some());
    }

    pub(crate) fn folders_at(&self, anchor: Option<&str>, names: &[&str]) -> Vec<u32> {
        if let Some(anchor) = anchor {
            return self.locate(&format!("{anchor}\\{}", names.join("\\"))).into_iter().collect();
        }
        let Some((&name, above)) = names.split_last() else { return Vec::new() };
        (1..self.nodes.len() as u32)
            .into_par_iter()
            .filter(|&index| {
                self.nodes.get(index as usize).is_some_and(|node| {
                    node.has(DIR)
                        && node.visible()
                        && self.slice(&self.lower, node.lower, node.lower_len) == name
                        && self.ends_with(node.parent, above)
                })
            })
            .collect()
    }

    fn ends_with(&self, index: u32, names: &[&str]) -> bool {
        let mut current = index;
        for &name in names.iter().rev() {
            let Some(node) = self.nodes.get(current as usize).filter(|_| current != ROOT) else { return false };
            if self.slice(&self.lower, node.lower, node.lower_len) != name {
                return false;
            }
            current = node.parent;
        }
        true
    }

    fn is_folder_at(&self, index: u32, names: &[&str]) -> bool {
        self.nodes.get(index as usize).is_some_and(|node| node.has(DIR) && !node.has(DEAD))
            && self.ancestors_are(index, names)
    }

    fn ancestors_are(&self, index: u32, names: &[&str]) -> bool {
        let mut current = index;
        for &name in names.iter().rev() {
            let Some(node) = self.nodes.get(current as usize) else { return false };
            if self.slice(&self.lower, node.lower, node.lower_len) != name {
                return false;
            }
            current = node.parent;
        }
        current == ROOT
    }
}
