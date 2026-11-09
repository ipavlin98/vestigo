use super::shortcut::Shortcut;
use super::tools::{BuiltinCommand, CommandSpec};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_PORT: u16 = 38431;
const MINUTE: u64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Suggestions {
    pub(crate) history: bool,
    pub(crate) windows_recent: bool,
    pub(crate) downloads: bool,
    pub(crate) desktop: bool,
    pub(crate) open_dialogs: bool,
    pub(crate) excluded_types: String,
    pub(crate) hidden: Vec<String>,
}

impl Default for Suggestions {
    fn default() -> Self {
        Self {
            history: true,
            windows_recent: true,
            downloads: true,
            desktop: false,
            open_dialogs: false,
            excluded_types: String::new(),
            hidden: Vec::new(),
        }
    }
}

impl Suggestions {
    pub(crate) fn allows(&self, path: &str) -> bool {
        let lower = path.to_lowercase();
        let hidden = self.hidden.iter().any(|hidden| hidden.eq_ignore_ascii_case(path));
        let excluded = self
            .excluded_types
            .split([';', ',', ' '])
            .map(|extension| extension.trim().trim_start_matches('.').to_lowercase())
            .filter(|extension| !extension.is_empty())
            .any(|extension| lower.rsplit_once('.').is_some_and(|(_, found)| found == extension));
        !hidden && !excluded
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Opening {
    #[default]
    Selected,
    Kept,
    Empty,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Repeat {
    #[default]
    Expand,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Hotkeys {
    pub(crate) search: Option<Shortcut>,
    pub(crate) window: Option<Shortcut>,
    pub(crate) menu: Option<Shortcut>,
    pub(crate) repeat: Repeat,
    pub(crate) fullscreen: bool,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            search: Shortcut::parse("alt-space"),
            window: None,
            menu: None,
            repeat: Repeat::Expand,
            fullscreen: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct IndexOptions {
    pub(crate) skipped_drives: Vec<char>,
    pub(crate) folders: Vec<String>,
    pub(crate) excluded: Vec<String>,
    pub(crate) high: Vec<String>,
    pub(crate) low: Vec<String>,
    pub(crate) hidden_low: bool,
    pub(crate) rewalk: Rewalk,
    pub(crate) full_text: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Rewalk {
    Never,
    QuarterHour,
    #[default]
    HalfHour,
    Hour,
    SixHours,
    Day,
}

impl Rewalk {
    pub(crate) const ALL: [Self; 6] =
        [Self::Never, Self::QuarterHour, Self::HalfHour, Self::Hour, Self::SixHours, Self::Day];

    pub(crate) fn every(self) -> Option<Duration> {
        let minutes = match self {
            Self::Never => return None,
            Self::QuarterHour => 15,
            Self::HalfHour => 30,
            Self::Hour => 60,
            Self::SixHours => 6 * 60,
            Self::Day => 24 * 60,
        };
        Some(Duration::from_secs(minutes * MINUTE))
    }
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            skipped_drives: Vec::new(),
            folders: Vec::new(),
            excluded: Vec::new(),
            high: Vec::new(),
            low: Vec::new(),
            hidden_low: true,
            rewalk: Rewalk::default(),
            full_text: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Favorite {
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Menu {
    pub(crate) favorites: Vec<Favorite>,
    pub(crate) tools: Vec<MenuTool>,
    pub(crate) double_click: bool,
    pub(crate) middle_click: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            favorites: Vec::new(),
            tools: vec![MenuTool::Command(BuiltinCommand::PowerShell), MenuTool::CopyPath, MenuTool::HiddenFiles],
            double_click: true,
            middle_click: false,
        }
    }
}

impl Menu {
    pub(crate) fn rename_command(&mut self, old: &str, new: &str) {
        for tool in &mut self.tools {
            if let MenuTool::Custom(name) = tool
                && name == old
            {
                new.clone_into(name);
            }
        }
    }

    pub(crate) fn forget_command(&mut self, removed: &CommandSpec, left: &[CommandSpec]) {
        let tool = MenuTool::of(removed);
        if tool.command(left).is_none() {
            self.tools.retain(|kept| *kept != tool);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MenuTool {
    CopyPath,
    HiddenFiles,
    Command(BuiltinCommand),
    Custom(String),
}

impl MenuTool {
    pub(crate) fn of(command: &CommandSpec) -> Self {
        match command.builtin {
            Some(builtin) => Self::Command(builtin),
            None => Self::Custom(command.name.clone()),
        }
    }

    pub(crate) fn fits(command: &CommandSpec) -> bool {
        !matches!(command.builtin, Some(BuiltinCommand::NewFolder | BuiltinCommand::NewFile))
    }

    pub(crate) fn command<'a>(&self, commands: &'a [CommandSpec]) -> Option<&'a CommandSpec> {
        match self {
            Self::CopyPath | Self::HiddenFiles => None,
            Self::Command(builtin) => commands.iter().find(|command| command.builtin == Some(*builtin)),
            Self::Custom(name) => commands.iter().find(|command| command.builtin.is_none() && command.name == *name),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Opener {
    pub(crate) path: String,
    pub(crate) arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Api {
    pub(crate) enabled: bool,
    pub(crate) port: u16,
}

impl Default for Api {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(name: &str) -> CommandSpec {
        CommandSpec { name: name.into(), ..CommandSpec::default() }
    }

    #[test]
    fn a_renamed_command_stays_in_the_menu() {
        let mut menu = Menu { tools: vec![MenuTool::Custom("Build".into())], ..Menu::default() };
        menu.rename_command("Build", "Build all");
        assert_eq!(menu.tools, [MenuTool::Custom("Build all".into())]);
        let commands = [custom("Build all")];
        assert!(menu.tools.first().and_then(|tool| tool.command(&commands)).is_some());
    }

    #[test]
    fn a_deleted_command_leaves_the_menu_unless_a_namesake_remains() {
        let tools = vec![MenuTool::Custom("Build".into()), MenuTool::CopyPath];
        let mut menu = Menu { tools: tools.clone(), ..Menu::default() };
        menu.forget_command(&custom("Build"), &[custom("Build")]);
        assert_eq!(menu.tools, tools);
        menu.forget_command(&custom("Build"), &[custom("Deploy")]);
        assert_eq!(menu.tools, [MenuTool::CopyPath]);
    }

    #[test]
    fn menu_tools_read_back_and_an_older_menu_gets_the_old_items() {
        let menu: Menu = serde_json::from_str(r#"{"double_click": false}"#).unwrap();
        assert_eq!(
            menu.tools,
            [MenuTool::Command(BuiltinCommand::PowerShell), MenuTool::CopyPath, MenuTool::HiddenFiles]
        );
        let tools =
            vec![MenuTool::Custom("Build".into()), MenuTool::Command(BuiltinCommand::Cmd), MenuTool::HiddenFiles];
        let saved = serde_json::to_string(&Menu { tools: tools.clone(), ..Menu::default() }).unwrap();
        let menu: Menu = serde_json::from_str(&saved).unwrap();
        assert_eq!(menu.tools, tools);
    }

    #[test]
    fn hidden_files_and_excluded_types_are_not_suggested() {
        let suggestions = Suggestions {
            excluded_types: "tmp; .LOG".into(),
            hidden: vec!["C:\\Work\\plan.txt".into()],
            ..Suggestions::default()
        };
        assert!(suggestions.allows("C:\\Work\\report.pdf"));
        assert!(!suggestions.allows("c:\\work\\PLAN.txt"));
        assert!(!suggestions.allows("C:\\Temp\\setup.log"));
        assert!(!suggestions.allows("C:\\Temp\\a.TMP"));
        assert!(suggestions.allows("C:\\Temp\\catalog"));
    }
}
