use super::SettingsPage;
use super::form::{Item, Slot};
use super::lists::intro;
use super::{Block, Heading, Menu, Row, change, field_row, section};
use crate::i18n::Strings;
use crate::settings::{ActionSpec, BuiltinCommand, CommandSpec, Program, Scope, Settings, Shortcut, Uninstaller};
use crate::theme::Theme;
use crate::ui::{Choice, action_label, command_label, toggle};
use gpui::{Context, SharedString, Window, prelude::*};

#[derive(Clone, Copy)]
enum Kind {
    Action,
    Command,
}

impl Kind {
    fn item(self, position: usize) -> Item {
        match self {
            Self::Action => Item::Action(position),
            Self::Command => Item::Command(position),
        }
    }

    fn slot(self, position: usize) -> Slot {
        match self {
            Self::Action => Slot::Action(position),
            Self::Command => Slot::Command(position),
        }
    }

    fn program(self, settings: &mut Settings, position: usize) -> Option<(&mut Program, &mut Scope, &mut bool)> {
        match self {
            Self::Action => {
                let action = settings.actions.get_mut(position)?;
                Some((&mut action.program, &mut action.scope, &mut action.enabled))
            }
            Self::Command => {
                let command = settings.commands.get_mut(position)?;
                Some((&mut command.program, &mut command.scope, &mut command.enabled))
            }
        }
    }
}

struct Tool {
    kind: Kind,
    position: usize,
    label: SharedString,
    hint: SharedString,
    enabled: bool,
    scope: Scope,
    custom: bool,
    program: Program,
    uninstaller: Option<Uninstaller>,
}

impl SettingsPage {
    pub(super) fn actions(&self, theme: &Theme, strings: &Strings, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let actions = cx.global::<Settings>().actions.clone();
        let tools = actions.iter().enumerate().map(|(position, action)| Tool {
            kind: Kind::Action,
            position,
            label: action_label(action, strings),
            hint: hint(action.shortcut.as_ref(), None, action.scope, None, strings),
            enabled: action.enabled,
            scope: action.scope,
            custom: action.builtin.is_none(),
            program: action.program.clone(),
            uninstaller: None,
        });
        let mut rows = Vec::new();
        rows.extend(tools.map(|tool| self.tool_row(tool, strings, theme, window, cx)));
        let name = strings.new_action;
        rows.push(self.add_row(
            "add-action",
            strings.add_action,
            strings,
            move |settings| {
                settings.actions.push(ActionSpec { name: name.to_owned(), ..ActionSpec::default() });
                Item::Action(settings.actions.len() - 1)
            },
            theme,
            cx,
        ));
        vec![section(Heading::Note(strings.actions_hint), rows)]
    }

    pub(super) fn commands(&self, theme: &Theme, strings: &Strings, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let commands = cx.global::<Settings>().commands.clone();
        let tools = commands.iter().enumerate().map(|(position, command)| {
            let warning = (command.enabled && command.builtin == Some(BuiltinCommand::Terminal))
                .then_some(strings.terminal_needed);
            Tool {
                kind: Kind::Command,
                position,
                label: command_label(command, strings),
                hint: hint(command.shortcut.as_ref(), Some(&command.keyword), command.scope, warning, strings),
                enabled: command.enabled,
                scope: command.scope,
                custom: command.builtin.is_none(),
                program: command.program.clone(),
                uninstaller: command.uninstaller(),
            }
        });
        let mut rows = Vec::new();
        rows.extend(tools.map(|tool| self.tool_row(tool, strings, theme, window, cx)));
        let name = strings.new_command;
        rows.push(self.add_row(
            "add-command",
            strings.add_command,
            strings,
            move |settings| {
                settings.commands.push(CommandSpec { name: name.to_owned(), ..CommandSpec::default() });
                Item::Command(settings.commands.len() - 1)
            },
            theme,
            cx,
        ));
        vec![section(Heading::Note(strings.commands_hint), rows)]
    }

