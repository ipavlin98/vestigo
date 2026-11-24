use super::{data_dir, load, save};
use gpui::Global;
use serde::{Deserialize, Serialize};

const ACTIVITY_FILE: &str = "activity.json";
const ACTIVITY_LIMIT: usize = 200;
const SESSION_GAP: u64 = 30 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Launch {
    pub(crate) path: String,
    pub(crate) at: u64,
    pub(crate) count: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Activity {
    apps: Vec<Launch>,
}

impl Global for Activity {}

impl Activity {
    pub(crate) fn load() -> Self {
        load(&data_dir().join(ACTIVITY_FILE))
    }

    pub(crate) fn save(&self) {
        if let Err(error) = save(&data_dir().join(ACTIVITY_FILE), self) {
            log::warn!("Couldn't save the programs in use: {error:#}");
        }
    }

    pub(crate) fn record(&mut self, path: &str, at: u64) {
        let path = path.to_lowercase();
        let known = self.apps.iter().position(|app| app.path == path);
        let mut app = known.map(|position| self.apps.remove(position)).unwrap_or(Launch { path, at, count: 0 });
        if app.count == 0 || at.saturating_sub(app.at) >= SESSION_GAP {
            app.count = app.count.saturating_add(1);
        }
        app.at = at;
        self.apps.insert(0, app);
        self.apps.truncate(ACTIVITY_LIMIT);
    }

    pub(crate) fn apps(&self) -> &[Launch] {
        &self.apps
    }

    pub(crate) fn clear(&mut self) {
        self.apps.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_700_000_000;

    #[test]
    fn switching_back_within_a_session_counts_one_use_and_moves_the_program_first() {
        let mut activity = Activity::default();
        activity.record("C:\\Tools\\Code.exe", NOW);
        activity.record("C:\\Tools\\Other.exe", NOW + 10);
        activity.record("c:\\tools\\code.EXE", NOW + 60);
        assert_eq!(
            activity.apps(),
            [
                Launch { path: "c:\\tools\\code.exe".into(), at: NOW + 60, count: 1 },
                Launch { path: "c:\\tools\\other.exe".into(), at: NOW + 10, count: 1 },
            ]
        );
    }

    #[test]
    fn a_return_after_a_break_counts_as_another_use() {
        let mut activity = Activity::default();
        activity.record("C:\\Tools\\Code.exe", NOW);
        activity.record("C:\\Tools\\Code.exe", NOW + SESSION_GAP + 1);
        assert_eq!(activity.apps().first().map(|app| app.count), Some(2));
    }
}
