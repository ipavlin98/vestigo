use super::fold;

const MIN_TERM_CHARS: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ContentQuery {
    terms: Vec<String>,
    excluded: Vec<String>,
}

impl ContentQuery {
    pub(super) fn add(&mut self, text: &str, negated: bool) {
        let mut plain = String::new();
        fold::collapse(text, &mut plain);
        if !plain.is_empty() {
            let into = if negated { &mut self.excluded } else { &mut self.terms };
            into.push(fold::folded(&plain));
        }
    }

    pub(crate) fn terms(&self) -> &[String] {
        &self.terms
    }

    pub(crate) fn excluded(&self) -> &[String] {
        &self.excluded
    }

    pub(crate) fn has_grams(term: &str) -> bool {
        term.chars().nth(MIN_TERM_CHARS - 1).is_some()
    }

    pub(crate) fn searchable(&self) -> bool {
        self.terms.iter().any(|term| Self::has_grams(term))
    }
}
