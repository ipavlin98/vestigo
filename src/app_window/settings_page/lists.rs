use super::super::view::filter_label;
use super::SettingsPage;
use super::form::Item;
use super::{
    Block, FIELD_PADDING, Folding, Heading, Menu, Row, Section, change, field_row, fold_row, section, setting,
    setting_row,
};
use crate::i18n::Strings;
use crate::search::Filter;
use crate::settings::{Favorite, MenuTool, Settings, WebSearch};
use crate::theme::{Theme, rem};
use crate::ui::{self, Choice, DETAIL_LINE, DETAIL_SIZE, button, toggle};
use gpui::{App, Context, Div, ElementId, PathPromptOptions, SharedString, Stateful, Window, div, prelude::*};
use std::path::Path;

impl SettingsPage {
    pub(super) fn list_item(
        &self,
        item: Item,
        (label, hint): (SharedString, SharedString),
        switch: Option<Stateful<Div>>,
        editor: Vec<Row>,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Row {
        let ordered = item.ordered();
        let mut controls: Vec<_> =
            ordered.then(|| self.steps(item, theme, cx).into_any_element()).into_iter().collect();
        controls.extend(switch.map(IntoElement::into_any_element));
        let grip = ordered.then(|| self.grip(item, (label.clone(), hint.clone()), theme, cx));
        let folding = Folding {
            id: ElementId::Name(format!("edit-{item:?}").into()),
            open: self.form.editing == Some(item),
            controls,
            toggle: Box::new(cx.listener(move |this, _, _, cx| this.edit(item, cx))),
            inner: editor,
            unfold: None,
        };
        let row = fold_row((label, hint), folding);
        match grip {
            Some(grip) => row.grip(grip),
            None => row,
        }
    }

    pub(super) fn item_fields(&self, item: Item, theme: &Theme, window: &Window, cx: &Context<Self>) -> Vec<Row> {
        let strings = cx.global::<crate::i18n::Language>().strings();
        item.fields(cx.global::<Settings>())
            .iter()
            .filter_map(|&field| Some(field_row(field.label(strings), self.field(field, theme, window, cx)?)))
            .collect()
    }

    pub(super) fn delete_row(
        &self,
        id: &'static str,
        item: Item,
        strings: &Strings,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Row {
        let delete = button(
            id,
            strings.delete,
            true,
            theme,
            cx.listener(move |this, _, _, cx| {
                this.remove_item(item, cx);
            }),
        );
        field_row("", delete)
    }

    pub(super) fn add_row(
        &self,
        id: &'static str,
        label: &'static str,
        strings: &Strings,
        add: impl Fn(&mut Settings) -> Item + 'static,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Row {
        let control = button(
            id,
            strings.add,
            true,
            theme,
            cx.listener(move |this, _, _, cx| {
                let mut added = None;
                change(cx, |settings| added = Some(add(settings)));
                if let Some(item) = added {
                    this.form.editing = None;
                    this.edit(item, cx);
                }
            }),
        );
        setting(label, "", control)
    }

    pub(super) fn pick_folder(&mut self, apply: impl Fn(&mut Settings, String) + 'static, cx: &mut Context<Self>) {
        let options = PathPromptOptions { files: false, directories: true, multiple: false, prompt: None };
        let chosen = cx.prompt_for_paths(options);
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Some(path) = paths.into_iter().next().and_then(|path| path.into_os_string().into_string().ok()) else {
                return;
            };
            cx.update(|cx| change(cx, |settings| apply(settings, path)));
        })
        .detach();
    }

    pub(super) fn filters(&self, theme: &Theme, strings: &Strings, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let filters = cx.global::<Settings>().filters.clone();
        let mut rows = Vec::new();
        for (position, filter) in filters.iter().enumerate() {
            let item = Item::Filter(position);
            let switch = toggle(("filter", position), filter.enabled, theme).on_click(move |_, _, cx| {
                change(cx, |settings| {
                    if let Some(filter) = settings.filters.get_mut(position) {
                        filter.enabled = !filter.enabled;
                    }
                })
            });
            let custom = filter.builtin.is_none();
            let editor = if self.form.editing != Some(item) {
                Vec::new()
            } else {
                let mut rows = self.item_fields(item, theme, window, cx);
                if custom {
                    rows.push(self.delete_row("delete-filter", item, strings, theme, cx));
                }
                rows
            };
            rows.push(self.list_item(
                item,
                (filter_label(filter, strings), filter_hint(filter)),
                Some(switch),
                editor,
                theme,
                cx,
            ));
        }
        rows.push(self.add_row(
            "add-filter",
            strings.add_filter,
            strings,
            {
                let name = strings.new_filter;
                move |settings| {
                    settings.filters.push(Filter { name: name.to_owned(), ..Filter::default() });
                    Item::Filter(settings.filters.len() - 1)
                }
            },
            theme,
            cx,
        ));
        vec![section(Heading::Note(strings.filters_hint), rows)]
    }

