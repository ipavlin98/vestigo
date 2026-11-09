use super::facets::{Counts, Sorter};
use super::volume::FOCUS_BONUS;
use super::{Boosts, Hit, Ranking, context_score};
use crate::search::{Name, Query, Tolerance};

const RANKED_AS: &str = ".lnk";
const DEPTH: usize = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoreApp {
    pub(crate) name: String,
    pub(crate) path: String,
}

struct Listed {
    app: StoreApp,
    shortcut: String,
    lower: String,
}

#[derive(Default)]
pub(super) struct Apps {
    listed: Vec<Listed>,
}

impl Apps {
    pub(super) fn new(apps: Vec<StoreApp>) -> Self {
        let listed = apps
            .into_iter()
            .map(|app| {
                let shortcut = format!("{}{RANKED_AS}", app.name);
                let lower = shortcut.to_lowercase();
                Listed { app, shortcut, lower }
            })
            .collect();
        Self { listed }
    }

    pub(super) fn lists(&self, apps: &[StoreApp]) -> bool {
        self.listed.len() == apps.len() && self.listed.iter().zip(apps).all(|(listed, app)| listed.app == *app)
    }

    pub(super) fn matches(
        &self,
        query: &Query,
        sorter: Option<&Sorter>,
        counts: &mut Counts,
        boosts: &Boosts,
        ranking: &Ranking,
    ) -> Vec<(i32, Hit)> {
        if query.needs_full_path() || query.needs_details() || query.folders().next().is_some() {
            return Vec::new();
        }
        self.listed
            .iter()
            .filter_map(|listed| {
                let name = Name::new(listed.shortcut.as_bytes(), &listed.lower, listed.lower.is_ascii());
                let scored = query.score_name(name, false, Tolerance::Loose)?;
                if sorter.is_some_and(|sorter| !sorter.admits(&listed.lower, counts)) {
                    return None;
                }
                let StoreApp { name, path } = &listed.app;
                let score = scored.with_focus(FOCUS_BONUS) + context_score(path, DEPTH, false, boosts, ranking);
                Some((score, Hit::app(path.clone(), name.clone())))
            })
            .collect()
    }
}
