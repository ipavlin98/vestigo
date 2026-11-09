use super::{Command, Extra, Tool, perform};
use crate::i18n::{Language, Strings};
use crate::index::Hit;
use crate::platform::{self, Event, Host, HostKind};
use crate::search;
use crate::settings::{ActionSpec, BuiltinAction, BuiltinCommand, CommandSpec, Program, Scope, Settings, WebSearch};
use gpui::{App, ClipboardItem, SharedString};
use std::path::Path;

const ACTION_PATH: &str = "{action_path}";
const ACTION_PARENT: &str = "{action_path:parent}";
const QUERY: &str = "{query}";
const CURRENT_FOLDER: &str = "{current_folder}";
const HOSTS: &str = "System32\\drivers\\etc\\hosts";
const SYSTEM_ROOT: &str = "SystemRoot";
const DEFAULT_SYSTEM_ROOT: &str = "C:\\Windows";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Subject {
    pub(crate) paths: Vec<String>,
    pub(crate) folder: Option<String>,
    pub(crate) query: String,
}

impl Subject {
    pub(crate) fn of(paths: Vec<String>) -> Self {
        Self { paths, ..Self::default() }
    }

    fn first(&self) -> &str {
        self.paths.first().map_or("", String::as_str)
    }

    fn parent(&self) -> &str {
        Path::new(self.first()).parent().and_then(Path::to_str).unwrap_or("")
    }

    fn current_folder(&self) -> &str {
        self.folder.as_deref().unwrap_or_else(|| self.parent())
    }

    fn expand(&self, template: &str) -> String {
        template
            .replace(ACTION_PARENT, self.parent())
            .replace(ACTION_PATH, self.first())
            .replace(QUERY, &self.query)
            .replace(CURRENT_FOLDER, self.current_folder())
    }

    pub(crate) fn program(&self, program: &Program) -> Program {
        let folder = if program.folder.trim().is_empty() { CURRENT_FOLDER } else { program.folder.as_str() };
        Program {
            path: self.expand(&program.path),
            arguments: self.expand(&program.arguments),
            folder: self.expand(folder),
            hidden: program.hidden,
            admin: program.admin,
        }
    }
}

pub(crate) fn action_label(action: &ActionSpec, strings: &Strings) -> SharedString {
    let builtin = match action.builtin {
        Some(BuiltinAction::Open) => strings.open,
        Some(BuiltinAction::OpenWith) => strings.action_open_with,
        Some(BuiltinAction::Reveal) => strings.reveal,
        Some(BuiltinAction::RunAsAdmin) => strings.action_admin,
        Some(BuiltinAction::Copy) => strings.action_copy,
        Some(BuiltinAction::Cut) => strings.action_cut,
        Some(BuiltinAction::CopyPath) => strings.action_copy_path,
        Some(BuiltinAction::CopyName) => strings.action_copy_name,
        Some(BuiltinAction::Delete) => strings.action_delete,
        Some(BuiltinAction::Properties) => strings.action_properties,
        Some(BuiltinAction::ContextMenu) => strings.action_context_menu,
        None => return named(&action.name, &action.keyword),
    };
    builtin.into()
}

pub(crate) fn command_label(command: &CommandSpec, strings: &Strings) -> SharedString {
    let builtin = match command.builtin {
        Some(BuiltinCommand::Settings) => strings.command_settings,
        Some(BuiltinCommand::Rebuild) => strings.command_rebuild,
        Some(BuiltinCommand::Parent) => strings.command_parent,
        Some(BuiltinCommand::NewFolder) => strings.command_new_folder,
        Some(BuiltinCommand::NewFile) => strings.command_new_file,
        Some(BuiltinCommand::Cmd) => strings.command_cmd,
        Some(BuiltinCommand::CmdAdmin) => strings.command_cmd_admin,
        Some(BuiltinCommand::PowerShell) => strings.command_powershell,
        Some(BuiltinCommand::PowerShellAdmin) => strings.command_powershell_admin,
        Some(BuiltinCommand::Terminal) => strings.command_terminal,
        Some(BuiltinCommand::Shutdown) => strings.command_shutdown,
        Some(BuiltinCommand::Restart) => strings.command_restart,
        Some(BuiltinCommand::Uninstall) => strings.command_uninstall,
        Some(BuiltinCommand::Connections) => strings.command_connections,
        Some(BuiltinCommand::Hosts) => strings.command_hosts,
        None => return named(&command.name, &command.keyword),
    };
    builtin.into()
}

