use super::shortcut::Shortcut;
use serde::{Deserialize, Serialize};

const QUERY: &str = "{query}";
const EDITOR: &str = "code";
const EDITOR_SHORTCUT: &str = "alt-e";
const OLD_EDITOR_SHORTCUT: &str = "alt-shift-c";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Scope {
    #[default]
    Vestigo,
    Explorer,
    Anywhere,
}

impl Scope {
    pub(crate) const ALL: [Self; 3] = [Self::Vestigo, Self::Explorer, Self::Anywhere];
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Program {
    pub(crate) path: String,
    pub(crate) arguments: String,
    pub(crate) folder: String,
    pub(crate) hidden: bool,
    pub(crate) admin: bool,
}

impl Program {
    fn new(path: &str, arguments: &str) -> Self {
        Self { path: path.to_owned(), arguments: arguments.to_owned(), ..Self::default() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuiltinAction {
    Open,
    OpenWith,
    Reveal,
    RunAsAdmin,
    Copy,
    Cut,
    CopyPath,
    CopyName,
    Delete,
    Properties,
    ContextMenu,
}

impl BuiltinAction {
    pub(crate) const ALL: [Self; 11] = [
        Self::Open,
        Self::OpenWith,
        Self::Reveal,
        Self::RunAsAdmin,
        Self::Copy,
        Self::Cut,
        Self::CopyPath,
        Self::CopyName,
        Self::Delete,
        Self::Properties,
        Self::ContextMenu,
    ];

    pub(crate) fn takes_many(self) -> bool {
        match self {
            Self::Copy | Self::Cut | Self::CopyPath | Self::CopyName | Self::Delete => true,
            Self::Open | Self::OpenWith | Self::Reveal | Self::RunAsAdmin | Self::Properties | Self::ContextMenu => {
                false
            }
        }
    }

    fn shortcut(self) -> Option<&'static str> {
        match self {
            Self::Open => Some("enter"),
            Self::Reveal => Some("ctrl-enter"),
            Self::RunAsAdmin => Some("ctrl-shift-enter"),
            Self::Copy => Some("ctrl-c"),
            Self::Cut => Some("ctrl-x"),
            Self::CopyPath => Some("ctrl-shift-c"),
            Self::Properties => Some("alt-enter"),
            Self::OpenWith | Self::CopyName | Self::Delete | Self::ContextMenu => None,
        }
    }

    fn action(self) -> ActionSpec {
        ActionSpec {
            builtin: Some(self),
            shortcut: self.shortcut().and_then(Shortcut::parse),
            enabled: self != Self::ContextMenu,
            ..ActionSpec::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ActionSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) builtin: Option<BuiltinAction>,
    pub(crate) name: String,
    pub(crate) keyword: String,
    pub(crate) shortcut: Option<Shortcut>,
    pub(crate) scope: Scope,
    #[serde(flatten)]
    pub(crate) program: Program,
    pub(crate) enabled: bool,
}

impl Default for ActionSpec {
    fn default() -> Self {
        Self {
            builtin: None,
            name: String::new(),
            keyword: String::new(),
            shortcut: None,
            scope: Scope::Vestigo,
            program: Program::default(),
            enabled: true,
        }
    }
}

impl ActionSpec {
    pub(crate) fn defaults() -> Vec<Self> {
        let mut actions: Vec<Self> = BuiltinAction::ALL.into_iter().map(BuiltinAction::action).collect();
        actions.push(Self {
            name: "VS Code".to_owned(),
            keyword: EDITOR.to_owned(),
            shortcut: Shortcut::parse(EDITOR_SHORTCUT),
            program: Program::new(EDITOR, "\"{action_path}\""),
            enabled: false,
            ..Self::default()
        });
        actions
    }

    pub(crate) fn complete(actions: &mut Vec<Self>) {
        let old_editor = Shortcut::parse(OLD_EDITOR_SHORTCUT);
        for action in actions.iter_mut().filter(|action| {
            action.builtin.is_none()
                && action.keyword == EDITOR
                && action.program.path == EDITOR
                && action.shortcut == old_editor
        }) {
            action.shortcut = Shortcut::parse(EDITOR_SHORTCUT);
        }
        for (position, builtin) in BuiltinAction::ALL.into_iter().enumerate() {
            if !actions.iter().any(|action| action.builtin == Some(builtin)) {
                actions.insert(position.min(actions.len()), builtin.action());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuiltinCommand {
    Settings,
    Rebuild,
    Parent,
    NewFolder,
    NewFile,
    Cmd,
    CmdAdmin,
    PowerShell,
    PowerShellAdmin,
    Terminal,
    Shutdown,
    Restart,
    Uninstall,
    Connections,
    Hosts,
}

impl BuiltinCommand {
    pub(crate) const ALL: [Self; 15] = [
        Self::Settings,
        Self::Rebuild,
        Self::Parent,
        Self::NewFolder,
        Self::NewFile,
        Self::Cmd,
        Self::CmdAdmin,
        Self::PowerShell,
        Self::PowerShellAdmin,
        Self::Terminal,
        Self::Shutdown,
        Self::Restart,
        Self::Uninstall,
        Self::Connections,
        Self::Hosts,
    ];

    fn keyword(self) -> &'static str {
        match self {
            Self::Settings => "opt",
            Self::Rebuild => "rebuild",
            Self::Parent => "..",
            Self::NewFolder => "mkdir",
            Self::NewFile => "touch",
            Self::Cmd => "cmd",
            Self::CmdAdmin => "cmda",
            Self::PowerShell => "psh",
            Self::PowerShellAdmin => "psha",
            Self::Terminal => "wt",
            Self::Shutdown => "shutdown",
            Self::Restart => "reboot",
            Self::Uninstall => "uninstall",
            Self::Connections => "connections",
            Self::Hosts => "hosts",
        }
    }

    fn shortcut(self) -> Option<(&'static str, Scope)> {
        match self {
            Self::PowerShell => Some(("ctrl-shift-r", Scope::Explorer)),
            _ => None,
        }
    }

    pub(crate) fn program(self) -> Option<Program> {
        let admin = |program: Program| Program { admin: true, ..program };
        let shell = |path: &str| Program { folder: "{current_folder}".to_owned(), ..Program::new(path, "") };
        Some(match self {
            Self::Settings | Self::Rebuild | Self::Parent | Self::NewFolder | Self::NewFile | Self::Hosts => {
                return None;
            }
            Self::Cmd => shell("cmd.exe"),
            Self::CmdAdmin => admin(shell("cmd.exe")),
            Self::PowerShell => shell("powershell.exe"),
            Self::PowerShellAdmin => admin(shell("powershell.exe")),
            Self::Terminal => Program { arguments: "-d .".to_owned(), ..shell("wt.exe") },
            Self::Shutdown => Program { hidden: true, ..Program::new("shutdown.exe", "/s /t 0") },
            Self::Restart => Program { hidden: true, ..Program::new("shutdown.exe", "/r /t 0") },
            Self::Uninstall => Uninstaller::Classic.program(),
            Self::Connections => Program::new("control.exe", "ncpa.cpl"),
        })
    }

    fn takes_query(self) -> bool {
        match self {
            Self::NewFolder | Self::NewFile => true,
            Self::Settings
            | Self::Rebuild
            | Self::Parent
            | Self::Cmd
            | Self::CmdAdmin
            | Self::PowerShell
            | Self::PowerShellAdmin
            | Self::Terminal
            | Self::Shutdown
            | Self::Restart
            | Self::Uninstall
            | Self::Connections
            | Self::Hosts => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Uninstaller {
    Classic,
    Settings,
}

impl Uninstaller {
    pub(crate) const ALL: [Self; 2] = [Self::Classic, Self::Settings];

    pub(crate) fn program(self) -> Program {
        match self {
            Self::Classic => Program::new("control.exe", "appwiz.cpl"),
            Self::Settings => Program::new("explorer.exe", "ms-settings:appsfeatures"),
        }
    }

    fn of(program: &Program) -> Self {
        if *program == Self::Settings.program() { Self::Settings } else { Self::Classic }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct CommandSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) builtin: Option<BuiltinCommand>,
    pub(crate) name: String,
    pub(crate) keyword: String,
    pub(crate) shortcut: Option<Shortcut>,
    pub(crate) scope: Scope,
    #[serde(flatten)]
    pub(crate) program: Program,
    pub(crate) enabled: bool,
}

impl Default for CommandSpec {
    fn default() -> Self {
        Self {
            builtin: None,
            name: String::new(),
            keyword: String::new(),
            shortcut: None,
            scope: Scope::Vestigo,
            program: Program::default(),
            enabled: true,
        }
    }
}

impl CommandSpec {
    fn builtin(builtin: BuiltinCommand) -> Self {
        let (shortcut, scope) =
            builtin.shortcut().map_or((None, Scope::Vestigo), |(text, scope)| (Shortcut::parse(text), scope));
        Self {
            builtin: Some(builtin),
            keyword: builtin.keyword().to_owned(),
            shortcut,
            scope,
            program: builtin.program().unwrap_or_default(),
            enabled: builtin != BuiltinCommand::Terminal,
            ..Self::default()
        }
    }

    pub(crate) fn uninstaller(&self) -> Option<Uninstaller> {
        (self.builtin == Some(BuiltinCommand::Uninstall)).then(|| Uninstaller::of(&self.program))
    }

    pub(crate) fn takes_query(&self) -> bool {
        match self.builtin {
            Some(builtin) => builtin.takes_query(),
            None => [&self.program.path, &self.program.arguments, &self.program.folder]
                .iter()
                .any(|text| text.contains(QUERY)),
        }
    }

    pub(crate) fn defaults() -> Vec<Self> {
        BuiltinCommand::ALL.into_iter().map(Self::builtin).collect()
    }

    pub(crate) fn complete(commands: &mut Vec<Self>) {
        for (position, builtin) in BuiltinCommand::ALL.into_iter().enumerate() {
            if !commands.iter().any(|command| command.builtin == Some(builtin)) {
                commands.insert(position.min(commands.len()), Self::builtin(builtin));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WebSearch {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) keyword: String,
    pub(crate) enabled: bool,
}

impl Default for WebSearch {
    fn default() -> Self {
        Self { name: String::new(), url: String::new(), keyword: String::new(), enabled: true }
    }
}

impl WebSearch {
    fn new(name: &str, keyword: &str, url: &str) -> Self {
        Self { name: name.to_owned(), url: url.to_owned(), keyword: keyword.to_owned(), enabled: true }
    }

    pub(crate) fn defaults() -> Vec<Self> {
        vec![
            Self::new("Google", "g", "https://www.google.com/search?q={query}"),
            Self::new("Wikipedia", "wiki", "https://en.wikipedia.org/wiki/Special:Search/{query}"),
            Self::new("Википедия", "вики", "https://ru.wikipedia.org/wiki/Special:Search/{query}"),
            Self::new("Яндекс", "ya", "https://yandex.ru/search/?text={query}"),
            Self::new("DuckDuckGo", "ddg", "https://duckduckgo.com/?q={query}"),
            Self::new("Bing", "bing", "https://www.bing.com/search?q={query}"),
            Self::new("Stack Overflow", "so", "https://stackoverflow.com/search?q={query}"),
            Self::new("GitHub", "gh", "https://github.com/search?q={query}"),
            Self::new("YouTube", "youtube", "https://www.youtube.com/results?search_query={query}"),
            Self::new("Google Maps", "maps", "https://www.google.com/maps/search/{query}"),
            Self::new("Gmail", "gmail", "https://mail.google.com/mail/#search/{query}"),
            Self::new("X", "x", "https://x.com/search?q={query}"),
            Self::new("Amazon", "amazon", "https://www.amazon.com/s?k={query}"),
            Self::new("eBay", "ebay", "https://www.ebay.com/sch/i.html?_nkw={query}"),
        ]
    }

    pub(crate) fn usable(&self) -> bool {
        self.enabled && !self.url.trim().is_empty()
    }

    pub(crate) fn link(&self, query: &str) -> String {
        self.url.replace(QUERY, &encode(query))
    }
}

fn encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => encoded.push(char::from(byte)),
            b' ' => encoded.push_str("%20"),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_queries_are_percent_encoded() {
        let google = WebSearch::new("Google", "g", "https://www.google.com/search?q={query}");
        assert_eq!(google.link("rust & gpui"), "https://www.google.com/search?q=rust%20%26%20gpui");
        assert_eq!(google.link("отчёт"), "https://www.google.com/search?q=%D0%BE%D1%82%D1%87%D1%91%D1%82");
    }

    #[test]
    fn missing_builtins_are_added_back_in_place() {
        let mut actions = vec![ActionSpec { name: "Mine".into(), ..ActionSpec::default() }];
        ActionSpec::complete(&mut actions);
        assert_eq!(actions.len(), BuiltinAction::ALL.len() + 1);
        assert_eq!(actions.first().and_then(|action| action.builtin), Some(BuiltinAction::Open));
        assert_eq!(actions.last().map(|action| action.name.as_str()), Some("Mine"));
        let mut commands = Vec::new();
        CommandSpec::complete(&mut commands);
        assert_eq!(commands, CommandSpec::defaults());
    }

    #[test]
    fn the_shell_menu_action_and_the_editor_start_switched_off() {
        let actions = ActionSpec::defaults();
        let off: Vec<_> = actions.iter().filter(|action| !action.enabled).collect();
        assert_eq!(off.len(), 2);
        assert!(off.iter().any(|action| action.builtin == Some(BuiltinAction::ContextMenu)));
        assert!(off.iter().any(|action| action.name == "VS Code"));
        let mut restored = Vec::new();
        ActionSpec::complete(&mut restored);
        assert!(restored.iter().any(|action| action.builtin == Some(BuiltinAction::ContextMenu) && !action.enabled));
    }

    #[test]
    fn the_editor_moves_from_its_old_default_key_to_alt_e_unless_changed() {
        let text = r#"[{ "name": "VS Code", "keyword": "code", "shortcut": "alt-shift-c", "path": "code" },
            { "name": "Mine", "keyword": "code", "shortcut": "alt-shift-c", "path": "subl" }]"#;
        let mut actions: Vec<ActionSpec> = serde_json::from_str(text).expect("json");
        ActionSpec::complete(&mut actions);
        let keys: Vec<String> = actions
            .iter()
            .filter(|action| action.builtin.is_none())
            .filter_map(|action| action.shortcut.as_ref().map(ToString::to_string))
            .collect();
        assert_eq!(keys, ["Alt+E", "Alt+Shift+C"]);
    }

    #[test]
    fn the_uninstall_command_keeps_the_chosen_window() {
        let mut command = CommandSpec::builtin(BuiltinCommand::Uninstall);
        assert_eq!(command.uninstaller(), Some(Uninstaller::Classic));
        command.program = Uninstaller::Settings.program();
        let saved = serde_json::to_string(&command).expect("json");
        let read: CommandSpec = serde_json::from_str(&saved).expect("json");
        assert_eq!(read.uninstaller(), Some(Uninstaller::Settings));
        assert_eq!(CommandSpec::builtin(BuiltinCommand::Cmd).uninstaller(), None);
    }

    #[test]
    fn a_saved_custom_command_reads_back() {
        let text = r#"{ "name": "Code here", "keyword": "vs", "path": "code", "arguments": ".", "folder": "{current_folder}",
            "shortcut": "ctrl-alt-v", "scope": "explorer" }"#;
        let command: CommandSpec = serde_json::from_str(text).expect("json");
        assert_eq!(command.program.folder, "{current_folder}");
        assert_eq!(command.shortcut.map(|shortcut| shortcut.to_string()).as_deref(), Some("Ctrl+Alt+V"));
        assert_eq!(command.scope, Scope::Explorer);
        assert!(command.enabled);
    }
}
