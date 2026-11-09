use super::{ASCII, Area, DIR, FOCUS_BONUS, HIDDEN, MAX_DEPTH, Node, PROGRAMS, Prefer, ROOT, SCAN_CHUNK, Volume};
use crate::index::Token;
use crate::index::facets::{Counts, Sorter};
use crate::search::{self, Name, Query, Tolerance, Words};
use rayon::prelude::*;

struct Scopes<'a> {
    within: Vec<u32>,
    outside: Vec<u32>,
    focus: Option<u32>,
    area: Area<'a>,
}

impl Scopes<'_> {
    fn may_hold(&self, parent: u32) -> bool {
        match self.area {
            Area::Children(folders) => folders.contains(&parent),
            Area::Everywhere | Area::Subtree(_) => true,
        }
    }

    fn lineage(&self, volume: &Volume, parent: u32) -> Lineage {
        let mut placed = match self.area {
            Area::Everywhere => true,
            Area::Children(folders) => folders.contains(&parent),
            Area::Subtree(_) => false,
        };
        let mut lineage = Lineage { inside: placed && self.within.is_empty(), outside: false, preferred: false };
        let walks = !self.within.is_empty()
            || !self.outside.is_empty()
            || self.focus.is_some()
            || !placed && matches!(self.area, Area::Subtree(_));
        if !walks {
            return lineage;
        }
        let mut found = 0;
        let mut current = parent;
        for _ in 0..MAX_DEPTH {
            let Some(node) = volume.nodes.get(current as usize) else { break };
            found += usize::from(self.within.contains(&current));
            lineage.outside |= self.outside.contains(&current);
            lineage.preferred |= self.focus == Some(current);
            if let Area::Subtree(folders) = self.area {
                placed |= folders.contains(&current);
            }
            current = node.parent;
        }
        lineage.inside = placed && found == self.within.len();
        lineage
    }
}

#[derive(Clone, Copy)]
struct Lineage {
    inside: bool,
    outside: bool,
    preferred: bool,
}

pub(crate) struct Found {
    pub(crate) total: usize,
    pub(crate) best: Vec<(i32, u32)>,
    pub(crate) counts: Counts,
}

impl Found {
    fn keep(&mut self, scored: (i32, u32), limit: usize) {
        self.total += 1;
        self.best.push(scored);
        if self.best.len() >= limit.saturating_mul(2) {
            trim(&mut self.best, limit);
        }
    }
}

fn trim(best: &mut Vec<(i32, u32)>, limit: usize) {
    if best.len() > limit {
        best.select_nth_unstable_by(limit, |a, b| b.0.cmp(&a.0));
        best.truncate(limit);
    }
}

#[derive(Default)]
struct Paths<'a> {
    parent: Option<u32>,
    parts: Vec<&'a str>,
    prefix: String,
    within: bool,
    path: String,
}

impl<'a> Paths<'a> {
    fn score(&mut self, volume: &'a Volume, query: &Query, node: Node, name: &str) -> Option<i32> {
        if self.parent != Some(node.parent) {
            self.parent = Some(node.parent);
            self.parts.clear();
            let mut current = node.parent;
            while self.parts.len() < MAX_DEPTH {
                let Some(ancestor) = volume.nodes.get(current as usize) else { break };
                self.parts.push(volume.slice(&volume.lower, ancestor.lower, ancestor.lower_len));
                current = ancestor.parent;
            }
            self.prefix.clear();
            for part in self.parts.iter().rev() {
                self.prefix.push_str(part);
                self.prefix.push('\\');
            }
            self.within = query.paths_within(&self.prefix);
        }
        if !self.within {
            return None;
        }
        self.path.clear();
        self.path.push_str(&self.prefix);
        self.path.push_str(name);
        Some(query.path_score(&self.path))
    }
}

#[derive(Default)]
struct Folders(Option<(u32, Words, bool)>);

impl Folders {
    fn hold(&mut self, volume: &Volume, query: &Query, parent: u32, rest: Words) -> bool {
        if let Some((known, words, held)) = self.0
            && known == parent
            && words == rest
        {
            return held;
        }
        let mut left = rest;
        let mut current = parent;
        for _ in 0..MAX_DEPTH {
            let Some(folder) = volume.nodes.get(current as usize) else { break };
            left = query.in_folder(left, volume.slice(&volume.lower, folder.lower, folder.lower_len));
            if left.is_empty() || current == ROOT {
                break;
            }
            current = folder.parent;
        }
        self.0 = Some((parent, rest, left.is_empty()));
        left.is_empty()
    }
}

#[derive(Default)]
struct Ancestry(Option<(u32, Lineage)>);

impl Ancestry {
    fn of(&mut self, volume: &Volume, scopes: &Scopes, parent: u32) -> Lineage {
        match self.0 {
            Some((known, lineage)) if known == parent => lineage,
            Some(_) | None => {
                let lineage = scopes.lineage(volume, parent);
                self.0 = Some((parent, lineage));
                lineage
            }
        }
    }
}

impl Volume {
    #[cfg(test)]
    pub(crate) fn matches(&self, query: &Query, prefer: Prefer, excluded: &[String]) -> Vec<(i32, u32)> {
        self.find(query, prefer, Area::Everywhere, excluded, None, usize::MAX, &Token::default()).best
    }

