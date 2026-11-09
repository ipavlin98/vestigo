use super::docs::Doc;
use super::library::Library;
use super::store::{OVERLAP, Scratch, with_scratch};
use crate::index::facets::Sorter;
use crate::index::{Boosts, Counts, Facets, Hit, Index, Ranking, Results, Snippet, Token, contains, context_score};
use crate::search::fold;
use crate::search::{ContentQuery, Listed, Name, Query, Tolerance, merged};
use anyhow::Result;
use memchr::memmem::{Finder, find_iter};
use rayon::prelude::*;
use std::ops::Range;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERIFY_BLOCKS: usize = 128;
const COUNT_BUDGET: Duration = Duration::from_millis(30);
const SNIPPET_BEFORE: usize = 24;
const SNIPPET_AFTER: usize = 200;
const ELLIPSIS: &str = "…";
const DAY_SECONDS: u32 = 24 * 60 * 60;
const FRESHNESS: i32 = 12;
const MAX_TERMS: usize = u64::BITS as usize;

struct Needle {
    finder: Finder<'static>,
    long: bool,
}

impl Needle {
    fn new(text: &str) -> Self {
        Self { finder: Finder::new(text.as_bytes()).into_owned(), long: text.len() > OVERLAP }
    }
}

pub(super) struct Probe<'a> {
    library: &'a Library,
    terms: Vec<Needle>,
    excluded: Vec<Needle>,
    wanted: Vec<u32>,
    every_block: bool,
    long: bool,
}

impl<'a> Probe<'a> {
    pub(super) fn new(library: &'a Library, content: &ContentQuery) -> Result<(Self, Vec<u32>)> {
        let mut docs: Option<Vec<u32>> = None;
        let mut wanted = Vec::new();
        for term in content.terms().iter().filter(|term| ContentQuery::has_grams(term)) {
            let blocks = library.grams.candidates(term)?;
            let mut holding: Vec<u32> = blocks.iter().filter_map(|&block| library.docs.doc_of(block)).collect();
            holding.dedup();
            docs = Some(match docs {
                Some(mut known) => {
                    known.retain(|doc| holding.binary_search(doc).is_ok());
                    known
                }
                None => holding,
            });
            wanted.extend(blocks);
        }
        wanted.sort_unstable();
        wanted.dedup();
        let terms: Vec<Needle> = content.terms().iter().take(MAX_TERMS).map(|term| Needle::new(term)).collect();
        let excluded: Vec<Needle> = content.excluded().iter().map(|term| Needle::new(term)).collect();
        let every_block = !excluded.is_empty() || !content.terms().iter().all(|term| ContentQuery::has_grams(term));
        let long = terms.iter().chain(&excluded).any(|needle| needle.long);
        Ok((Self { library, terms, excluded, wanted, every_block, long }, docs.unwrap_or_default()))
    }

    fn blocks_of(&self, doc: &Doc) -> &[u32] {
        let first = self.wanted.partition_point(|&block| block < doc.blocks.start);
        let end = self.wanted.partition_point(|&block| block < doc.blocks.end);
        self.wanted.get(first..end).unwrap_or_default()
    }

    fn cost(&self, doc: u32) -> usize {
        let Some(doc) = self.library.docs.get(doc) else { return 0 };
        if self.every_block { doc.blocks.len() } else { self.blocks_of(doc).len() }
    }

    fn scan(&self, doc: &Doc, id: u32, scratch: &mut Scratch) -> Result<()> {
        let store = &self.library.store;
        let Some(block) = self.library.docs.block(id) else { return Ok(()) };
        store.read(block, scratch)?;
        if self.long
            && id + 1 < doc.blocks.end
            && let Some(next) = self.library.docs.block(id + 1)
        {
            store.read_next(next, scratch)?;
            scratch.raw.truncate(block.own as usize);
            let Scratch { raw, next, .. } = scratch;
            raw.extend_from_slice(next);
        }
        Ok(())
    }

    fn verify(&self, doc: u32) -> Result<Option<u32>> {
        let Some(doc) = self.library.docs.get(doc).filter(|doc| doc.live()) else { return Ok(None) };
        if self.every_block {
            self.verify_blocks(doc, doc.blocks.clone())
        } else {
            self.verify_blocks(doc, self.blocks_of(doc).iter().copied())
        }
    }