fn named(name: &str, keyword: &str) -> SharedString {
    let name = name.trim();
    if name.is_empty() { keyword.trim().to_owned().into() } else { name.to_owned().into() }
}

pub(crate) fn extras(
    text: &str,
    place: Option<HostKind>,
    settings: &Settings,
    language: Language,
) -> (Vec<Extra>, Vec<Extra>) {
    let strings = language.strings();
    let text = text.trim();
    let (word, rest) = text.split_once(char::is_whitespace).map_or((text, ""), |(word, rest)| (word, rest.trim()));
    let word = word.to_lowercase();
    let swapped = search::other_layout(&word);
    let named = |keyword: &str| {
        let keyword = keyword.trim().to_lowercase();
        !keyword.is_empty() && (keyword == word || swapped.as_ref() == Some(&keyword))
    };
    let mut before = Vec::new();
    for (index, command) in settings.commands.iter().enumerate() {
        let takes_query = command.takes_query();
        if !command.enabled || !named(&command.keyword) || !(takes_query || rest.is_empty()) || !applies(command, place)
        {
            continue;
        }
        let detail = match (takes_query, rest, command.builtin) {
            (false, _, _) => strings.command_hint.to_owned(),
            (true, "", Some(_)) => strings.new_name_hint.to_owned(),
            (true, "", None) => strings.argument_hint.to_owned(),
            (true, rest, _) => rest.to_owned(),
        };
        let title = command_label(command, strings).to_string();
        before.push(Extra { tool: Tool::Command(index), title, detail, argument: rest.to_owned() });
    }
    for (index, web) in settings.web.iter().enumerate().filter(|(_, web)| web.enabled && named(&web.keyword)) {
        before.push(web_row(index, web, rest, language));
    }
    let suggested = place.is_none() && settings.web_suggestions && before.is_empty() && !text.is_empty();
    let after = settings
        .web
        .iter()
        .enumerate()
        .find(|(_, web)| suggested && web.usable())
        .map(|(index, web)| web_row(index, web, text, language))
        .into_iter()
        .collect();
    (before, after)
}

fn applies(command: &CommandSpec, place: Option<HostKind>) -> bool {
    match command.builtin {
        Some(BuiltinCommand::Parent) => matches!(place, Some(HostKind::Explorer | HostKind::Dialog)),
        Some(_) | None => true,
    }
}

fn web_row(index: usize, web: &WebSearch, query: &str, language: Language) -> Extra {
    let name = web.name.trim();
    let (title, detail) = if query.is_empty() {
        (name.to_owned(), language.strings().argument_hint.to_owned())
    } else {
        (language.web(name, query), web.link(query))
    };
    Extra { tool: Tool::Web(index), title, detail, argument: query.to_owned() }
}

pub(crate) fn action_rows(targets: &[Hit], settings: &Settings, strings: &Strings) -> Vec<Extra> {
    let Some(first) = targets.first() else { return Vec::new() };
    let dir = targets.iter().any(|hit| hit.dir);
    settings
        .actions
        .iter()
        .enumerate()
        .filter(|(_, action)| action.enabled && action.scope != Scope::Explorer && offers(action, dir, targets.len()))
        .map(|(index, action)| Extra {
            tool: Tool::Action { index, dir, builtin: action.builtin },
            title: action_label(action, strings).to_string(),
            detail: String::new(),
            argument: first.path.clone(),
        })
        .collect()
}

fn offers(action: &ActionSpec, dir: bool, count: usize) -> bool {
    match action.builtin {
        Some(builtin) if count > 1 => builtin.takes_many(),
        Some(BuiltinAction::RunAsAdmin | BuiltinAction::OpenWith) => !dir,
        Some(_) => true,
        None => count == 1 && !action.program.path.trim().is_empty(),
    }
}

pub(crate) fn action_paths(action: &ActionSpec, hit: &Hit, targets: Vec<Hit>) -> Vec<String> {
    match action.builtin {
        Some(builtin) if builtin.takes_many() => targets.into_iter().map(|target| target.path).collect(),
        Some(_) | None => vec![hit.path.clone()],
    }
}

