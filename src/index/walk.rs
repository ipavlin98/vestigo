use super::volume::{ROOT, Volume};
use jwalk::{Parallelism, WalkDir};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

pub(super) fn scan(mut volume: Volume, root: &Path) -> Volume {
    let mut folders: FxHashMap<PathBuf, u32> = FxHashMap::default();
    folders.insert(root.to_path_buf(), ROOT);
    let walk = WalkDir::new(root)
        .skip_hidden(false)
        .follow_links(false)
        .min_depth(1)
        .parallelism(Parallelism::RayonNewPool(0));
    for entry in walk.into_iter().flatten() {
        let Some(&parent) = folders.get(entry.parent_path.as_ref()) else { continue };
        let name = entry.file_name.to_string_lossy();
        let dir = entry.file_type.is_dir();
        let index = volume.insert(parent, &name, dir);
        if dir {
            folders.insert(entry.path(), index);
        }
    }
    volume
}