    fn verify_blocks(&self, doc: &Doc, ids: impl Iterator<Item = u32>) -> Result<Option<u32>> {
        let mut missing = self.terms.iter().enumerate().fold(0u64, |mask, (bit, _)| mask | 1 << bit);
        let mut first = None;
        with_scratch(|scratch| {
            for id in ids {
                self.scan(doc, id, scratch)?;
                let text = scratch.folded();
                if self.excluded.iter().any(|needle| needle.finder.find(text).is_some()) {
                    return Ok(None);
                }
                for (bit, needle) in self.terms.iter().enumerate() {
                    if missing & 1 << bit != 0 && needle.finder.find(text).is_some() {
                        missing &= !(1 << bit);
                        first = first.or(Some(id));
                    }
                }
                if missing == 0 && self.excluded.is_empty() {
                    break;
                }
            }
            Ok(first.filter(|_| missing == 0))
        })
    }
}

pub(super) struct Verified {
    pub(super) found: Vec<(u32, u32)>,
    pub(super) total: usize,
    pub(super) exact: bool,
}

pub(super) struct Sifting<'a> {
    pub(super) limit: usize,
    pub(super) budget: Duration,
    pub(super) sorter: Option<&'a Sorter<'a>>,
    pub(super) token: &'a Token,
}

pub(super) fn sift(
    probe: &Probe,
    ranked: &[u32],
    sifting: &Sifting,
    counts: &mut Counts,
    started: Instant,
) -> Option<Verified> {
    let docs = &probe.library.docs;
    let mut verified = Verified { found: Vec::new(), total: 0, exact: true };
    let mut next = 0;
    while next < ranked.len() {
        if sifting.token.cancelled() {
            return None;
        }
        let (mut end, mut cost) = (next, 0);
        while let Some(&doc) = ranked.get(end).filter(|_| cost < VERIFY_BLOCKS) {
            cost += probe.cost(doc);
            end += 1;
        }
        let batch = ranked.get(next..end).unwrap_or_default();
        let passed: Vec<Option<u32>> = batch
            .par_iter()
            .map(|&doc| {
                probe.verify(doc).unwrap_or_else(|error| {
                    log::warn!("Reading the stored text failed: {error:#}");
                    None
                })
            })
            .collect();
        next = end;
        for (&doc, block) in batch.iter().zip(passed) {
            let (Some(block), Some(found)) = (block, docs.get(doc)) else { continue };
            let kept = sifting.sorter.is_none_or(|sorter| sorter.sort_file(&found.path, found.modified, counts));
            if kept {
                verified.total += 1;
                if verified.found.len() < sifting.limit {
                    verified.found.push((doc, block));
                }
            }
        }
        if verified.found.len() >= sifting.limit && started.elapsed() >= sifting.budget {
            verified.exact = next >= ranked.len();
            break;
        }
    }
    Some(verified)
}

fn snippet_of(text: &str, terms: &[String]) -> Snippet {
    let (lower, origins) = fold::with_origins(text);
    let first = terms
        .iter()
        .find_map(|term| {
            let at = lower.find(term.as_str())?;
            fold::origin(&origins, at..at + term.len())
        })
        .unwrap_or(0..0);
    let start =
        text.get(..first.start).and_then(|head| head.char_indices().rev().nth(SNIPPET_BEFORE - 1).map(|(at, _)| at));
    let start = start.unwrap_or(0);
    let end =
        text.get(first.end..).and_then(|tail| tail.char_indices().nth(SNIPPET_AFTER).map(|(at, _)| first.end + at));
    let end = end.unwrap_or(text.len());
    let lead = if start > 0 { ELLIPSIS } else { "" };
    let spans = terms.iter().flat_map(|term| {
        let found = find_iter(lower.as_bytes(), term.as_bytes());
        found.map(|at| at..at + term.len()).collect::<Vec<_>>()
    });
    let ranges: Vec<Range<usize>> = spans
        .filter_map(|span| fold::origin(&origins, span))
        .filter(|range| range.start >= start && range.end <= end)
        .map(|range| range.start - start + lead.len()..range.end - start + lead.len())
        .collect();
    let tail = if end < text.len() { ELLIPSIS } else { "" };
    Snippet { text: format!("{lead}{}{tail}", text.get(start..end).unwrap_or_default()), ranges: merged(ranges) }
}

pub(super) fn snippet(library: &Library, block: u32, terms: &[String]) -> Option<Snippet> {
    let block = library.docs.block(block)?;
    let text = with_scratch(|scratch| {
        library.store.read(block, scratch)?;
        Ok(String::from_utf8_lossy(&scratch.raw).into_owned())
    });
    match text {
        Ok(text) => Some(snippet_of(&text, terms)),
        Err(error) => {
            log::warn!("Reading the stored text failed: {error:#}");
            None
        }
    }
}

fn freshness(modified: u32, now: u32) -> i32 {
    let days = now.saturating_sub(modified) / DAY_SECONDS;
    FRESHNESS - (days + 1).ilog2() as i32
}

struct Sieve<'a> {
    query: &'a Query,
    boosts: &'a Boosts,
    places: Option<Vec<String>>,
    folders: Vec<String>,
    ranking: &'a Ranking,
    now: u32,
}