pub(crate) fn perform_action(action: &ActionSpec, subject: Subject, cx: &mut App) {
    let Some(first) = subject.paths.first().cloned() else { return };
    let command = match action.builtin {
        Some(BuiltinAction::Open) => Command::Open(first),
        Some(BuiltinAction::OpenWith) => Command::OpenWith(first),
        Some(BuiltinAction::Reveal) => Command::Reveal(first),
        Some(BuiltinAction::RunAsAdmin) => Command::RunAsAdmin(first),
        Some(BuiltinAction::Copy) => Command::PutFiles { paths: subject.paths, cut: false },
        Some(BuiltinAction::Cut) => Command::PutFiles { paths: subject.paths, cut: true },
        Some(BuiltinAction::Delete) => Command::Recycle(subject.paths),
        Some(BuiltinAction::Properties) => Command::Properties(first),
        Some(BuiltinAction::ContextMenu) => {
            super::open_shell_menu(first, cx);
            return;
        }
        Some(BuiltinAction::CopyPath) => {
            cx.write_to_clipboard(ClipboardItem::new_string(subject.paths.join("\r\n")));
            return;
        }
        Some(BuiltinAction::CopyName) => {
            let names: Vec<&str> = subject
                .paths
                .iter()
                .map(|path| Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path))
                .collect();
            cx.write_to_clipboard(ClipboardItem::new_string(names.join("\r\n")));
            return;
        }
        None => Command::Run(subject.program(&action.program)),
    };
    perform(command, cx);
}

pub(crate) fn perform_command(command: &CommandSpec, subject: Subject, host: Option<Host>, cx: &mut App) {
    let job = match command.builtin {
        Some(BuiltinCommand::Settings) => return platform::send(Event::OpenSettings),
        Some(BuiltinCommand::Rebuild) => return platform::send(Event::Rebuild),
        Some(BuiltinCommand::Parent) => {
            let (Some(host), Some(folder)) = (host, subject.folder.as_deref()) else { return };
            let Some(parent) = Path::new(folder).parent().and_then(Path::to_str) else { return };
            let path = parent.to_owned();
            match host.kind {
                HostKind::Explorer => Command::Navigate(host, path),
                HostKind::Desktop => Command::Open(path),
                HostKind::Dialog => Command::Choose { host, path, keep_name: true },
            }
        }
        Some(builtin @ (BuiltinCommand::NewFolder | BuiltinCommand::NewFile)) => {
            let Some(path) = new_path(&subject) else { return };
            Command::Create { path, folder: builtin == BuiltinCommand::NewFolder, reveal: host.is_none() }
        }
        Some(BuiltinCommand::Hosts) => {
            let root = std::env::var(SYSTEM_ROOT).unwrap_or_else(|_| DEFAULT_SYSTEM_ROOT.to_owned());
            let hosts = Path::new(&root).join(HOSTS);
            let arguments = format!("\"{}\"", hosts.display());
            Command::Run(Program { path: "notepad.exe".into(), arguments, admin: true, ..Program::default() })
        }
        Some(_) | None => Command::Run(subject.program(&command.program)),
    };
    perform(job, cx);
}

