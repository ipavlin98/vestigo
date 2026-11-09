use super::volume::{Hidden, Volume};
use super::{Index, contains};
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Diagnosis {
    Indexed { hidden: bool, low: bool },
    Missing,
    Gone,
    Uncovered,
    Scanning,
    System,
    Recycled,
    Excluded(String),
    Unseen { live: bool },
}

impl Index {
    pub(crate) fn diagnose(&self, path: &str, exists: bool) -> Diagnosis {
        let lower = path.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_lowercase();
        let scanning = self.scanning.load(Ordering::Relaxed) > 0;
        let volumes = self.volumes.read().clone();
        let covering = volumes
            .iter()
            .map(|volume| volume.read())
            .filter_map(|volume| volume.below_root(&lower).map(|names| (names.len(), volume)))
            .min_by_key(|&(depth, _)| depth);
        let Some((_, volume)) = covering else {
            return match (scanning, exists) {
                (true, _) => Diagnosis::Scanning,
                (false, false) => Diagnosis::Missing,
                (false, true) => Diagnosis::Uncovered,
            };
        };
        let found = volume.entry(&lower);
        if !exists {
            return if found.is_some() { Diagnosis::Gone } else { Diagnosis::Missing };
        }
        let names = volume.below_root(&lower).unwrap_or_default();
        match Volume::hidden_by_rules(&names) {
            Hidden::Recycled => return Diagnosis::Recycled,
            Hidden::System => return Diagnosis::System,
            Hidden::Shown => {}
        }
        let ranking = self.ranking.read().clone();
        let inside = |folders: &[String]| folders.iter().find(|folder| contains(folder, &lower)).cloned();
        if let Some(folder) = inside(&ranking.excluded) {
            return Diagnosis::Excluded(folder);
        }
        match found {
            Some(entry) => Diagnosis::Indexed {
                hidden: ranking.hidden_low && volume.is_hidden(entry),
                low: inside(&ranking.low).is_some(),
            },
            None if scanning => Diagnosis::Scanning,
            None => Diagnosis::Unseen { live: volume.checkpoint().is_some() },
        }
    }
}
