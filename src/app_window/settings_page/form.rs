use super::SettingsPage;
use super::{Section, change};
use crate::i18n::{Language, Strings};
use crate::input::{Changed, TextInput};
use crate::platform;
use crate::search::{Builtin, Filter};
use crate::settings::{ActionSpec, CommandSpec, Favorite, Settings, Shortcut, WebSearch};
use crate::theme::Theme;
use crate::ui::{self, FIELD_WIDTH, text_field};
use gpui::{AppContext, Context, Div, Entity, Focusable, Keystroke, Subscription, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    ExcludedTypes,
    DisabledApps,
    OpenerPath,
    OpenerArguments,
    ApiPort,
    Name,
    Keyword,
    Extensions,
    Folder,
    Path,
    Arguments,
    WorkingFolder,
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Filter(usize),
    Action(usize),
    Command(usize),
    Web(usize),
    Favorite(usize),
    MenuTool(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Search,
    Window,
    Menu,
    Action(usize),
    Command(usize),
}

#[derive(Default)]
pub(in crate::app_window) struct Form {
    fields: Vec<(Field, Entity<TextInput>)>,
    pub(super) editing: Option<Item>,
    pub(super) lifted: Option<Item>,
    pub(super) recording: Option<Slot>,
    pub(super) keys_shown: bool,
    subscriptions: Vec<Subscription>,
}

impl Field {
    fn example(self, strings: &Strings) -> &'static str {
        match self {
            Self::ExcludedTypes => strings.example_types,
            Self::DisabledApps => strings.example_apps,
            Self::OpenerPath => strings.example_opener,
            Self::OpenerArguments => strings.example_opener_arguments,
            Self::ApiPort => "",
            Self::Name | Self::Keyword => "",
            Self::Extensions => strings.example_extensions,
            Self::Folder => strings.example_folder,
            Self::Path => strings.example_path,
            Self::Arguments => strings.example_arguments,
            Self::WorkingFolder => strings.example_working,
            Self::Url => strings.example_url,
        }
    }

    pub(super) fn label(self, strings: &Strings) -> &'static str {
        match self {
            Self::ExcludedTypes => strings.excluded_types,
            Self::DisabledApps => strings.disabled_apps,
            Self::OpenerPath => strings.opener,
            Self::OpenerArguments => strings.opener_arguments,
            Self::ApiPort => strings.api_port,
            Self::Name => strings.field_name,
            Self::Keyword => strings.field_keyword,
            Self::Extensions => strings.field_extensions,
            Self::Folder => strings.field_folder,
            Self::Path => strings.field_path,
            Self::Arguments => strings.field_arguments,
            Self::WorkingFolder => strings.field_working,
            Self::Url => strings.field_url,
        }
    }

    fn read(self, item: Option<Item>, settings: &mut Settings) -> String {
        match self {
            Self::DisabledApps => settings.disabled_apps.join("; "),
            Self::ApiPort => settings.api.port.to_string(),
            Self::ExcludedTypes
            | Self::OpenerPath
            | Self::OpenerArguments
            | Self::Name
            | Self::Keyword
            | Self::Extensions
            | Self::Folder
            | Self::Path
            | Self::Arguments
            | Self::WorkingFolder
            | Self::Url => self.text(item, settings).cloned().unwrap_or_default(),
        }
    }

    fn write(self, item: Option<Item>, settings: &mut Settings, value: &str) {
        match self {
            Self::DisabledApps => {
                let apps = value.split([';', ',']).map(str::trim).filter(|app| !app.is_empty());
                settings.disabled_apps = apps.map(str::to_owned).collect();
            }
            Self::ApiPort => {
                if let Ok(port) = value.trim().parse::<u16>()
                    && port > 0
                {
                    settings.api.port = port;
                }
            }
            Self::ExcludedTypes
            | Self::OpenerPath
            | Self::OpenerArguments
            | Self::Name
            | Self::Keyword
            | Self::Extensions
            | Self::Folder
            | Self::Path
            | Self::Arguments
            | Self::WorkingFolder
            | Self::Url => {
                if let (Self::Name, Some(Item::Command(position))) = (self, item) {
                    settings.rename_command(position, value);
                } else if let Some(text) = self.text(item, settings) {
                    value.clone_into(text);
                }
            }
        }
    }

    fn text(self, item: Option<Item>, settings: &mut Settings) -> Option<&mut String> {
        match item {
            None => Some(find(&GLOBAL, self)?(settings)),
            Some(Item::Filter(position)) => Some(find(&FILTER, self)?(settings.filters.get_mut(position)?)),
            Some(Item::Action(position)) => Some(find(&ACTION, self)?(settings.actions.get_mut(position)?)),
            Some(Item::Command(position)) => Some(find(&COMMAND, self)?(settings.commands.get_mut(position)?)),
            Some(Item::Web(position)) => Some(find(&WEB, self)?(settings.web.get_mut(position)?)),
            Some(Item::Favorite(position)) => Some(find(&FAVORITE, self)?(settings.menu.favorites.get_mut(position)?)),
            Some(Item::MenuTool(_)) => None,
        }
    }
}