    pub(super) fn web(&self, theme: &Theme, strings: &Strings, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let searches = cx.global::<Settings>().web.clone();
        let mut rows = Vec::new();
        for (position, web) in searches.iter().enumerate() {
            let item = Item::Web(position);
            let switch = toggle(("web", position), web.enabled, theme).on_click(move |_, _, cx| {
                change(cx, |settings| {
                    if let Some(web) = settings.web.get_mut(position) {
                        web.enabled = !web.enabled;
                    }
                })
            });
            let editor = if self.form.editing != Some(item) {
                Vec::new()
            } else {
                let mut rows = vec![intro(strings.web_variables, theme)];
                rows.extend(self.item_fields(item, theme, window, cx));
                rows.push(self.delete_row("delete-web", item, strings, theme, cx));
                rows
            };
            let hint = SharedString::from(web.keyword.clone());
            rows.push(self.list_item(item, (web.name.clone().into(), hint), Some(switch), editor, theme, cx));
        }
        rows.push(self.add_row(
            "add-web",
            strings.add_web,
            strings,
            {
                let name = strings.new_web;
                move |settings| {
                    settings.web.push(WebSearch { name: name.to_owned(), ..WebSearch::default() });
                    Item::Web(settings.web.len() - 1)
                }
            },
            theme,
            cx,
        ));
        let default = self.default_web(theme, window, cx).map(|control| {
            section(Heading::Plain, vec![setting(strings.default_web, strings.default_web_hint, control)])
        });
        default.into_iter().chain([section(Heading::Note(strings.web_hint), rows)]).collect()
    }

    fn default_web(&self, theme: &Theme, window: &Window, cx: &Context<Self>) -> Option<Div> {
        let choices: Vec<_> = cx
            .global::<Settings>()
            .web
            .iter()
            .enumerate()
            .filter(|(_, web)| web.usable())
            .map(|(position, web)| Choice { value: position, label: web.name.clone().into(), note: None, swatch: None })
            .collect();
        let current = choices.first()?.value;
        let picker = self.picker(Menu::DefaultWeb, choices, current, theme);
        let this = cx.weak_entity();
        let apply = move |position, cx: &mut App| {
            let _ = this.update(cx, |this, cx| this.move_item(Item::Web(position), 0, cx));
        };
        Some(self.dropdown(Menu::DefaultWeb, picker, apply, theme, window, cx))
    }

