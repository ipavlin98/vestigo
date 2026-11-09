use super::volume::{Area, Prefer, ROOT, Volume};
use super::{Boosts, Counts, Facets, Focus, Hit, Index, Ranking, Results, SHORTLIST, Token, context_score};
use crate::search::{Listed, Query, Reach};
use parking_lot::RwLock;
use std::sync::Arc;

enum Scope {
    Everywhere,
    Children(Vec<Vec<u32>>),
    Subtree(Vec<Vec<u32>>),
}

impl Scope {
    fn listed(folders: &[Vec<u32>], volume: usize) -> Option<&[u32]> {
        folders.get(volume).map(Vec::as_slice).filter(|folders| !folders.is_empty())
    }

    fn area(&self, volume: usize) -> Option<Area<'_>> {
        match self {
            Self::Everywhere => Some(Area::Everywhere),
            Self::Children(folders) => Self::listed(folders, volume).map(Area::Children),
            Self::Subtree(folders) => Self::listed(folders, volume).map(Area::Subtree),
        }
    }
}

struct Search<'a> {
    query: &'a Query,
    boosts: &'a Boosts,
    focus: &'a Focus,
    limit: usize,
    facets: &'a Facets,
    token: &'a Token,
    ranking: Ranking,
}

impl Search<'_> {
    fn contents(&self, volumes: &[Arc<RwLock<Volume>>], folders: &[Vec<u32>]) -> Option<(Results, Counts)> {
        let mut counts = Counts::new(self.facets);
        let mut results = Results { listed: Listed::Contents, ..Results::default() };
        let listed =
            volumes.iter().zip(folders).flat_map(|(volume, folders)| folders.iter().map(move |&f| (volume, f)));
        for (volume, folder) in listed {
            let volume = volume.read();
            let sorter = self.facets.sorter();
            let excluded = &self.ranking.excluded;
            let found =
                volume.contents(folder, self.query, excluded, sorter.as_ref(), self.ranking.hidden_low, self.token);
            if self.token.cancelled() {
                return None;
            }
            results.total += found.total;
            counts.merge(&found.counts);
            let kept =
                self.query.kept().and_then(|name| found.best.iter().position(|&(_, at)| volume.lower_name(at) == name));
            let start =
                kept.map_or(0, |at| at.saturating_sub(self.limit / 2).min(found.best.len().saturating_sub(self.limit)));
            results
                .hits
                .extend(found.best.into_iter().skip(start).take(self.limit).map(|(_, index)| hit(&volume, index)));
        }
        Some((results, counts))
    }

    fn folders(
        &self,
        volumes: &[Arc<RwLock<Volume>>],
        folders: &[Vec<u32>],
        named: Option<&Query>,
    ) -> (Results, Counts) {
        let mut hits: Vec<Hit> = volumes
            .iter()
            .zip(folders)
            .flat_map(|(volume, folders)| {
                let volume = volume.read();
                folders.iter().map(|&folder| hit(&volume, folder)).collect::<Vec<_>>()
            })
            .collect();
        hits.sort_by_cached_key(|hit| (hit.path.matches('\\').count(), hit.path.to_lowercase()));
        let total = hits.len();
        hits.truncate(self.limit);
        for hit in &mut hits {
            hit.highlights = named.map(|query| query.highlights(hit.name())).unwrap_or_default();
        }
        (Results { hits, total, listed: Listed::Matches }, Counts::new(self.facets))
    }
}

fn hit(volume: &Volume, index: u32) -> Hit {
    let mut hit = Hit::new(volume.path(index), volume.is_dir(index));
    if let Some(meta) = volume.meta(index) {
        hit.size = (!hit.dir).then_some(meta.size);
        hit.modified = Some(i64::from(meta.modified));
    }
    hit
}

impl Index {
    pub(crate) fn search_until(
        &self,
        query: &Query,
        boosts: &Boosts,
        focus: &Focus,
        limit: usize,
        facets: &Facets,
        token: &Token,
    ) -> Option<(Results, Counts)> {
        if let Some(content) = query.content() {
            return self.search_content(query, content, boosts, limit, facets, token);
        }
        let volumes: Vec<Arc<RwLock<Volume>>> = self.volumes.read().clone();
        let search = Search { query, boosts, focus, limit, facets, token, ranking: self.ranking.read().clone() };
        let Some(place) = query.place() else { return self.matching(&volumes, &search, &Scope::Everywhere) };
        let path = place.path();
        let resolved = || -> Vec<Vec<u32>> {
            volumes.iter().map(|volume| volume.read().folders_at(place.anchor(), &path)).collect()
        };
        let tail = match place.reach() {
            Reach::Subtree => return self.matching(&volumes, &search, &Scope::Subtree(resolved())),
            Reach::Contents(tail) => tail,
        };
        let folders = resolved();
        match folders.iter().map(Vec::len).sum::<usize>() {
            0 => {}
            1 => return search.contents(&volumes, &folders),
            _ => return Some(search.folders(&volumes, &folders, tail.as_ref().map(|tail| &*tail.query))),
        }
        let Some(tail) = tail else { return Some((Results::default(), Counts::new(facets))) };
        let search = Search { query: &tail.query, ..search };
        let parents = place.parents();
        if place.anchor().is_none() && parents.is_empty() {
            return self.matching(&volumes, &search, &Scope::Everywhere);
        }
        let folders = volumes.iter().map(|volume| volume.read().folders_at(place.anchor(), &parents)).collect();
        self.matching(&volumes, &search, &Scope::Children(folders))
    }

    fn matching(&self, volumes: &[Arc<RwLock<Volume>>], search: &Search, scope: &Scope) -> Option<(Results, Counts)> {
        let Search { query, boosts, focus, limit, facets, token, ref ranking } = *search;
        let mut counts = Counts::new(facets);
        let sorter = facets.sorter();
        let mut total = 0;
        let mut ranked = Vec::new();
        for (position, volume) in volumes.iter().enumerate() {
            let Some(area) = scope.area(position) else { continue };
            let volume = volume.read();
            let prefer = match focus {
                Focus::Programs => Prefer::Programs,
                Focus::Folder(path) => {
                    volume.locate(path).filter(|&folder| folder != ROOT).map_or(Prefer::Nothing, Prefer::Within)
                }
                Focus::Anything => Prefer::Nothing,
            };
            let found = volume.find(query, prefer, area, &ranking.excluded, sorter.as_ref(), SHORTLIST, token);
            if token.cancelled() {
                return None;
            }
            total += found.total;
            counts.merge(&found.counts);
            ranked.extend(found.best.into_iter().map(|(score, index)| {
                let hit = hit(&volume, index);
                let hidden = volume.is_hidden(index);
                (score + context_score(&hit.path, volume.depth(index), hidden, boosts, ranking), hit)
            }));
        }
        if *focus == Focus::Programs && matches!(scope, Scope::Everywhere) {
            let apps = self.apps.read().matches(query, sorter.as_ref(), &mut counts, boosts, ranking);
            total += apps.len();
            ranked.extend(apps);
        }
        ranked.sort_by(|(a, left), (b, right)| b.cmp(a).then_with(|| left.path.len().cmp(&right.path.len())));
        let hits = ranked
            .into_iter()
            .take(limit)
            .map(|(_, mut hit)| {
                hit.highlights = query.highlights(hit.name());
                hit.marks = query.marks(hit.folder());
                hit
            })
            .collect();
        Some((Results { hits, total, listed: Listed::Matches }, counts))
    }
}