type Text<T> = (Field, fn(&mut T) -> &mut String);

const GLOBAL: [Text<Settings>; 3] = [
    (Field::ExcludedTypes, |settings| &mut settings.suggestions.excluded_types),
    (Field::OpenerPath, |settings| &mut settings.opener.path),
    (Field::OpenerArguments, |settings| &mut settings.opener.arguments),
];
const FILTER: [Text<Filter>; 4] = [
    (Field::Name, |filter| &mut filter.name),
    (Field::Keyword, |filter| &mut filter.keyword),
    (Field::Extensions, |filter| &mut filter.extensions),
    (Field::Folder, |filter| &mut filter.folder),
];
const ACTION: [Text<ActionSpec>; 5] = [
    (Field::Name, |action| &mut action.name),
    (Field::Keyword, |action| &mut action.keyword),
    (Field::Path, |action| &mut action.program.path),
    (Field::Arguments, |action| &mut action.program.arguments),
    (Field::WorkingFolder, |action| &mut action.program.folder),
];
const COMMAND: [Text<CommandSpec>; 5] = [
    (Field::Name, |command| &mut command.name),
    (Field::Keyword, |command| &mut command.keyword),
    (Field::Path, |command| &mut command.program.path),
    (Field::Arguments, |command| &mut command.program.arguments),
    (Field::WorkingFolder, |command| &mut command.program.folder),
];
const WEB: [Text<WebSearch>; 3] =
    [(Field::Name, |web| &mut web.name), (Field::Keyword, |web| &mut web.keyword), (Field::Url, |web| &mut web.url)];
const FAVORITE: [Text<Favorite>; 2] =
    [(Field::Name, |favorite| &mut favorite.name), (Field::Path, |favorite| &mut favorite.path)];

fn find<T>(texts: &[Text<T>], field: Field) -> Option<fn(&mut T) -> &mut String> {
    texts.iter().find(|(known, _)| *known == field).map(|&(_, text)| text)
}

impl Item {
    pub(super) fn fields(self, settings: &Settings) -> &'static [Field] {
        match self {
            Self::Filter(position) => match settings.filters.get(position).and_then(|filter| filter.builtin) {
                Some(Builtin::Folders | Builtin::Files) => &[Field::Keyword, Field::Folder],
                Some(_) => &[Field::Keyword, Field::Extensions, Field::Folder],
                None => &[Field::Name, Field::Keyword, Field::Extensions, Field::Folder],
            },
            Self::Action(position) => match settings.actions.get(position).and_then(|action| action.builtin) {
                Some(_) => &[],
                None => &[Field::Name, Field::Path, Field::Arguments, Field::WorkingFolder],
            },
            Self::Command(position) => match settings.commands.get(position).and_then(|command| command.builtin) {
                Some(_) => &[Field::Keyword],
                None => &[Field::Name, Field::Keyword, Field::Path, Field::Arguments, Field::WorkingFolder],
            },
            Self::Web(_) => &[Field::Name, Field::Keyword, Field::Url],
            Self::Favorite(_) => &[Field::Name, Field::Path],
            Self::MenuTool(_) => &[],
        }
    }
}

