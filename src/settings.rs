mod lists;
mod options;
mod shortcut;
mod tools;
mod usage;

pub(crate) use lists::{List, after_move, after_removal};
pub(crate) use options::{
    Api, Favorite, Hotkeys, IndexOptions, Menu, MenuTool, Opener, Opening, Repeat, Rewalk, Suggestions,
};
pub(crate) use shortcut::Shortcut;
pub(crate) use tools::{
    ActionSpec, BuiltinAction, BuiltinCommand, CommandSpec, Program, Scope, Uninstaller, WebSearch,
};
pub(crate) use usage::Usage;

use crate::i18n::LanguageChoice;
use crate::index::Boosts;
use crate::search::Filter;
use crate::theme::{ThemeChoice, ThemeEdits};
use anyhow::{Context, Result, bail};
use gpui::Global;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const APP_DIR: &str = "Vestigo";
const LEGACY_DIR: &str = "sfs";
const DATA_ROOT: &str = "APPDATA";
const CACHE_ROOT: &str = "LOCALAPPDATA";
const SETTINGS_FILE: &str = "settings.json";
const HISTORY_FILE: &str = "history.json";
const USAGE_FILE: &str = "usage.json";
const THEMES_DIR: &str = "themes";
const THEME_EXTENSION: &str = "json";
const THEME_COPIES: usize = 100;
const HISTORY_LIMIT: usize = 200;
const HISTORY_BOOST: i32 = 60;

pub(crate) fn data_dir() -> PathBuf {
    root(DATA_ROOT).join(APP_DIR)
}

pub(crate) fn cache_dir() -> PathBuf {
    root(CACHE_ROOT).join(APP_DIR)
}

pub(crate) fn themes_dir() -> PathBuf {
    data_dir().join(THEMES_DIR)
}

pub(crate) fn retire_themes(files: &[(&str, &str)]) {
    retire(&themes_dir(), files);
}

pub(crate) fn theme_path(id: &str) -> PathBuf {
    themes_dir().join(format!("{id}.{THEME_EXTENSION}"))
}

pub(crate) fn theme_files() -> Vec<(String, String)> {
    read_themes(&themes_dir())
}

pub(crate) fn create_theme(stem: &str, taken: impl Fn(&str) -> bool, text: &str) -> Result<String> {
    create_in(&themes_dir(), stem, taken, text)
}

fn create_in(dir: &Path, stem: &str, taken: impl Fn(&str) -> bool, text: &str) -> Result<String> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let temp = dir.join(format!("{stem}.tmp"));
    std::fs::write(&temp, text).with_context(|| format!("writing {}", temp.display()))?;
    let candidates = (1..=THEME_COPIES).map(|copy| if copy == 1 { stem.to_owned() } else { format!("{stem} {copy}") });
    let mut created = None;
    for id in candidates.filter(|id| !taken(id)) {
        match std::fs::hard_link(&temp, dir.join(format!("{id}.{THEME_EXTENSION}"))) {
            Ok(()) => {
                created = Some(id);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(error).with_context(|| format!("saving the theme {id}"));
            }
        }
    }
    let _ = std::fs::remove_file(&temp);
    match created {
        Some(id) => Ok(id),
        None => bail!("every name like {stem} is taken"),
    }
}

fn retire(dir: &Path, files: &[(&str, &str)]) {
    for (name, text) in files {
        let path = dir.join(name);
        let untouched = std::fs::read_to_string(&path).is_ok_and(|held| held.lines().eq(text.lines()));
        if untouched && let Err(error) = std::fs::remove_file(&path) {
            log::warn!("Couldn't remove the old copy {}: {error}", path.display());
        }
    }
}

fn read_themes(dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut themes: Vec<(String, String)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case(THEME_EXTENSION)))
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?.to_owned();
            match std::fs::read_to_string(&path) {
                Ok(text) => Some((id, text)),
                Err(error) => {
                    log::warn!("Couldn't read the theme {}: {error}", path.display());
                    None
                }
            }
        })
        .collect();
    themes.sort_by_key(|(id, _)| id.to_lowercase());
    themes
}

pub(crate) fn adopt_legacy_data() -> Result<bool> {
    adopt(&root(DATA_ROOT).join(LEGACY_DIR), &data_dir(), &[SETTINGS_FILE, HISTORY_FILE])
}

pub(crate) fn adopt_legacy_cache() -> Result<bool> {
    adopt(&root(CACHE_ROOT).join(LEGACY_DIR), &cache_dir(), &[])
}

fn root(variable: &str) -> PathBuf {
    std::env::var_os(variable).map_or_else(std::env::temp_dir, PathBuf::from)
}

