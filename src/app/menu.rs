use super::Shell;
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::platform::{self, Event, Host, HostKind};
use crate::settings::{CommandSpec, History, MenuTool, Settings};
use crate::ui::{self, Command, Go, MenuEntry, MenuIcon, MenuItem, Subject};
use futures::FutureExt;
use futures::future::Shared;
use gpui::{App, AppContext, ClipboardItem, Context, Task};
use std::rc::Rc;

type Folders = Shared<Task<Vec<String>>>;

pub(super) fn show(host: Option<Host>, cx: &mut Context<Shell>) {
    cx.spawn(async move |this, cx| {
        let (folder, hidden_shown) = cx
            .background_spawn(async move { (host.and_then(platform::host_folder), platform::hidden_files_shown()) })
            .await;
        let _ = this.update(cx, |_, cx| open(host, folder, hidden_shown, cx));
    })
    .detach();
}

pub(super) fn open(host: Option<Host>, current: Option<String>, hidden_shown: bool, cx: &mut App) {
    let visits = cx.global::<History>().visits();
    let recent = cx.background_spawn(async move { ui::recent_folders(visits) }).shared();
    let explorer = cx.background_spawn(async { platform::explorer_folders() }).shared();
    let strings: &'static Strings = cx.global::<Language>().strings();
    let go: Go = Rc::new(move |path, dir, cx| ui::perform(go(host, path, dir), cx));
    let reopened = current.clone();
    let reopen: ui::Choose = Rc::new(move |cx| open(host, reopened.clone(), hidden_shown, cx));
    let mut entries = ui::favorite_entries(current.as_deref(), reopen, &go, cx);
    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::Item(listed(strings.menu_recent, assets::HISTORY, recent, &go, strings)));
    entries.push(MenuEntry::Item(listed(strings.open_folders, assets::FOLDER, explorer, &go, strings)));
    let settings = cx.global::<Settings>();
    entries.push(MenuEntry::Separator);
    let tools = folder_tools(host, current, hidden_shown, settings, strings);
    if !tools.is_empty() {
        entries.extend(tools);
        entries.push(MenuEntry::Separator);
    }
    let open_settings = MenuItem::new(strings.command_settings, MenuIcon::Glyph(assets::SETTINGS))
        .on_choose(|_| platform::send(Event::OpenSettings));
    entries.push(MenuEntry::Item(open_settings));
    ui::open_menu(entries, cx);
}

fn listed(label: &'static str, glyph: &'static str, paths: Folders, go: &Go, strings: &'static Strings) -> MenuItem {
    let go = go.clone();
    MenuItem::new(label, MenuIcon::Glyph(glyph)).submenu(move |cx| {
        let (paths, go) = (paths.clone(), go.clone());
        cx.spawn(async move |_| {
            let paths = paths.await;
            if paths.is_empty() {
                return vec![ui::nothing(strings)];
            }
            ui::folder_entries(&paths, &go)
        })
    })
}

fn folder_tools(
    host: Option<Host>,
    current: Option<String>,
    hidden_shown: bool,
    settings: &Settings,
    strings: &'static Strings,
) -> Vec<MenuEntry> {
    let subject = Subject { folder: current.clone(), ..Subject::default() };
    let usable = |command: &&CommandSpec| command.enabled && MenuTool::fits(command);
    let mut entries = Vec::new();
    for tool in &settings.menu.tools {
        let item = match tool {
            MenuTool::CopyPath => current.clone().map(|current| {
                MenuItem::new(strings.menu_copy_folder_path, MenuIcon::Glyph(assets::COPY_PATH))
                    .on_choose(move |cx| cx.write_to_clipboard(ClipboardItem::new_string(current.clone())))
            }),
            MenuTool::HiddenFiles => Some(hidden_files(hidden_shown, strings)),
            MenuTool::Command(_) | MenuTool::Custom(_) => tool
                .command(&settings.commands)
                .filter(usable)
                .map(|spec| command(spec.clone(), subject.clone(), host, strings)),
        };
        entries.extend(item.map(MenuEntry::Item));
    }
    let pinned = |command: &CommandSpec| settings.menu.tools.contains(&MenuTool::of(command));
    let rest: Vec<CommandSpec> =
        settings.commands.iter().filter(usable).filter(|command| !pinned(command)).cloned().collect();
    if !rest.is_empty() {
        let all = MenuItem::new(strings.menu_commands, MenuIcon::Glyph(assets::ACTION)).submenu(move |_| {
            let entries = rest
                .iter()
                .map(|spec| MenuEntry::Item(command(spec.clone(), subject.clone(), host, strings)))
                .collect();
            Task::ready(entries)
        });
        entries.push(MenuEntry::Item(all));
    }
    entries
}

fn hidden_files(hidden_shown: bool, strings: &Strings) -> MenuItem {
    let label = if hidden_shown { strings.menu_hide_hidden } else { strings.menu_show_hidden };
    MenuItem::new(label, MenuIcon::Glyph(assets::EYE)).on_choose(move |cx| {
        cx.background_spawn(async move {
            if let Err(error) = platform::show_hidden_files(!hidden_shown) {
                log::warn!("{error:#}");
            }
        })
        .detach();
    })
}

fn command(spec: CommandSpec, subject: Subject, host: Option<Host>, strings: &Strings) -> MenuItem {
    let item = MenuItem::new(ui::command_label(&spec, strings), MenuIcon::Glyph(assets::COMMAND));
    let item = match spec.shortcut.as_ref() {
        Some(shortcut) => item.hint(shortcut.to_string()),
        None => item,
    };
    item.on_choose(move |cx| ui::perform_command(&spec, subject.clone(), host, cx))
}

fn go(host: Option<Host>, path: String, dir: bool) -> Command {
    match host {
        Some(host) if host.kind == HostKind::Explorer && dir => Command::Navigate(host, path),
        Some(host) if host.kind == HostKind::Dialog => Command::Choose { host, path, keep_name: dir },
        Some(_) | None => Command::Open(path),
    }
}
