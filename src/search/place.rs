use super::{Query, Rule, syntax};

const SEPARATORS: [char; 2] = ['\\', '/'];
const HOME: char = '~';

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Listed {
    #[default]
    Matches,
    AtLeast,
    Contents,
    ContentOff,
    ContentShort,
}

pub(crate) struct Place {
    anchor: Option<String>,
    folders: Vec<String>,
    reach: Reach,
}

pub(crate) enum Reach {
    Contents(Option<Tail>),
    Subtree,
}

pub(crate) struct Tail {
    pub(crate) name: String,
    pub(crate) query: Box<Query>,
}

impl Place {
    pub(super) fn parse(word: &str) -> Option<Self> {
        let (anchor, rest) = if let Some(rest) = word.strip_prefix('\\') {
            (None, rest)
        } else if let Some(rest) = home_rest(word) {
            (Some(HOME.to_string()), rest)
        } else {
            let (drive, rest) = word.split_once(":\\")?;
            let mut letters = drive.chars();
            let letter = letters.next().filter(|letter| letter.is_ascii_alphabetic() && letters.next().is_none())?;
            (Some(format!("{letter}:")), rest)
        };
        let folders: Vec<String> = rest.split('\\').filter(|part| !part.is_empty()).map(str::to_owned).collect();
        (anchor.is_some() || !folders.is_empty()).then_some(Self { anchor, folders, reach: Reach::Subtree })
    }

    pub(super) fn alone(&mut self, open: bool, rules: &[Rule]) {
        let tail = if open { None } else { self.folders.pop() };
        self.reach = Reach::Contents(tail.map(|name| Tail { query: Box::new(Query::named(&name, rules)), name }));
    }

    pub(crate) fn anchor(&self) -> Option<&str> {
        self.anchor.as_deref()
    }

    pub(crate) fn reach(&self) -> &Reach {
        &self.reach
    }

    pub(crate) fn parents(&self) -> Vec<&str> {
        self.folders.iter().map(String::as_str).collect()
    }

    pub(crate) fn path(&self) -> Vec<&str> {
        let mut path = self.parents();
        if let Reach::Contents(Some(tail)) = &self.reach {
            path.push(&tail.name);
        }
        path
    }
}

fn home_rest(word: &str) -> Option<&str> {
    word.strip_prefix(HOME).filter(|rest| rest.is_empty() || rest.starts_with('\\'))
}

pub(super) fn at_home(word: &str, home: Option<&str>) -> String {
    match (home_rest(word), home) {
        (Some(rest), Some(home)) => format!("{}{rest}", home.trim_end_matches('\\')),
        (Some(_) | None, _) => word.to_owned(),
    }
}

pub(super) fn is_open(text: &str) -> bool {
    text.ends_with(SEPARATORS)
}

pub(crate) fn enter(text: &str, listed: Listed, path: &str, name: &str) -> String {
    match base(text.trim(), listed, name) {
        Some(base) => format!("{base}\\{name}\\"),
        None => format!("{}\\", path.trim_end_matches('\\')),
    }
}

pub(crate) fn origin(text: &str, listed: Listed, name: &str) -> String {
    let typed = text.trim();
    match (listed, base(typed, listed, name)) {
        (Listed::Matches, Some(base)) if !base.is_empty() => format!("{base}\\"),
        (Listed::Matches | Listed::AtLeast | Listed::Contents | Listed::ContentOff | Listed::ContentShort, _) => {
            typed.to_owned()
        }
    }
}

pub(crate) fn up(text: &str) -> Option<(String, Option<String>)> {
    let typed = text.trim();
    let place = lone(typed)?;
    let inner = typed.trim_end_matches(SEPARATORS);
    let parent = inner.get(..=inner.rfind(SEPARATORS)?)?;
    if parent.trim_start_matches(SEPARATORS).is_empty() {
        return None;
    }
    Some((parent.to_owned(), place.anchor().map(|_| inner.to_owned())))
}

fn lone(typed: &str) -> Option<Place> {
    let mut tokens = syntax::tokens(typed).into_iter();
    match (tokens.next(), tokens.next()) {
        (Some(token), None) if !token.negated && !token.quoted => {
            Place::parse(&token.text.to_lowercase().replace('/', "\\"))
        }
        (Some(_), _) | (None, _) => None,
    }
}

fn base<'a>(typed: &'a str, listed: Listed, name: &str) -> Option<&'a str> {
    lone(typed)?;
    let tail = typed.rsplit(SEPARATORS).next().unwrap_or_default();
    match listed {
        Listed::Contents => Some(typed.trim_end_matches(SEPARATORS)),
        Listed::Matches if !is_open(typed) && tail.to_lowercase() != name.to_lowercase() => {
            typed.rfind(SEPARATORS).and_then(|at| typed.get(..at))
        }
        Listed::Matches | Listed::AtLeast | Listed::ContentOff | Listed::ContentShort => None,
    }
}