impl Slot {
    pub(super) fn get(self, settings: &Settings) -> Option<&Shortcut> {
        match self {
            Self::Search => settings.hotkeys.search.as_ref(),
            Self::Window => settings.hotkeys.window.as_ref(),
            Self::Menu => settings.hotkeys.menu.as_ref(),
            Self::Action(position) => settings.actions.get(position)?.shortcut.as_ref(),
            Self::Command(position) => settings.commands.get(position)?.shortcut.as_ref(),
        }
    }

    fn accepts(self, shortcut: &Shortcut) -> bool {
        match self {
            Self::Search | Self::Window | Self::Menu => shortcut.has_modifier(),
            Self::Action(_) | Self::Command(_) => shortcut.spares_typing(),
        }
    }

    fn set(self, settings: &mut Settings, shortcut: Option<Shortcut>) {
        let slot = match self {
            Self::Search => &mut settings.hotkeys.search,
            Self::Window => &mut settings.hotkeys.window,
            Self::Menu => &mut settings.hotkeys.menu,
            Self::Action(position) => {
                let Some(action) = settings.actions.get_mut(position) else { return };
                &mut action.shortcut
            }
            Self::Command(position) => {
                let Some(command) = settings.commands.get_mut(position) else { return };
                &mut command.shortcut
            }
        };
        *slot = shortcut;
    }
}

impl Form {
    fn input(&self, field: Field) -> Option<&Entity<TextInput>> {
        self.fields.iter().find(|(known, _)| *known == field).map(|(_, input)| input)
    }
}

impl SettingsPage {
    pub(in crate::app_window) fn open_section(&mut self, section: Section, cx: &mut Context<Self>) {
        let same = self.section == section;
        self.section = section;
        if section == Section::Appearance {
            self.theme_editor.update(cx, |editor, cx| editor.load_fonts(cx));
        }
        if !same {
            self.form.editing = None;
            self.settings_scroll.set_offset(gpui::Point::default());
        }
        self.stop_recording(cx);
        self.rebuild_fields(cx);
    }

    pub(super) fn edit(&mut self, item: Item, cx: &mut Context<Self>) {
        self.form.editing = if self.form.editing == Some(item) { None } else { Some(item) };
        self.menus.close();
        self.rebuild_fields(cx);
    }

    pub(super) fn stop_editing(&mut self, cx: &mut Context<Self>) {
        self.form.editing = None;
        self.rebuild_fields(cx);
    }

    pub(in crate::app_window) fn close_editor(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.form.editing.is_some() || self.form.recording.is_some();
        if open {
            self.stop_recording(cx);
            self.stop_editing(cx);
        }
        open
    }

    pub(super) fn rebuild_fields(&mut self, cx: &mut Context<Self>) {
        self.form.fields.clear();
        self.form.subscriptions.clear();
        {
            let section = self.section;
            let mut settings = cx.global::<Settings>().clone();
            let item_fields = self.form.editing.map_or(&[][..], |item| item.fields(&settings));
            for &field in section.fields().iter().chain(item_fields) {
                self.add_field(field, field.read(self.form.editing, &mut settings), cx);
            }
        }
        cx.notify();
    }

    fn add_field(&mut self, field: Field, value: String, cx: &mut Context<Self>) {
        let example = field.example(cx.global::<Language>().strings());
        let under = cx.global::<Theme>().app.background;
        let active = self.finder.read(cx).blinking();
        let input = cx.new(TextInput::field);
        input.update(cx, |input, cx| {
            input.set_text(&value, cx);
            input.set_placeholder(example);
            input.set_under(under, cx);
            input.set_blinking(active, cx);
        });
        self.form
            .subscriptions
            .push(cx.subscribe(&input, move |this, _, _: &Changed, cx| this.field_changed(field, cx)));
        self.form.subscriptions.push(ui::paint_changes_of(&input, cx));
        self.form.fields.push((field, input));
    }