    fn tool_row(&self, tool: Tool, strings: &Strings, theme: &Theme, window: &Window, cx: &Context<Self>) -> Row {
        let (kind, position) = (tool.kind, tool.position);
        let item = kind.item(position);
        let id = match kind {
            Kind::Action => "action",
            Kind::Command => "command",
        };
        let switch = toggle((id, position), tool.enabled, theme).on_click(move |_, _, cx| {
            change(cx, |settings| {
                if let Some((_, _, enabled)) = kind.program(settings, position) {
                    *enabled = !*enabled;
                }
            })
        });
        let editor = if self.form.editing == Some(item) {
            self.tool_editor(&tool, strings, theme, window, cx)
        } else {
            Vec::new()
        };
        self.list_item(item, (tool.label, tool.hint), Some(switch), editor, theme, cx)
    }

    fn tool_editor(
        &self,
        tool: &Tool,
        strings: &Strings,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Row> {
        let mut rows = Vec::new();
        let Tool { kind, position, scope, custom, .. } = *tool;
        if custom {
            let variables = match kind {
                Kind::Action => strings.action_variables,
                Kind::Command => strings.command_variables,
            };
            rows.push(intro(variables, theme));
        }
        rows.extend(self.item_fields(kind.item(position), theme, window, cx));
        rows.push(field_row(strings.field_shortcut, self.recorder(kind.slot(position), strings, theme, window, cx)));
        let scopes = Scope::ALL.map(|scope| Choice {
            value: scope,
            label: scope_label(scope, strings).into(),
            note: None,
            swatch: None,
        });
        let picker = self.picker(Menu::Scope, scopes.into(), scope, theme);
        let scope = self.dropdown(
            Menu::Scope,
            picker,
            move |scope, cx| {
                change(cx, |settings| {
                    if let Some((_, current, _)) = kind.program(settings, position) {
                        *current = scope;
                    }
                })
            },
            theme,
            window,
            cx,
        );
        rows.push(field_row(strings.field_scope, scope));
        if let Some(uninstaller) = tool.uninstaller {
            let choices = Uninstaller::ALL.map(|value| Choice {
                value,
                label: uninstaller_label(value, strings).into(),
                note: None,
                swatch: None,
            });
            let picker = self.picker(Menu::Uninstaller, choices.into(), uninstaller, theme);
            let choose = move |uninstaller: Uninstaller, cx: &mut gpui::App| {
                change(cx, |settings| {
                    if let Some(command) = settings.commands.get_mut(position) {
                        command.program = uninstaller.program();
                    }
                })
            };
            let dropdown = self.dropdown(Menu::Uninstaller, picker, choose, theme, window, cx);
            rows.push(field_row(strings.field_uninstaller, dropdown));
        }
        if custom {
            let flag = |id: &'static str, on: bool, pick: fn(&mut Program) -> &mut bool| {
                toggle((id, position), on, theme).on_click(move |_, _, cx| {
                    change(cx, |settings| {
                        if let Some((program, _, _)) = kind.program(settings, position) {
                            let flag = pick(program);
                            *flag = !*flag;
                        }
                    })
                })
            };
            rows.push(field_row(
                strings.field_hidden,
                flag("hidden", tool.program.hidden, |program| &mut program.hidden),
            ));
            rows.push(field_row(strings.field_admin, flag("admin", tool.program.admin, |program| &mut program.admin)));
            rows.push(self.delete_row("delete-tool", kind.item(position), strings, theme, cx));
        }
        rows
    }
}

fn hint(
    shortcut: Option<&Shortcut>,
    keyword: Option<&str>,
    scope: Scope,
    warning: Option<&str>,
    strings: &Strings,
) -> SharedString {
    let mut parts = Vec::new();
    if let Some(keyword) = keyword.map(str::trim).filter(|keyword| !keyword.is_empty()) {
        parts.push(keyword.to_owned());
    }
    if let Some(shortcut) = shortcut {
        parts.push(format!("{shortcut} · {}", scope_label(scope, strings)));
    }
    parts.extend(warning.map(str::to_owned));
    parts.join("  ·  ").into()
}

fn uninstaller_label(uninstaller: Uninstaller, strings: &Strings) -> &'static str {
    match uninstaller {
        Uninstaller::Classic => strings.uninstaller_classic,
        Uninstaller::Settings => strings.uninstaller_settings,
    }
}

fn scope_label(scope: Scope, strings: &Strings) -> &'static str {
    match scope {
        Scope::Vestigo => strings.scope_vestigo,
        Scope::Explorer => strings.scope_explorer,
        Scope::Anywhere => strings.scope_anywhere,
    }
}