fn adopt(legacy: &Path, current: &Path, keep: &[&str]) -> Result<bool> {
    if current.exists() || !legacy.is_dir() {
        return Ok(false);
    }
    if std::fs::rename(legacy, current).is_ok() {
        return Ok(true);
    }
    std::fs::create_dir_all(current).with_context(|| format!("creating {}", current.display()))?;
    for name in keep {
        let file = legacy.join(name);
        if file.exists() {
            std::fs::copy(&file, current.join(name)).with_context(|| format!("copying {}", file.display()))?;
        }
    }
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) theme: ThemeChoice,
    pub(crate) theme_edits: ThemeEdits,
    pub(crate) zoom: Zoom,
    pub(crate) language: LanguageChoice,
    pub(crate) recent: bool,
    pub(crate) opening: Opening,
    pub(crate) idle_status: bool,
    pub(crate) path_crumbs: bool,
    pub(crate) hide_shortcut_extensions: bool,
    pub(crate) replace_small_icons: bool,
    pub(crate) position: Option<Position>,
    pub(crate) triggers: Triggers,
    pub(crate) filters: Vec<Filter>,
    pub(crate) tray: bool,
    pub(crate) suggestions: Suggestions,
    pub(crate) hotkeys: Hotkeys,
    pub(crate) actions: Vec<ActionSpec>,
    pub(crate) commands: Vec<CommandSpec>,
    pub(crate) web: Vec<WebSearch>,
    pub(crate) web_suggestions: bool,
    pub(crate) index: IndexOptions,
    pub(crate) menu: Menu,
    pub(crate) opener: Opener,
    pub(crate) api: Api,
    pub(crate) disabled_apps: Vec<String>,
    pub(crate) preview: Preview,
    pub(crate) marks: BTreeMap<char, String>,
    pub(crate) check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            theme_edits: ThemeEdits::default(),
            zoom: Zoom::default(),
            language: LanguageChoice::default(),
            recent: true,
            opening: Opening::default(),
            idle_status: true,
            path_crumbs: true,
            hide_shortcut_extensions: true,
            replace_small_icons: true,
            position: None,
            triggers: Triggers::default(),
            filters: Filter::defaults(),
            tray: true,
            suggestions: Suggestions::default(),
            hotkeys: Hotkeys::default(),
            actions: ActionSpec::defaults(),
            commands: CommandSpec::defaults(),
            web: WebSearch::defaults(),
            web_suggestions: true,
            index: IndexOptions::default(),
            menu: Menu::default(),
            opener: Opener::default(),
            api: Api::default(),
            disabled_apps: Vec::new(),
            preview: Preview::default(),
            marks: BTreeMap::new(),
            check_updates: true,
        }
    }
}

