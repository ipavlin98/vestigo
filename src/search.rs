mod content;
mod filter;
pub(crate) mod fold;
mod fuzzy;
mod place;
mod syntax;
#[cfg(test)]
mod tests;

pub(crate) use content::ContentQuery;
pub(crate) use filter::{Builtin, Filter, Kinds, Rule};
pub(crate) use fuzzy::{Name, alphabet};
pub(crate) use place::{Listed, Place, Reach, enter, origin, up};
pub(crate) use syntax::{Clock, Period};

use memchr::memmem::Finder;
use std::ops::{Range, RangeInclusive};

const PREFIX: i32 = 30;
const WORD_START: i32 = 18;
const ABBREVIATION: i32 = 16;
const INSIDE: i32 = 6;
const TYPO: i32 = 4;
const LETTERS: i32 = 8;
const BREAK: i32 = 4;
const EXACT: i32 = 40;
const DIRECTORY: i32 = 4;
const APPLICATION: i32 = 12;
const MAX_EXTRA_CHARS: i32 = 40;
const OTHER_LAYOUT: i32 = 10_000;
const ADJACENT_FOLDERS: i32 = 8;
const APP_TYPES: [&str; 4] = ["exe", "lnk", "appref-ms", "url"];
const EXTENSION: &str = "ext";
const DATE: &str = "date";
const SIZE: &str = "size";
const WILDCARD_EXTENSION: &str = "*.";
const KEYS: [(char, char); 33] = [
    ('q', 'й'),
    ('w', 'ц'),
    ('e', 'у'),
    ('r', 'к'),
    ('t', 'е'),
    ('y', 'н'),
    ('u', 'г'),
    ('i', 'ш'),
    ('o', 'щ'),
    ('p', 'з'),
    ('[', 'х'),
    (']', 'ъ'),
    ('a', 'ф'),
    ('s', 'ы'),
    ('d', 'в'),
    ('f', 'а'),
    ('g', 'п'),
    ('h', 'р'),
    ('j', 'о'),
    ('k', 'л'),
    ('l', 'д'),
    (';', 'ж'),
    ('\'', 'э'),
    ('z', 'я'),
    ('x', 'ч'),
    ('c', 'с'),
    ('v', 'м'),
    ('b', 'и'),
    ('n', 'т'),
    ('m', 'ь'),
    (',', 'б'),
    ('.', 'ю'),
    ('`', 'ё'),
];
const SHIFTED_KEYS: [(char, char); 7] =
    [('{', 'х'), ('}', 'ъ'), (':', 'ж'), ('"', 'э'), ('<', 'б'), ('>', 'ю'), ('~', 'ё')];

