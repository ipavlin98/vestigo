use crate::i18n::Stamp;
use crate::index::Hit;
use crate::platform;
use rustc_hash::FxHashMap;
use std::cmp::Ordering;
use std::time::{Duration, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Column {
    Name,
    Folder,
    Size,
    Modified,
}

impl Column {
    pub(super) const ALL: [Self; 4] = [Self::Name, Self::Folder, Self::Size, Self::Modified];

    pub(super) fn align(self) -> Align {
        match self {
            Self::Name | Self::Folder | Self::Modified => Align::Start,
            Self::Size => Align::End,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Align {
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sort {
    pub(super) column: Column,
    pub(super) descending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Details {
    pub(super) size: Option<u64>,
    pub(super) modified: Option<Stamp>,
}

pub(super) type DetailMap = FxHashMap<String, Details>;

pub(super) fn arrange(mut hits: Vec<Hit>, sort: Option<Sort>) -> (Vec<Hit>, DetailMap) {
    let details = details_of(&hits);
    if let Some(sort) = sort {
        hits.sort_by(|left, right| {
            let order = compare(sort.column, left, right, &details);
            if sort.descending { order.reverse() } else { order }
        });
    }
    (hits, details)
}

fn compare(column: Column, left: &Hit, right: &Hit, details: &DetailMap) -> Ordering {
    let detail = |hit: &Hit| details.get(&hit.path).copied().unwrap_or_default();
    let text = |left: &str, right: &str| left.to_lowercase().cmp(&right.to_lowercase());
    match column {
        Column::Name => text(left.name(), right.name()),
        Column::Folder => text(left.folder(), right.folder()).then_with(|| text(left.name(), right.name())),
        Column::Size => detail(left).size.cmp(&detail(right).size),
        Column::Modified => detail(left).modified.map(stamp_key).cmp(&detail(right).modified.map(stamp_key)),
    }
}

fn stamp_key(stamp: Stamp) -> (u16, u8, u8, u8, u8) {
    (stamp.year, stamp.month, stamp.day, stamp.hour, stamp.minute)
}

fn details_of(hits: &[Hit]) -> DetailMap {
    hits.iter()
        .filter_map(|hit| {
            let details = match hit.modified {
                Some(seconds) => {
                    let time = UNIX_EPOCH + Duration::from_secs(u64::try_from(seconds).ok()?);
                    Details { size: hit.size, modified: platform::local_time(time) }
                }
                None => {
                    let metadata = std::fs::metadata(&hit.path).ok()?;
                    let size = metadata.is_file().then_some(metadata.len());
                    Details { size, modified: metadata.modified().ok().and_then(platform::local_time) }
                }
            };
            Some((hit.path.clone(), details))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(Hit::name).collect()
    }

    #[test]
    fn sorting_by_name_ignores_case_and_flips() {
        let hits = vec![
            Hit::new("C:\\b\\beta.txt".into(), false),
            Hit::new("C:\\a\\Alpha.txt".into(), false),
            Hit::new("C:\\c\\альфа.txt".into(), false),
        ];
        let (ascending, _) = arrange(hits.clone(), Some(Sort { column: Column::Name, descending: false }));
        assert_eq!(names(&ascending), ["Alpha.txt", "beta.txt", "альфа.txt"]);
        let (descending, _) = arrange(hits.clone(), Some(Sort { column: Column::Name, descending: true }));
        assert_eq!(names(&descending), ["альфа.txt", "beta.txt", "Alpha.txt"]);
        let (relevance, _) = arrange(hits, None);
        assert_eq!(names(&relevance), ["beta.txt", "Alpha.txt", "альфа.txt"]);
    }

    #[test]
    fn sorting_by_folder_breaks_ties_by_name() {
        let hits = vec![
            Hit::new("C:\\Work\\b.txt".into(), false),
            Hit::new("C:\\apps\\z.txt".into(), false),
            Hit::new("C:\\work\\a.txt".into(), false),
        ];
        let (sorted, _) = arrange(hits, Some(Sort { column: Column::Folder, descending: false }));
        assert_eq!(names(&sorted), ["z.txt", "a.txt", "b.txt"]);
    }
}
