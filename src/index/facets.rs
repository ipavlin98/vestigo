use super::volume::Volume;
use crate::search::{Kinds, Rule};
use std::ops::RangeInclusive;

#[derive(Clone, Debug, Default)]
pub(crate) struct Facets {
    pub(crate) kinds: Vec<Rule>,
    pub(crate) dates: Vec<RangeInclusive<i64>>,
    pub(crate) chosen: u64,
    pub(crate) date: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) kinds: Vec<usize>,
    pub(crate) dates: Vec<usize>,
    pub(crate) any_kind: usize,
    pub(crate) any_date: usize,
}

impl Counts {
    pub(super) fn new(facets: &Facets) -> Self {
        Self { kinds: vec![0; facets.kinds.len()], dates: vec![0; facets.dates.len()], any_kind: 0, any_date: 0 }
    }

    pub(super) fn merge(&mut self, other: &Self) {
        self.any_kind += other.any_kind;
        self.any_date += other.any_date;
        for (count, more) in self.kinds.iter_mut().zip(&other.kinds).chain(self.dates.iter_mut().zip(&other.dates)) {
            *count += more;
        }
    }
}

pub(super) struct Sorter<'a> {
    facets: &'a Facets,
    kinds: Kinds,
}

impl Facets {
    pub(crate) fn mask(positions: impl IntoIterator<Item = usize>) -> u64 {
        positions.into_iter().filter(|&position| position < Kinds::MAX).fold(0, |mask, position| mask | 1 << position)
    }

    pub(super) fn sorter(&self) -> Option<Sorter<'_>> {
        (!self.kinds.is_empty() || !self.dates.is_empty())
            .then(|| Sorter { facets: self, kinds: Kinds::new(&self.kinds) })
    }
}

impl Sorter<'_> {
    pub(super) fn counts(&self) -> Counts {
        Counts::new(self.facets)
    }

    pub(super) fn admits(&self, lower: &str, counts: &mut Counts) -> bool {
        self.place(self.kinds.of(lower, false), None, counts)
    }

    pub(super) fn sort_file(&self, path: &str, modified: u32, counts: &mut Counts) -> bool {
        let lower = crate::search::fold::lowered(path.rsplit('\\').next().unwrap_or(path));
        self.place(self.kinds.of(&lower, false), Some(i64::from(modified)), counts)
    }

    pub(super) fn sort(&self, volume: &Volume, index: u32, counts: &mut Counts) -> bool {
        let kinds = self.kinds.of(volume.lower_name(index), volume.is_dir(index));
        let modified = volume.meta(index).map(|meta| i64::from(meta.modified));
        self.place(kinds, modified, counts)
    }

    fn place(&self, kinds: u64, modified: Option<i64>, counts: &mut Counts) -> bool {
        let dates =
            mask(self.facets.dates.iter().map(|range| modified.is_some_and(|modified| range.contains(&modified))));
        let kind_fits = self.facets.chosen == 0 || kinds & self.facets.chosen != 0;
        let date_fits = self.facets.date.is_none_or(|date| has(dates, date));
        if date_fits {
            counts.any_kind += 1;
            add(&mut counts.kinds, kinds);
        }
        if kind_fits {
            counts.any_date += 1;
            add(&mut counts.dates, dates);
        }
        kind_fits && date_fits
    }
}

fn mask(fits: impl Iterator<Item = bool>) -> u64 {
    fits.take(Kinds::MAX).enumerate().fold(0, |mask, (position, fit)| if fit { mask | 1 << position } else { mask })
}

fn has(mask: u64, position: usize) -> bool {
    position < Kinds::MAX && mask & (1 << position) != 0
}

fn add(counts: &mut [usize], mask: u64) {
    for (position, count) in counts.iter_mut().enumerate().take(Kinds::MAX) {
        if has(mask, position) {
            *count += 1;
        }
    }
}