    pub(super) fn favorites(
        &self,
        theme: &Theme,
        strings: &Strings,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let settings = cx.global::<Settings>();
        let mouse = |id: &'static str, on: bool, label, hint, flip: fn(&mut Settings)| {
            setting(label, hint, toggle(id, on, theme).on_click(move |_, _, cx| change(cx, flip)))
        };
        let triggers = vec![
            mouse(
                "double-click",
                settings.menu.double_click,
                strings.double_click,
                strings.double_click_hint,
                |settings| settings.menu.double_click = !settings.menu.double_click,
            ),
            mouse(
                "middle-click",
                settings.menu.middle_click,
                strings.middle_click,
                strings.middle_click_hint,
                |settings| settings.menu.middle_click = !settings.menu.middle_click,
            ),
        ];
        let mut rows = Vec::new();
        for (position, favorite) in settings.menu.favorites.iter().enumerate() {
            let item = Item::Favorite(position);
            let editor = if self.form.editing != Some(item) {
                Vec::new()
            } else {
                let mut rows = self.item_fields(item, theme, window, cx);
                rows.push(self.delete_row("delete-favorite", item, strings, theme, cx));
                rows
            };
            let name = if favorite.name.trim().is_empty() { favorite.path.clone() } else { favorite.name.clone() };
            rows.push(self.list_item(item, (name.into(), favorite.path.clone().into()), None, editor, theme, cx));
        }
        let add = button(
            "add-favorite",
            strings.add,
            true,
            theme,
            cx.listener(|this, _, _, cx| {
                this.pick_folder(
                    |settings, path| {
                        let name =
                            Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or(&path).to_owned();
                        settings.menu.favorites.push(Favorite { name, path });
                    },
                    cx,
                )
            }),
        );
        rows.push(setting(strings.add_favorite, "", add));
        let mut blocks = vec![section(Heading::Plain, triggers), section(Heading::Note(strings.favorites_hint), rows)];
        blocks.extend(self.menu_tools(settings, theme, strings, cx));
        blocks
    }

    fn menu_tools(&self, settings: &Settings, theme: &Theme, strings: &Strings, cx: &Context<Self>) -> Vec<Block> {
        let mut shown = Vec::new();
        for (position, tool) in settings.menu.tools.iter().enumerate() {
            let item = Item::MenuTool(position);
            let (label, hint) = tool_label(tool, settings, strings);
            let switch = toggle(("menu-tool-on", position), true, theme)
                .on_click(cx.listener(move |this, _, _, cx| this.remove_item(item, cx)));
            let controls = vec![self.steps(item, theme, cx).into_any_element(), switch.into_any_element()];
            let grip = self.grip(item, (label.clone(), hint.clone()), theme, cx);
            shown.push(setting_row(label, hints(hint), controls).id(("menu-tool", position)).grip(grip));
        }
        let hidden: Vec<Row> = hidden_tools(settings)
            .into_iter()
            .enumerate()
            .map(|(position, tool)| {
                let (label, hint) = tool_label(&tool, settings, strings);
                let switch = toggle(("menu-tool-off", position), false, theme)
                    .on_click(move |_, _, cx| change(cx, |settings| settings.menu.tools.push(tool.clone())));
                setting_row(label, hints(hint), vec![switch.into_any_element()])
            })
            .collect();
        let mut blocks = Vec::new();
        if !shown.is_empty() {
            blocks.push(section(Heading::Note(strings.menu_tools_hint), shown));
        }
        let to_commands = cx.listener(|this, _: &(), window, cx| {
            this.open(Section::Commands, window, cx);
        });
        let heading = Heading::Linked { note: strings.menu_tools_hidden_hint, link: strings.menu_tools_hidden_link };
        blocks.push(section(heading, hidden).follow(Box::new(move |window, cx| to_commands(&(), window, cx))));
        blocks
    }
}

fn hidden_tools(settings: &Settings) -> Vec<MenuTool> {
    let commands = settings.commands.iter().filter(|command| command.enabled && MenuTool::fits(command));
    let offered = [MenuTool::CopyPath, MenuTool::HiddenFiles].into_iter().chain(commands.map(MenuTool::of));
    offered.filter(|tool| !settings.menu.tools.contains(tool)).collect()
}

fn hints(hint: SharedString) -> Vec<SharedString> {
    if hint.is_empty() { Vec::new() } else { vec![hint] }
}

fn tool_label(tool: &MenuTool, settings: &Settings, strings: &Strings) -> (SharedString, SharedString) {
    match tool {
        MenuTool::CopyPath => (strings.menu_copy_folder_path.into(), SharedString::default()),
        MenuTool::HiddenFiles => (strings.menu_hidden_files.into(), SharedString::default()),
        MenuTool::Command(_) | MenuTool::Custom(_) => match tool.command(&settings.commands) {
            Some(command) => {
                let hint = if command.enabled { SharedString::default() } else { strings.menu_tool_off.into() };
                (ui::command_label(command, strings), hint)
            }
            None => {
                let name = if let MenuTool::Custom(name) = tool { name.clone() } else { String::new() };
                (name.into(), strings.menu_tool_off.into())
            }
        },
    }
}

pub(super) fn intro(text: &'static str, theme: &Theme) -> Row {
    Row::plain(
        div()
            .py(rem(FIELD_PADDING))
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(theme.settings.hint)
            .child(text),
    )
}

fn filter_hint(filter: &Filter) -> SharedString {
    let keyword = filter.keyword.trim();
    let mut parts = Vec::new();
    if !keyword.is_empty() {
        parts.push(format!("{}:", keyword.trim_end_matches(':')));
    }
    for detail in [filter.extensions.trim(), filter.folder.trim()] {
        if !detail.is_empty() {
            parts.push(detail.to_owned());
        }
    }
    parts.join("  ·  ").into()
}