fn new_path(subject: &Subject) -> Option<String> {
    let name = subject.query.trim();
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return None;
    }
    Some(Path::new(subject.folder.as_deref()?).join(name).to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Pairs = Vec<(Tool, String)>;

    fn subject() -> Subject {
        Subject {
            paths: vec!["C:\\Work\\plan v2.txt".into(), "C:\\Work\\notes.md".into()],
            folder: Some("D:\\Projects".into()),
            query: "new name".into(),
        }
    }

    #[test]
    fn variables_expand_in_paths_arguments_and_folders() {
        let program = Program {
            path: "code".into(),
            arguments: "--goto \"{action_path}\" \"{action_path:parent}\" {query}".into(),
            folder: "{current_folder}\\sub".into(),
            hidden: true,
            admin: false,
        };
        let expanded = subject().program(&program);
        assert_eq!(expanded.arguments, "--goto \"C:\\Work\\plan v2.txt\" \"C:\\Work\" new name");
        assert_eq!(expanded.folder, "D:\\Projects\\sub");
        assert!(expanded.hidden);
    }

    #[test]
    fn an_empty_working_folder_is_the_current_one() {
        let expanded = subject().program(&Program { path: "cmd.exe".into(), ..Program::default() });
        assert_eq!(expanded.folder, "D:\\Projects");
        let without = Subject { folder: None, ..subject() }.program(&Program::default());
        assert_eq!(without.folder, "C:\\Work");
    }

    #[test]
    fn new_names_must_be_plain_names_in_a_known_folder() {
        assert_eq!(new_path(&subject()).as_deref(), Some("D:\\Projects\\new name"));
        assert_eq!(new_path(&Subject { query: "a/b".into(), ..subject() }), None);
        assert_eq!(new_path(&Subject { query: "  ".into(), ..subject() }), None);
        assert_eq!(new_path(&Subject { folder: None, ..subject() }), None);
    }

    fn tools(text: &str, place: Option<HostKind>) -> (Pairs, Pairs) {
        let (before, after) = extras(text, place, &Settings::default(), Language::English);
        let pairs = |extras: Vec<Extra>| extras.into_iter().map(|extra| (extra.tool, extra.argument)).collect();
        (pairs(before), pairs(after))
    }

    fn command(builtin: BuiltinCommand) -> Tool {
        let position = CommandSpec::defaults().iter().position(|command| command.builtin == Some(builtin));
        Tool::Command(position.expect("built-in command"))
    }

    #[test]
    fn a_command_keyword_takes_the_rest_as_its_argument() {
        let (before, _) = tools("  MKDIR  Новая папка ", Some(HostKind::Explorer));
        assert_eq!(before, [(command(BuiltinCommand::NewFolder), "Новая папка".to_owned())]);
        assert_eq!(tools("cmd", None).0, [(command(BuiltinCommand::Cmd), String::new())]);
        assert!(tools("cmd notes", None).0.is_empty());
        assert!(tools("cmdx", None).0.is_empty());
    }

    #[test]
    fn keywords_typed_in_the_other_layout_still_match() {
        assert_eq!(tools("сьв", None).0, [(command(BuiltinCommand::Cmd), String::new())]);
        assert_eq!(
            tools("ЬЛВШК отчёт", Some(HostKind::Explorer)).0,
            [(command(BuiltinCommand::NewFolder), "отчёт".to_owned())]
        );
        assert_eq!(tools("пр gpui", None).0, [(Tool::Web(7), "gpui".to_owned())]);
        assert_eq!(tools("dbrb ркуст", None).0, [(Tool::Web(2), "ркуст".to_owned())]);
    }

    #[test]
    fn windows_terminal_starts_switched_off() {
        assert!(tools("wt", None).0.is_empty());
        let mut settings = Settings::default();
        for command in &mut settings.commands {
            command.enabled = true;
        }
        let (before, _) = extras("wt", None, &settings, Language::English);
        assert_eq!(before.into_iter().map(|extra| extra.tool).collect::<Vec<_>>(), [command(BuiltinCommand::Terminal)]);
    }

    #[test]
    fn going_up_needs_a_folder_window() {
        assert!(tools("..", None).0.is_empty());
        assert_eq!(tools("..", Some(HostKind::Dialog)).0, [(command(BuiltinCommand::Parent), String::new())]);
    }

    #[test]
    fn web_keywords_search_the_rest_and_plain_queries_suggest_the_first_engine() {
        let (before, after) = tools("gh gpui", None);
        assert_eq!(before, [(Tool::Web(7), "gpui".to_owned())]);
        assert!(after.is_empty());
        let (before, after) = tools("отчёт 2026", None);
        assert!(before.is_empty());
        assert_eq!(after, [(Tool::Web(0), "отчёт 2026".to_owned())]);
        assert!(tools("отчёт", Some(HostKind::Explorer)).1.is_empty());
        let mut settings = Settings::default();
        settings.web.iter_mut().take(2).for_each(|web| web.enabled = false);
        let (_, after) = extras("report", None, &settings, Language::English);
        assert_eq!(after.first().map(|extra| extra.tool), Some(Tool::Web(2)));
    }

    #[test]
    fn folders_are_not_offered_programs_only_actions() {
        let settings = Settings::default();
        let file = action_rows(&[Hit::new("C:\\a.exe".into(), false)], &settings, Language::English.strings());
        let folder = action_rows(&[Hit::new("C:\\Work".into(), true)], &settings, Language::English.strings());
        let admin = |rows: &[Extra]| rows.iter().any(|row| row.title == Language::English.strings().action_admin);
        assert!(admin(&file));
        assert!(!admin(&folder));
        assert_eq!(file.len(), folder.len() + 2);
        assert!(file.iter().all(|row| row.argument == "C:\\a.exe"));
    }

    #[test]
    fn several_files_are_offered_only_the_actions_that_take_them_all() {
        let settings = Settings::default();
        let strings = Language::English.strings();
        let both = [Hit::new("C:\\a.txt".into(), false), Hit::new("C:\\b.txt".into(), false)];
        let titles: Vec<String> = action_rows(&both, &settings, strings).into_iter().map(|row| row.title).collect();
        let expected = [
            strings.action_copy,
            strings.action_cut,
            strings.action_copy_path,
            strings.action_copy_name,
            strings.action_delete,
        ];
        assert_eq!(titles, expected);
    }
}