impl Sieve<'_> {
    fn ranked(&self, library: &Library, docs: &[u32]) -> Vec<u32> {
        let mut ranked: Vec<(i32, u32)> =
            docs.par_iter().filter_map(|&doc| Some((self.rank(library.docs.get(doc)?)?, doc))).collect();
        ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
        ranked.into_iter().map(|(_, doc)| doc).collect()
    }

    fn rank(&self, doc: &Doc) -> Option<i32> {
        let lower = fold::lowered(&doc.path);
        let slash = lower.rfind('\\')? + 1;
        let (folder, lower_name) = lower.split_at(slash);
        let inside = |prefixes: &[String]| prefixes.iter().any(|prefix| lower.starts_with(prefix.as_str()));
        if !self.places.as_deref().is_none_or(inside)
            || !self.folders.iter().all(|folder| contains(folder, &lower))
            || self.ranking.excluded.iter().any(|folder| contains(folder, &lower))
            || !self.query.accepts_details(false, Some((doc.size, i64::from(doc.modified))))
            || !self.query.paths_within(folder)
        {
            return None;
        }
        let original = doc.path.as_bytes().get(doc.path.rfind('\\')? + 1..)?;
        let name = Name::new(original, lower_name, doc.path.is_ascii());
        let (scored, rest) = self.query.score_entry(name, false, Tolerance::Strict)?;
        if !rest.is_empty() && !self.query.in_folder(rest, folder).is_empty() {
            return None;
        }
        let named = if self.query.scores_names() { scored.with_focus(0) } else { 0 };
        let depth = doc.path.matches('\\').count();
        Some(
            named
                + context_score(&doc.path, depth, false, self.boosts, self.ranking)
                + freshness(doc.modified, self.now),
        )
    }
}

fn hit_of(library: &Library, query: &Query, content: &ContentQuery, doc: u32, block: u32) -> Option<Hit> {
    let doc = library.docs.get(doc)?;
    let mut hit = Hit::new(doc.path.clone(), false);
    hit.size = Some(doc.size);
    hit.modified = Some(i64::from(doc.modified));
    hit.highlights = query.highlights(hit.name());
    hit.marks = query.marks(hit.folder());
    hit.snippet = snippet(library, block, content.terms());
    Some(hit)
}

impl Index {
    fn places_of(&self, query: &Query) -> Option<Vec<String>> {
        let place = query.place()?;
        let path = place.path();
        let volumes = self.volumes.read().clone();
        let folders = volumes.iter().flat_map(|volume| {
            let volume = volume.read();
            let found = volume.folders_at(place.anchor(), &path);
            found.into_iter().map(|folder| volume.path(folder)).collect::<Vec<_>>()
        });
        Some(folders.map(|folder| format!("{}\\", fold::lowered(folder.trim_end_matches('\\')))).collect())
    }

    pub(in crate::index) fn search_content(
        &self,
        query: &Query,
        content: &ContentQuery,
        boosts: &Boosts,
        limit: usize,
        facets: &Facets,
        token: &Token,
    ) -> Option<(Results, Counts)> {
        let started = Instant::now();
        let mut counts = Counts::new(facets);
        let empty = |listed| Results { listed, ..Results::default() };
        if !self.content.enabled() {
            return Some((empty(Listed::ContentOff), counts));
        }
        if !content.searchable() {
            return Some((empty(Listed::ContentShort), counts));
        }
        let guard = self.content.library.read();
        let Some(library) = guard.as_ref() else { return Some((empty(Listed::Matches), counts)) };
        let (probe, docs) = match Probe::new(library, content) {
            Ok(found) => found,
            Err(error) => {
                log::warn!("Reading the content index failed: {error:#}");
                return Some((empty(Listed::Matches), counts));
            }
        };
        let ranking = self.ranking.read().clone();
        let sieve = Sieve {
            query,
            boosts,
            places: self.places_of(query),
            folders: query.folders().map(|folder| fold::lowered(folder.trim_end_matches('\\'))).collect(),
            ranking: &ranking,
            now: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as u32),
        };
        let ranked = sieve.ranked(library, &docs);
        let sorter = facets.sorter();
        let sifting = Sifting { limit, budget: COUNT_BUDGET, sorter: sorter.as_ref(), token };
        let verified = sift(&probe, &ranked, &sifting, &mut counts, started)?;
        let found = verified.found.par_iter();
        let hits: Vec<Hit> = found.filter_map(|&(doc, block)| hit_of(library, query, content, doc, block)).collect();
        let listed = if verified.exact { Listed::Matches } else { Listed::AtLeast };
        Some((Results { hits, total: verified.total, listed }, counts))
    }
}