impl Global for Settings {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Changes {
    pub(crate) theme: bool,
    pub(crate) zoom: bool,
    pub(crate) language: bool,
    pub(crate) indexed: bool,
    pub(crate) rewalk: bool,
    pub(crate) full_text: bool,
    pub(crate) ranking: bool,
    pub(crate) filters: bool,
    pub(crate) api: bool,
    pub(crate) actions: bool,
    pub(crate) recent: bool,
    pub(crate) suggestions: bool,
    pub(crate) updates: bool,
}

impl Settings {
    pub(crate) fn changes(&self, before: &Self) -> Changes {
        let (index, was) = (&self.index, &before.index);
        Changes {
            theme: (&self.theme, &self.theme_edits) != (&before.theme, &before.theme_edits),
            zoom: self.zoom != before.zoom,
            language: self.language != before.language,
            indexed: (&index.skipped_drives, &index.folders) != (&was.skipped_drives, &was.folders),
            rewalk: index.rewalk != was.rewalk,
            full_text: index.full_text != was.full_text,
            ranking: (&index.excluded, &index.high, &index.low, index.hidden_low)
                != (&was.excluded, &was.high, &was.low, was.hidden_low),
            filters: self.filters != before.filters,
            api: self.api != before.api,
            actions: self.actions != before.actions,
            recent: self.recent != before.recent,
            suggestions: self.suggestions != before.suggestions,
            updates: self.check_updates != before.check_updates,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preview {
    pub(crate) window: bool,
    pub(crate) launcher: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "u16", into = "u16")]
pub(crate) enum Zoom {
    Smaller,
    #[default]
    Normal,
    Larger,
    Largest,
}

impl Zoom {
    pub(crate) const ALL: [Self; 4] = [Self::Smaller, Self::Normal, Self::Larger, Self::Largest];

    pub(crate) fn percent(self) -> u16 {
        match self {
            Self::Smaller => 75,
            Self::Normal => 100,
            Self::Larger => 125,
            Self::Largest => 150,
        }
    }

    pub(crate) fn factor(self) -> f32 {
        f32::from(self.percent()) / 100.0
    }
}

impl From<u16> for Zoom {
    fn from(percent: u16) -> Self {
        Self::ALL.into_iter().min_by_key(|zoom| zoom.percent().abs_diff(percent)).unwrap_or_default()
    }
}

impl From<Zoom> for u16 {
    fn from(zoom: Zoom) -> Self {
        zoom.percent()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Position {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Triggers {
    pub(crate) double_ctrl: bool,
    pub(crate) explorer: bool,
    pub(crate) desktop: bool,
    pub(crate) dialogs: bool,
}

impl Default for Triggers {
    fn default() -> Self {
        Self { double_ctrl: true, explorer: true, desktop: true, dialogs: true }
    }
}

impl Settings {
    pub(crate) fn load() -> Self {
        load_with(&data_dir().join(SETTINGS_FILE), Self::parse)
    }

    fn parse(text: &str) -> serde_json::Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let alt_space_off = value.pointer("/triggers/alt_space").and_then(serde_json::Value::as_bool) == Some(false);
        let new_hotkeys = value.get("hotkeys").is_none();
        let new_desktop = value.pointer("/triggers/desktop").is_none();
        let mut settings: Self = serde_json::from_value(value)?;
        if alt_space_off && new_hotkeys {
            settings.hotkeys.search = None;
        }
        if new_desktop {
            settings.triggers.desktop = settings.triggers.explorer;
        }
        Filter::complete(&mut settings.filters);
        ActionSpec::complete(&mut settings.actions);
        CommandSpec::complete(&mut settings.commands);
        Ok(settings)
    }

    pub(crate) fn save(&self) {
        if let Err(error) = save(&data_dir().join(SETTINGS_FILE), self) {
            log::warn!("Couldn't save settings: {error:#}");
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Visit {
    pub(crate) path: String,
    pub(crate) dir: bool,
    #[serde(default)]
    pub(crate) at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct History {
    visits: Vec<Visit>,
    #[serde(skip)]
    usage: Usage,
}

impl Global for History {}

impl History {
    pub(crate) fn load() -> Self {
        let mut history: Self = load(&data_dir().join(HISTORY_FILE));
        history.usage = load(&data_dir().join(USAGE_FILE));
        history
    }

    pub(crate) fn record(&mut self, path: &str, dir: bool) {
        self.visits.retain(|visit| !visit.path.eq_ignore_ascii_case(path));
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        self.visits.insert(0, Visit { path: path.to_owned(), dir, at });
        self.visits.truncate(HISTORY_LIMIT);
        self.usage.record(i64::try_from(at).unwrap_or_default());
    }

    pub(crate) fn usage(&self) -> &Usage {
        &self.usage
    }

    pub(crate) fn forget(&mut self, path: &str) {
        self.visits.retain(|visit| !visit.path.eq_ignore_ascii_case(path));
    }

    pub(crate) fn clear(&mut self) {
        self.visits.clear();
        self.usage.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    pub(crate) fn save(&self) {
        if let Err(error) = save(&data_dir().join(HISTORY_FILE), self) {
            log::warn!("Couldn't save history: {error:#}");
        }
        if let Err(error) = save(&data_dir().join(USAGE_FILE), &self.usage) {
            log::warn!("Couldn't save usage: {error:#}");
        }
    }

    pub(crate) fn visits(&self) -> Vec<Visit> {
        self.visits.clone()
    }

    pub(crate) fn boosts(&self) -> Boosts {
        self.visits
            .iter()
            .enumerate()
            .map(|(rank, visit)| {
                (visit.path.to_lowercase(), HISTORY_BOOST - (rank as i32 * HISTORY_BOOST / HISTORY_LIMIT as i32))
            })
            .collect()
    }
}

pub(crate) fn recent_folders(mut visits: Vec<Visit>) -> Vec<String> {
    visits.sort_by_key(|visit| Reverse(visit.at));
    let mut folders: Vec<String> = Vec::new();
    for visit in visits {
        let folder = if visit.dir {
            visit.path
        } else {
            let Some(parent) = Path::new(&visit.path).parent().and_then(Path::to_str) else { continue };
            parent.to_owned()
        };
        if !folders.iter().any(|known| known.eq_ignore_ascii_case(&folder)) {
            folders.push(folder);
        }
    }
    folders
}

fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    load_with(path, |text| serde_json::from_str(text))
}

fn load_with<T: Default>(path: &Path, parse: impl Fn(&str) -> serde_json::Result<T>) -> T {
    let Ok(text) = std::fs::read_to_string(path) else {
        return T::default();
    };
    parse(&text).unwrap_or_else(|error| {
        log::warn!("{} is damaged ({error}), keeping a copy and starting over", path.display());
        let _ = std::fs::copy(path, path.with_extension("broken.json"));
        T::default()
    })
}

fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let dir = path.parent().context("file has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(value)?).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_reports_only_what_it_touches() {
        let before = Settings::default();
        assert_eq!(before.changes(&before), Changes::default());
        let changed = |edit: fn(&mut Settings)| {
            let mut after = before.clone();
            edit(&mut after);
            after.changes(&before)
        };
        assert_eq!(
            changed(|s| s.index.full_text = !s.index.full_text),
            Changes { full_text: true, ..Changes::default() }
        );
        assert_eq!(
            changed(|s| s.index.hidden_low = !s.index.hidden_low),
            Changes { ranking: true, ..Changes::default() }
        );
        assert_eq!(changed(|s| s.index.skipped_drives.push('D')), Changes { indexed: true, ..Changes::default() });
        assert_eq!(
            changed(|s| s.index.folders.push("D:\\Work".into())),
            Changes { indexed: true, ..Changes::default() }
        );
        assert_eq!(changed(|s| s.recent = !s.recent), Changes { recent: true, ..Changes::default() });
        assert_eq!(changed(|s| s.actions.clear()), Changes { actions: true, ..Changes::default() });
        assert_eq!(changed(|s| s.filters.clear()), Changes { filters: true, ..Changes::default() });
        assert_eq!(changed(|s| s.tray = !s.tray), Changes::default());
    }

    #[test]
    fn the_opening_choice_reads_back() {
        let settings = Settings::parse(r#"{ "opening": "empty" }"#).expect("json");
        assert_eq!(settings.opening, Opening::Empty);
    }

    #[test]
    fn settings_from_an_older_version_keep_new_features_on() {
        let settings = Settings::parse(r#"{ "theme": "dark", "language": "russian" }"#).expect("json");
        assert_eq!(settings.theme, ThemeChoice::Named("dark".to_owned()));
        assert_eq!(settings.zoom, Zoom::Normal);
        assert_eq!(settings.theme_edits, ThemeEdits::default());
        assert_eq!(settings.language, LanguageChoice::Russian);
        assert!(settings.recent);
        assert_eq!(settings.opening, Opening::Selected);
        assert!(settings.hide_shortcut_extensions);
        assert_eq!(settings.position, None);
        assert_eq!(settings.triggers, Triggers { double_ctrl: true, explorer: true, desktop: true, dialogs: true });
        assert_eq!(settings.hotkeys.search, Shortcut::parse("alt-space"));
        assert_eq!(settings.filters, Filter::defaults());
        assert_eq!(settings.actions, ActionSpec::defaults());
        assert_eq!(settings.commands, CommandSpec::defaults());
        assert_eq!(settings.web, WebSearch::defaults());
        assert!(settings.tray && settings.web_suggestions && settings.suggestions.history);
        assert!(settings.check_updates);
    }

    #[test]
    fn settings_with_the_removed_follow_explorer_switch_still_load() {
        let settings = Settings::parse(r#"{ "follow_explorer": false, "theme": "dark" }"#).expect("json");
        assert_eq!(settings.theme, ThemeChoice::Named("dark".to_owned()));
    }

    #[test]
    fn switched_off_triggers_of_an_older_version_stay_off() {
        let settings = Settings::parse(r#"{ "triggers": { "alt_space": false, "explorer": false } }"#).expect("json");
        assert_eq!(settings.triggers, Triggers { double_ctrl: true, explorer: false, desktop: false, dialogs: true });
        assert_eq!(settings.hotkeys.search, None);
    }

    #[test]
    fn saved_hotkeys_win_over_the_old_alt_space_switch() {
        let text = r#"{ "triggers": { "alt_space": false, "desktop": true }, "hotkeys": { "search": "ctrl-k" } }"#;
        let settings = Settings::parse(text).expect("json");
        assert_eq!(settings.hotkeys.search, Shortcut::parse("ctrl-k"));
        assert!(settings.triggers.desktop);
        let saved = serde_json::to_string(&settings).expect("json");
        assert_eq!(Settings::parse(&saved).expect("round trip"), settings);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vestigo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_new_theme_never_replaces_a_file_or_a_reserved_name() {
        let dir = scratch("create-theme");
        std::fs::create_dir_all(&dir).expect("themes folder");
        std::fs::write(dir.join("Night.json"), "old").expect("existing theme");
        let first = create_in(&dir, "Night", |_| false, "new").expect("saved");
        let second = create_in(&dir, "dark", |id| id.eq_ignore_ascii_case("dark"), "copy").expect("saved");
        let kept = std::fs::read_to_string(dir.join("Night.json")).expect("existing theme");
        let written = std::fs::read_to_string(dir.join("Night 2.json")).expect("new theme");
        let leftovers = std::fs::read_dir(&dir)
            .expect("folder")
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "tmp"));
        let leftovers = leftovers.count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((first.as_str(), second.as_str()), ("Night 2", "dark 2"));
        assert_eq!((kept.as_str(), written.as_str()), ("old", "new"));
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn settings_move_over_from_the_old_folder() {
        let dir = scratch("adopt");
        let (legacy, current) = (dir.join("sfs"), dir.join("Vestigo"));
        std::fs::create_dir_all(&legacy).expect("legacy folder");
        std::fs::write(legacy.join(SETTINGS_FILE), r#"{ "theme": "dark" }"#).expect("old settings");
        let adopted = adopt(&legacy, &current, &[SETTINGS_FILE]).expect("adopted");
        let settings: Settings =
            serde_json::from_str(&std::fs::read_to_string(current.join(SETTINGS_FILE)).expect("new settings"))
                .expect("json");
        let again = adopt(&legacy, &current, &[SETTINGS_FILE]).expect("second run");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(adopted);
        assert_eq!(settings.theme, ThemeChoice::Named("dark".to_owned()));
        assert!(!again);
    }

    #[test]
    fn an_existing_folder_is_never_overwritten() {
        let dir = scratch("keep");
        let (legacy, current) = (dir.join("sfs"), dir.join("Vestigo"));
        for (folder, theme) in [(&legacy, "dark"), (&current, "light")] {
            std::fs::create_dir_all(folder).expect("folder");
            std::fs::write(folder.join(SETTINGS_FILE), format!(r#"{{ "theme": "{theme}" }}"#)).expect("settings");
        }
        let adopted = adopt(&legacy, &current, &[SETTINGS_FILE]).expect("checked");
        let kept = std::fs::read_to_string(current.join(SETTINGS_FILE)).expect("settings");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!adopted);
        assert!(kept.contains("light"));
    }

    #[test]
    fn an_old_settings_file_leaves_full_text_search_off() {
        let settings: Settings = serde_json::from_str(r#"{ "index": { "hidden_low": false } }"#).expect("json");
        assert!(!settings.index.hidden_low);
        assert!(!settings.index.full_text);
    }

    #[test]
    fn a_hand_edited_zoom_snaps_to_the_nearest_step() {
        let settings: Settings = serde_json::from_str(r#"{ "zoom": 130 }"#).expect("json");
        assert_eq!(settings.zoom, Zoom::Larger);
        assert_eq!(serde_json::to_value(Zoom::Smaller).expect("json"), 75);
    }

    #[test]
    fn untouched_copies_of_old_examples_go_and_everything_else_stays() {
        let dir = scratch("themes").join("themes");
        std::fs::create_dir_all(&dir).expect("folder");
        std::fs::write(dir.join("example.json"), "{\r\n}\r\n").expect("old copy, written with other line ends");
        std::fs::write(dir.join("edited.json"), "{ \"name\": \"Mine\" }").expect("edited copy");
        std::fs::write(dir.join("Zeta.JSON"), "{ }").expect("own theme");
        std::fs::write(dir.join("notes.txt"), "").expect("other file");
        let shipped = "{\n}\n";
        retire(&dir, &[("example.json", shipped), ("edited.json", shipped), ("absent.json", shipped)]);
        let themes = read_themes(&dir);
        let _ = std::fs::remove_dir_all(dir.parent().expect("scratch folder"));
        assert_eq!(
            themes,
            [("edited".to_owned(), "{ \"name\": \"Mine\" }".to_owned()), ("Zeta".to_owned(), "{ }".to_owned())]
        );
    }

    #[test]
    fn recent_folders_include_folders_of_files_newest_first() {
        let history: History = serde_json::from_str(
            r#"[{ "path": "C:\\Work", "dir": true }, { "path": "C:\\Work\\a.txt", "dir": false }]"#,
        )
        .expect("json from a version without times");
        let mut visits = history.visits();
        visits.push(Visit { path: "D:\\Фото\\море.jpg".into(), dir: false, at: 20 });
        visits.push(Visit { path: "C:\\report.pdf".into(), dir: false, at: 5 });
        assert_eq!(recent_folders(visits), ["D:\\Фото", "C:\\", "C:\\Work"]);
    }
}