    #[expect(clippy::too_many_arguments, reason = "one scan takes every search input")]
    pub(crate) fn find(
        &self,
        query: &Query,
        prefer: Prefer,
        area: Area,
        excluded: &[String],
        sorter: Option<&Sorter>,
        limit: usize,
        token: &Token,
    ) -> Found {
        let empty = || Found { total: 0, best: Vec::new(), counts: sorter.map(Sorter::counts).unwrap_or_default() };
        let Some(scopes) = self.scopes(query, prefer, area, excluded) else { return empty() };
        let wanted = query.letters();
        let parts: Vec<Found> = self
            .nodes
            .par_chunks(SCAN_CHUNK)
            .enumerate()
            .map(|(chunk, nodes)| {
                let mut found = empty();
                let base = (chunk * SCAN_CHUNK) as u32;
                let nodes: &[Node] = if token.cancelled() { &[] } else { nodes };
                let letters = self.letters.get(base as usize..base as usize + nodes.len()).unwrap_or_default();
                let mut ancestry = Ancestry::default();
                let mut paths = Paths::default();
                let mut folders = Folders::default();
                for (offset, (node, &letters)) in nodes.iter().zip(letters).enumerate() {
                    let index = base + offset as u32;
                    if !node.visible() || query.needs_full_path() && index == ROOT || !scopes.may_hold(node.parent) {
                        continue;
                    }
                    let tolerance = tolerance(prefer, *node);
                    if tolerance == Tolerance::Strict && !wanted.admit(letters) {
                        continue;
                    }
                    let (name, lower) = self.view(*node);
                    let Some((scored, rest)) = query.score_entry(name, node.has(DIR), tolerance) else {
                        continue;
                    };
                    let lineage = ancestry.of(self, &scopes, node.parent);
                    if !self.fits(query, index, *node, &scopes, lineage)
                        || !rest.is_empty() && !folders.hold(self, query, node.parent, rest)
                    {
                        continue;
                    }
                    let path = if query.needs_full_path() { paths.score(self, query, *node, lower) } else { Some(0) };
                    let Some(path) = path else {
                        continue;
                    };
                    if sorter.is_none_or(|sorter| sorter.sort(self, index, &mut found.counts)) {
                        let score = scored.with_focus(self.focus_bonus(prefer, *node, lower, lineage)) + path;
                        found.keep((score, index), limit);
                    }
                }
                found
            })
            .collect();
        let mut found = empty();
        for part in parts {
            found.total += part.total;
            found.counts.merge(&part.counts);
            found.best.extend(part.best);
        }
        trim(&mut found.best, limit);
        found
    }

    pub(crate) fn contents(
        &self,
        folder: u32,
        query: &Query,
        excluded: &[String],
        sorter: Option<&Sorter>,
        hidden_last: bool,
        token: &Token,
    ) -> Found {
        let mut found =
            self.find(query, Prefer::Nothing, Area::Children(&[folder]), excluded, sorter, usize::MAX, token);
        found.best.sort_by(|&(_, left), &(_, right)| {
            self.listing_order(left, hidden_last).cmp(&self.listing_order(right, hidden_last))
        });
        found
    }

    fn listing_order(&self, index: u32, hidden_last: bool) -> (bool, bool, &str) {
        let Some(&node) = self.nodes.get(index as usize) else { return (true, true, "") };
        (!node.has(DIR), hidden_last && node.has(HIDDEN), self.slice(&self.lower, node.lower, node.lower_len))
    }

    fn scopes<'a>(&self, query: &Query, prefer: Prefer, area: Area<'a>, excluded: &[String]) -> Option<Scopes<'a>> {
        let mut within: Vec<u32> = query.folders().map(|folder| self.locate(folder)).collect::<Option<_>>()?;
        within.sort_unstable();
        within.dedup();
        let outside = excluded.iter().filter_map(|folder| self.locate(folder)).collect();
        let focus = match prefer {
            Prefer::Within(folder) => Some(folder),
            Prefer::Programs | Prefer::Nothing => None,
        };
        Some(Scopes { within, outside, focus, area })
    }

    fn fits(&self, query: &Query, index: u32, node: Node, scopes: &Scopes, lineage: Lineage) -> bool {
        let details = || {
            let meta = self.meta(index).map(|meta| (meta.size, i64::from(meta.modified)));
            query.accepts_details(node.has(DIR), meta)
        };
        lineage.inside && !lineage.outside && !scopes.outside.contains(&index) && (!query.needs_details() || details())
    }

    fn view(&self, node: Node) -> (Name<'_>, &str) {
        let lower = self.slice(&self.lower, node.lower, node.lower_len);
        let start = node.name as usize;
        let original = self.names.as_bytes().get(start..start + usize::from(node.name_len)).unwrap_or_default();
        (Name::new(original, lower, node.has(ASCII)), lower)
    }

    fn focus_bonus(&self, prefer: Prefer, node: Node, lower: &str, lineage: Lineage) -> i32 {
        let preferred = match prefer {
            Prefer::Programs => node.has(PROGRAMS) && !node.has(DIR) && search::is_app(lower),
            Prefer::Within(_) => lineage.preferred,
            Prefer::Nothing => false,
        };
        if preferred { FOCUS_BONUS } else { 0 }
    }
}

fn tolerance(prefer: Prefer, node: Node) -> Tolerance {
    let close = match prefer {
        Prefer::Programs => node.has(PROGRAMS),
        Prefer::Within(folder) => node.parent == folder,
        Prefer::Nothing => false,
    };
    if close { Tolerance::Loose } else { Tolerance::Strict }
}