pub(crate) fn is_app(lower: &str) -> bool {
    lower.rsplit_once('.').is_some_and(|(_, extension)| APP_TYPES.contains(&extension))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tolerance {
    Strict,
    Loose,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Scored {
    Exact(i32),
    Fuzzy(i32),
}

impl Scored {
    pub(crate) fn with_focus(self, bonus: i32) -> i32 {
        match self {
            Self::Exact(score) => score + bonus,
            Self::Fuzzy(score) => score,
        }
    }
}

enum Fit {
    Substring(usize),
    Abbreviation,
    Typo(Range<usize>),
    Letters(u32),
}

impl Fit {
    fn score(&self, name: Name) -> i32 {
        match self {
            Self::Substring(0) => PREFIX,
            Self::Substring(position) if name.starts_word(*position) => WORD_START,
            Self::Substring(_) => INSIDE,
            Self::Abbreviation => ABBREVIATION,
            Self::Typo(_) => TYPO,
            Self::Letters(breaks) => LETTERS - BREAK * *breaks as i32,
        }
    }
}

struct Spelling {
    text: String,
    chars: Vec<char>,
    ascii: bool,
    letters: u32,
    finder: Finder<'static>,
}

impl Spelling {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            chars: text.chars().collect(),
            ascii: text.is_ascii(),
            letters: fuzzy::alphabet(text.as_bytes()),
            finder: Finder::new(text.as_bytes()).into_owned(),
        }
    }

    fn fit(&self, name: Name, tolerance: Tolerance) -> Option<Fit> {
        if name.ascii && !self.ascii {
            return None;
        }
        let ordered = fuzzy::in_order(self.text.as_bytes(), name.lower.as_bytes());
        ordered.then(|| self.ordered_fit(name)).flatten().or_else(|| match tolerance {
            Tolerance::Strict => None,
            Tolerance::Loose => fuzzy::typo(&self.chars, name.lower).map(Fit::Typo),
        })
    }

    fn ordered_fit(&self, name: Name) -> Option<Fit> {
        if let Some(position) = self.finder.find(name.lower.as_bytes()) {
            return Some(Fit::Substring(position));
        }
        if fuzzy::abbreviation(self.text.as_bytes(), name, &mut |_| {}) {
            return Some(Fit::Abbreviation);
        }
        fuzzy::letters(&self.text, name, &mut |_| {}).map(Fit::Letters)
    }

    fn spans(&self, fit: Fit, name: Name) -> Vec<Range<usize>> {
        let mut spans = Vec::new();
        match fit {
            Fit::Substring(position) => spans.push(position..position + self.text.len()),
            Fit::Abbreviation => {
                fuzzy::abbreviation(self.text.as_bytes(), name, &mut |span| spans.push(span));
            }
            Fit::Typo(range) => spans.push(range),
            Fit::Letters(_) => {
                fuzzy::letters(&self.text, name, &mut |span| spans.push(span));
            }
        }
        spans
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Words(u32);

impl Words {
    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }
}

pub(crate) struct Letters {
    every: Vec<(u32, u32)>,
    some: Vec<(u32, u32)>,
}

impl Letters {
    pub(crate) fn admit(&self, name: u32) -> bool {
        let fits = |&(typed, swapped): &(u32, u32)| typed & !name == 0 || swapped & !name == 0;
        self.every.iter().all(fits) && (self.some.is_empty() || self.some.iter().any(fits))
    }
}

struct Term {
    typed: Spelling,
    swapped: Option<Spelling>,
    anywhere: bool,
}

impl Term {
    fn new(text: &str) -> Self {
        Self {
            typed: Spelling::new(text),
            swapped: other_layout(text).map(|text| Spelling::new(&text)),
            anywhere: false,
        }
    }

    fn anywhere(text: &str) -> Self {
        Self { anywhere: true, ..Self::new(text) }
    }

    fn within(&self, folder: &str) -> bool {
        let found = |spelling: &Spelling| spelling.finder.find(folder.as_bytes()).is_some();
        found(&self.typed) || self.swapped.as_ref().is_some_and(found)
    }

    fn letters(&self) -> (u32, u32) {
        (self.typed.letters, self.swapped.as_ref().map_or(self.typed.letters, |swapped| swapped.letters))
    }

    fn fit(&self, name: Name, tolerance: Tolerance) -> Option<(Fit, &Spelling, i32)> {
        if let Some(fit) = self.typed.fit(name, tolerance) {
            return Some((fit, &self.typed, 0));
        }
        let swapped = self.swapped.as_ref()?;
        swapped.fit(name, tolerance).map(|fit| (fit, swapped, OTHER_LAYOUT))
    }
}

struct PathTerm {
    typed: String,
    swapped: Option<String>,
}

impl PathTerm {
    fn new(text: String) -> Self {
        Self { swapped: other_layout(&text), typed: text }
    }

    fn spellings(&self) -> impl Iterator<Item = &str> {
        [Some(self.typed.as_str()), self.swapped.as_deref()].into_iter().flatten()
    }

    fn within(&self, folders: &str) -> bool {
        self.spellings().any(|text| in_order(text, folders))
    }

    fn adjacent(&self, lower_path: &str) -> bool {
        self.spellings().any(|text| lower_path.contains(text))
    }

    fn names(&self, lower_folder: &str) -> bool {
        self.spellings().any(|text| {
            let parts = text.rsplit_once('\\').map_or("", |(parts, _)| parts);
            parts.split('\\').any(|part| !part.is_empty() && lower_folder.contains(part))
        })
    }
}

fn in_order(text: &str, folders: &str) -> bool {
    let parts = text.rsplit_once('\\').map_or("", |(parts, _)| parts);
    let mut names = folders.split('\\');
    parts.split('\\').filter(|part| !part.is_empty()).all(|part| names.any(|name| name.contains(part)))
}

pub(crate) struct Grammar {
    filters: Vec<(String, Rule)>,
    clock: Clock,
    home: Option<String>,
}

impl Grammar {
    pub(crate) fn new(filters: &[Filter], clock: Clock) -> Self {
        let filters = filters.iter().filter_map(|filter| Some((filter.keyword()?, filter.rule()))).collect();
        Self { filters, clock, home: None }
    }

    pub(crate) fn with_home(self, home: Option<String>) -> Self {
        Self { home: home.map(|home| home.to_lowercase().replace('/', "\\")), ..self }
    }

    fn filter(&self, keyword: &str) -> Option<&Rule> {
        self.filters.iter().find(|(known, _)| known.eq_ignore_ascii_case(keyword)).map(|(_, rule)| rule)
    }
}

#[derive(Default)]
pub(crate) struct Query {
    names: Vec<Term>,
    phrases: Vec<Spelling>,
    excluded: Vec<Finder<'static>>,
    paths: Vec<PathTerm>,
    place: Option<Place>,
    kept: Option<String>,
    rules: Vec<Rule>,
    extensions: Vec<String>,
    excluded_extensions: Vec<String>,
    dates: Vec<RangeInclusive<i64>>,
    sizes: Vec<RangeInclusive<u64>>,
    content: Option<ContentQuery>,
}

impl Query {
    pub(crate) fn parse(input: &str, grammar: &Grammar) -> Option<Self> {
        Self::parse_with(input, grammar, None)
    }

    pub(crate) fn parse_with(input: &str, grammar: &Grammar, rule: Option<Rule>) -> Option<Self> {
        let mut query = Self { rules: rule.into_iter().collect(), ..Self::default() };
        let tokens = syntax::tokens(input);
        let alone = tokens.len() == 1;
        let open = tokens.first().is_some_and(|token| place::is_open(&token.text));
        for token in tokens {
            query.add(token, grammar);
        }
        if let Some(place) = query.place.as_mut().filter(|_| alone) {
            place.alone(open, &query.rules);
        }
        let empty = query.place.is_none()
            && query.names.is_empty()
            && query.phrases.is_empty()
            && query.excluded.is_empty()
            && query.paths.is_empty()
            && query.rules.is_empty()
            && query.extensions.is_empty()
            && query.excluded_extensions.is_empty()
            && query.dates.is_empty()
            && query.sizes.is_empty()
            && query.content.is_none();
        (!empty).then_some(query)
    }

    fn add(&mut self, token: syntax::Token, grammar: &Grammar) {
        let lower = token.text.to_lowercase();
        if token.quoted {
            match (&mut self.content, token.negated, lower.is_empty()) {
                (_, _, true) => {}
                (Some(content), negated, false) => content.add(&token.text, negated),
                (None, true, false) => self.excluded.push(Finder::new(lower.as_bytes()).into_owned()),
                (None, false, false) => self.phrases.push(Spelling::new(&lower)),
            }
            return;
        }
        if let Some(value) = syntax::operator(&token.text, syntax::FULL).filter(|_| !token.negated) {
            self.content.get_or_insert_default().add(value, false);
            return;
        }
        let extensions = syntax::operator(&lower, EXTENSION)
            .or_else(|| lower.strip_prefix(WILDCARD_EXTENSION).filter(|rest| !rest.contains('*')))
            .map(syntax::extensions);
        if let Some(extensions) = extensions {
            let into = if token.negated { &mut self.excluded_extensions } else { &mut self.extensions };
            into.extend(extensions);
            return;
        }
        if let Some(value) = syntax::operator(&lower, DATE) {
            if let Some(range) = syntax::date(value, grammar.clock).filter(|_| !token.negated) {
                self.dates.push(range);
            }
            return;
        }
        if let Some(value) = syntax::operator(&lower, SIZE) {
            if let Some(range) = syntax::size(value).filter(|_| !token.negated) {
                self.sizes.push(range);
            }
            return;
        }
        if let Some((keyword, rest)) = lower.split_once([':', '：'])
            && !rest.starts_with(['\\', '/'])
            && let Some(rule) = grammar.filter(keyword).filter(|_| !token.negated)
        {
            self.rules.push(rule.clone());
            if !rest.is_empty() {
                self.add_word(rest, grammar);
            }
            return;
        }
        if token.negated {
            match &mut self.content {
                Some(content) => content.add(&token.text, true),
                None => self.excluded.push(Finder::new(lower.as_bytes()).into_owned()),
            }
            return;
        }
        self.add_word(&lower, grammar);
    }

    fn named(name: &str, rules: &[Rule]) -> Self {
        Self { names: vec![Term::new(name)], rules: rules.to_vec(), ..Self::default() }
    }

    fn add_word(&mut self, typed: &str, grammar: &Grammar) {
        let word = typed.replace('/', "\\");
        if self.place.is_none()
            && let Some(place) = Place::parse(&place::at_home(&word, grammar.home.as_deref()))
        {
            self.place = Some(place);
            return;
        }
        if let Some(content) = &mut self.content {
            content.add(typed, false);
            return;
        }
        if word.trim_start_matches('\\').is_empty() {
            return;
        }
        if word.contains('\\') {
            if let Some(last) = word.rsplit('\\').next().filter(|last| !last.is_empty()) {
                self.names.push(Term::new(last));
            }
            self.paths.push(PathTerm::new(word));
        } else {
            self.names.push(Term::anywhere(&word));
        }
    }

    pub(crate) fn keep(&mut self, path: &str) {
        self.kept = path.trim_end_matches('\\').rsplit('\\').next().map(str::to_lowercase);
    }

    pub(crate) fn kept(&self) -> Option<&str> {
        self.kept.as_deref()
    }

    pub(crate) fn place(&self) -> Option<&Place> {
        self.place.as_ref()
    }

    pub(crate) fn content(&self) -> Option<&ContentQuery> {
        self.content.as_ref()
    }

    pub(crate) fn scores_names(&self) -> bool {
        !self.names.is_empty() || !self.phrases.is_empty()
    }

    pub(crate) fn needs_full_path(&self) -> bool {
        !self.paths.is_empty()
    }

    pub(crate) fn paths_within(&self, folders: &str) -> bool {
        self.paths.iter().all(|path| path.within(folders))
    }

    pub(crate) fn path_score(&self, lower_path: &str) -> i32 {
        self.paths.iter().filter(|path| path.adjacent(lower_path)).count() as i32 * ADJACENT_FOLDERS
    }

    pub(crate) fn marks(&self, folder: &str) -> Vec<Range<usize>> {
        let place = self.place.as_ref().and_then(|place| place.path().last().map(|name| name.to_lowercase()));
        if place.is_none() && self.paths.is_empty() {
            return Vec::new();
        }
        let mut start = 0;
        folder
            .split('\\')
            .filter_map(|name| {
                let range = start..start + name.len();
                start = range.end + 1;
                let lower = name.to_lowercase();
                let named = place.as_ref() == Some(&lower) || self.paths.iter().any(|path| path.names(&lower));
                (named && !name.is_empty()).then_some(range)
            })
            .collect()
    }

    pub(crate) fn folders(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().filter_map(Rule::folder)
    }

    pub(crate) fn needs_details(&self) -> bool {
        !self.dates.is_empty() || !self.sizes.is_empty()
    }

    pub(crate) fn accepts_details(&self, dir: bool, details: Option<(u64, i64)>) -> bool {
        let Some((size, modified)) = details else { return !self.needs_details() };
        let sized = self.sizes.is_empty() || !dir && self.sizes.iter().all(|range| range.contains(&size));
        sized && self.dates.iter().all(|range| range.contains(&modified))
    }

    pub(crate) fn score_name(&self, name: Name, dir: bool, tolerance: Tolerance) -> Option<Scored> {
        self.score_entry(name, dir, tolerance).filter(|(_, rest)| rest.is_empty()).map(|(scored, _)| scored)
    }

    pub(crate) fn score_entry(&self, name: Name, dir: bool, tolerance: Tolerance) -> Option<(Scored, Words)> {
        let lower = name.lower;
        if !self.rules.iter().all(|rule| rule.accepts(lower, dir))
            || !self.extensions.is_empty() && (dir || !filter::ends_with_any(lower, &self.extensions))
            || !dir && filter::ends_with_any(lower, &self.excluded_extensions)
            || self.excluded.iter().any(|finder| finder.find(lower.as_bytes()).is_some())
        {
            return None;
        }
        let mut score = 0;
        let mut length = 0;
        let mut matched = "";
        let mut fuzzy = false;
        let mut rest = Words(0);
        for (position, term) in self.names.iter().enumerate() {
            let Some((fit, spelling, penalty)) = term.fit(name, tolerance) else {
                rest.0 |= term.anywhere.then(|| 1u32.checked_shl(position as u32)).flatten()?;
                continue;
            };
            fuzzy |= matches!(fit, Fit::Letters(_) | Fit::Typo(_));
            score += fit.score(name) - penalty;
            length += spelling.text.len() as i32;
            matched = &spelling.text;
        }
        for phrase in &self.phrases {
            let position = phrase.finder.find(lower.as_bytes())?;
            score += Fit::Substring(position).score(name);
            length += phrase.text.len() as i32;
            matched = &phrase.text;
        }
        let named = self.names.len() + self.phrases.len() - rest.0.count_ones() as usize;
        if named == 0 && !rest.is_empty() {
            return None;
        }
        if named == 1 {
            let stem = if dir { lower } else { lower.rsplit_once('.').map_or(lower, |(stem, _)| stem) };
            if lower == matched || stem == matched {
                score += EXACT;
            }
        }
        score -= (lower.len() as i32 - length).clamp(0, MAX_EXTRA_CHARS) / 2;
        if dir {
            score += DIRECTORY;
        } else if is_app(lower) {
            score += APPLICATION;
        }
        Some((if fuzzy { Scored::Fuzzy(score) } else { Scored::Exact(score) }, rest))
    }

    pub(crate) fn letters(&self) -> Letters {
        let mut every: Vec<(u32, u32)> = self.names.iter().filter(|term| !term.anywhere).map(Term::letters).collect();
        every.extend(self.phrases.iter().map(|phrase| (phrase.letters, phrase.letters)));
        let some = if every.is_empty() { self.names.iter().map(Term::letters).collect() } else { Vec::new() };
        Letters { every, some }
    }

    pub(crate) fn in_folder(&self, rest: Words, folder: &str) -> Words {
        let found = self.names.iter().enumerate().filter(|(_, term)| term.within(folder));
        Words(found.fold(rest.0, |rest, (position, _)| rest & !1u32.checked_shl(position as u32).unwrap_or(0)))
    }

    pub(crate) fn highlights(&self, name: &str) -> Vec<Range<usize>> {
        let (lower, origins) = fold::with_origins(name);
        let view = Name::new(name.as_bytes(), &lower, name.is_ascii());
        let terms = self
            .names
            .iter()
            .filter_map(|term| term.fit(view, Tolerance::Loose).map(|(fit, spelling, _)| spelling.spans(fit, view)))
            .flatten();
        let phrases = self.phrases.iter().filter_map(|phrase| {
            phrase.finder.find(view.lower.as_bytes()).map(|position| position..position + phrase.text.len())
        });
        merged(terms.chain(phrases).filter_map(|span| fold::origin(&origins, span)).collect())
    }
}

pub(crate) fn merged(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

pub(crate) fn other_layout(text: &str) -> Option<String> {
    let swapped: String = text
        .chars()
        .map(|ch| {
            let to_russian =
                KEYS.iter().chain(&SHIFTED_KEYS).find(|(latin, _)| *latin == ch).map(|&(_, russian)| russian);
            let to_latin = || KEYS.iter().find(|(_, russian)| *russian == ch).map(|&(latin, _)| latin);
            to_russian.or_else(to_latin).unwrap_or(ch)
        })
        .collect();
    (swapped != text).then_some(swapped)
}