    pub(in crate::app_window) fn set_fields_blinking(&self, active: bool, cx: &mut Context<Self>) {
        let editing = self.theme_editor.read(cx).inputs().map(Entity::clone);
        let diagnosing = self.diagnostics.input().clone();
        let fields = self.form.fields.iter().map(|(_, input)| input.clone());
        for input in fields.chain(editing).chain([diagnosing]) {
            input.update(cx, |input, cx| input.set_blinking(active, cx));
        }
    }

    fn field_changed(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(input) = self.form.input(field) else { return };
        let value = input.read(cx).text().to_owned();
        let item = self.form.editing;
        change(cx, |settings| field.write(item, settings, &value));
    }

    pub(super) fn field(&self, field: Field, theme: &Theme, window: &Window, cx: &Context<Self>) -> Option<Div> {
        let input = self.form.input(field)?;
        let focused = input.focus_handle(cx).is_focused(window);
        Some(text_field(input.clone(), focused, FIELD_WIDTH, theme))
    }

    pub(super) fn start_recording(&mut self, slot: Slot, cx: &mut Context<Self>) {
        self.form.recording = Some(slot);
        self.menus.close();
        platform::pause_hotkeys(true);
        cx.notify();
    }

    pub(in crate::app_window) fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.form.recording.take().is_some() {
            platform::pause_hotkeys(false);
            cx.notify();
        }
    }

    pub(in crate::app_window) fn record_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let Some(slot) = self.form.recording else { return false };
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.control && !modifiers.alt && !modifiers.shift && !modifiers.platform;
        match keystroke.key.as_str() {
            "escape" if plain => self.stop_recording(cx),
            "backspace" | "delete" if plain => {
                change(cx, |settings| slot.set(settings, None));
                self.stop_recording(cx);
            }
            key => {
                let Some((key, shifted)) = platform::layout_key(key) else { return true };
                let shift = modifiers.shift || shifted;
                let shortcut = Shortcut::new(modifiers.control, modifiers.alt, shift, modifiers.platform, &key);
                if let Some(shortcut) = shortcut.filter(|shortcut| slot.accepts(shortcut)) {
                    change(cx, |settings| slot.set(settings, Some(shortcut)));
                    self.stop_recording(cx);
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_shown_for_an_item_reads_and_writes_its_text() {
        let mut settings = Settings::default();
        settings.filters.push(Filter::default());
        settings.actions.push(ActionSpec::default());
        settings.commands.push(CommandSpec::default());
        settings.web.push(WebSearch::default());
        settings.menu.favorites.push(Favorite::default());
        let mut items = Vec::new();
        items.extend((0..settings.filters.len()).map(Item::Filter));
        items.extend((0..settings.actions.len()).map(Item::Action));
        items.extend((0..settings.commands.len()).map(Item::Command));
        items.extend((0..settings.web.len()).map(Item::Web));
        items.extend((0..settings.menu.favorites.len()).map(Item::Favorite));
        for item in items {
            for &field in item.fields(&settings.clone()) {
                field.write(Some(item), &mut settings, "C:\\Проверка");
                assert_eq!(field.read(Some(item), &mut settings), "C:\\Проверка", "{field:?} of {item:?}");
            }
        }
    }

    #[test]
    fn every_section_field_reads_back_what_was_written() {
        let mut settings = Settings::default();
        for field in [Field::ExcludedTypes, Field::DisabledApps, Field::OpenerPath, Field::OpenerArguments] {
            field.write(None, &mut settings, "проверка");
            assert_eq!(field.read(None, &mut settings), "проверка", "{field:?}");
        }
    }
}
