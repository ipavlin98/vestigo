mod diagnose;
mod find;
mod form;
mod general;
mod index;
mod launcher;
mod lists;
mod order;
mod row;
mod theme_editor;
mod tools;

pub(super) use index::load_drives;
use index::load_estimate;
pub(super) use order::Renumber;

use super::menus::{MenuId, Menus, dropdown};
use super::view::top_bar;
use super::{refresh_autostart, reload_themes};
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::index::{Index, Status};
use crate::input::{Changed, TextInput};
use crate::settings::Settings;
use crate::theme::{Theme, rem};
use crate::ui::{
    self, Choice, Confirm, DETAIL_LINE, DETAIL_SIZE, Dismiss, Edge, GUTTER, INSET, NAME_LINE, Picker, SelectNext,
    SelectPrevious, Tip, divider, edge_fades, edge_scroll, glide, icon_button, path_label, scroll_to_child,
};
use crate::update::Updates;
use diagnose::Diagnostics;
use find::{Finding, Step};
use form::{Field, Form};
use gpui::{
    AnyElement, App, AppContext, Context, Div, ElementId, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    Render, ScrollHandle, SharedString, Subscription, Task, Window, div, prelude::*,
};
use row::{
    Block, Folding, Heading, Row, field_row, field_row_with, fold_row, lay_out, section, setting, setting_row,
    setting_with,
};
use std::sync::Arc;
use std::time::Duration;
use theme_editor::ThemeEditor;

const SECTION_TITLE: f32 = 48.0;
const ROW: f32 = 64.0;
const ROW_PADDING: f32 = 12.0;
const ROW_GAP: f32 = 24.0;
const FIELD_ROW: f32 = 48.0;
const FIELD_PADDING: f32 = 8.0;
const FOLD_END: f32 = 12.0;
const SETTINGS_WIDTH: f32 = 800.0;
const REVEAL_ID: &str = "settings-reveal";
const FINDER_KEY: &str = "/";
const FINDER_CHORD: &str = "f";
const CONFIRMING: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Section {
    General,
    Appearance,
    Launcher,
    Hotkeys,
    Filters,
    Index,
    Actions,
    Commands,
    Web,
    Menu,
    Integration,
    About,
}

impl Section {
    pub(super) const ALL: [Self; 12] = [
        Self::General,
        Self::Appearance,
        Self::Launcher,
        Self::Hotkeys,
        Self::Filters,
        Self::Index,
        Self::Actions,
        Self::Commands,
        Self::Web,
        Self::Menu,
        Self::Integration,
        Self::About,
    ];

    pub(super) fn label(self, strings: &Strings) -> &'static str {
        match self {
            Self::General => strings.section_general,
            Self::Appearance => strings.section_appearance,
            Self::Launcher => strings.section_launcher,
            Self::Hotkeys => strings.section_shortcuts,
            Self::Filters => strings.section_filters,
            Self::Index => strings.index,
            Self::Actions => strings.actions_status,
            Self::Commands => strings.section_commands,
            Self::Web => strings.section_web,
            Self::Menu => strings.section_menu,
            Self::Integration => strings.section_integration,
            Self::About => strings.section_about,
        }
    }

    fn fields(self) -> &'static [Field] {
        match self {
            Self::Launcher => &[Field::ExcludedTypes],
            Self::Integration => &[Field::DisabledApps, Field::OpenerPath, Field::OpenerArguments, Field::ApiPort],
            Self::General
            | Self::Appearance
            | Self::Hotkeys
            | Self::Filters
            | Self::Index
            | Self::Actions
            | Self::Commands
            | Self::Web
            | Self::Menu
            | Self::About => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    Theme,
    Zoom,
    Language,
    Opening,
    DefaultWeb,
    Repeat,
    Rewalk,
    Scope,
    Uninstaller,
    Date,
}

impl MenuId for Menu {
    fn id(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Zoom => "zoom",
            Self::Language => "language",
            Self::Opening => "opening",
            Self::DefaultWeb => "default-web",
            Self::Repeat => "repeat",
            Self::Rewalk => "rewalk",
            Self::Scope => "scope",
            Self::Uninstaller => "uninstaller",
            Self::Date => "date",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Folds {
    Chosen,
    All,
}

pub(super) struct FiltersRenumbered(pub(super) Renumber);

pub(super) struct SettingsPage {
    section: Section,
    focus: FocusHandle,
    finder: Entity<TextInput>,
    found: Option<Finding>,
    menus: Menus<Menu>,
    settings_scroll: ScrollHandle,
    form: Form,
    theme_editor: Entity<ThemeEditor>,
    diagnostics: Diagnostics,
    resetting: Option<Task<()>>,
    enabling_full_text: Option<Task<()>>,
    deleting_content: Option<Task<()>>,
    speed: Option<Duration>,
    index: Arc<Index>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FiltersRenumbered> for SettingsPage {}

impl Focusable for SettingsPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        self.page(theme, language, window, cx)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &SelectNext, window, cx| this.step_match(Step::Next, window, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, window, cx| this.step_match(Step::Previous, window, cx)))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
    }
}

impl SettingsPage {
    pub(super) fn new(index: Arc<Index>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let finder = cx.new(TextInput::new);
        let mut subscriptions = vec![
            ui::paint_changes(cx),
            ui::paint_changes_of(&finder, cx),
            cx.subscribe_in(&finder, window, |this, _, _: &Changed, window, cx| this.find(window, cx)),
            cx.observe_global::<Status>(|this, cx| this.status_changed(cx)),
            cx.observe_global::<Updates>(|_, cx| cx.notify()),
        ];
        let theme_editor = cx.new(ThemeEditor::new);
        subscriptions.push(cx.observe(&theme_editor, |_, _, cx| cx.notify()));
        let (diagnostics, diagnosing) = Diagnostics::new(cx);
        subscriptions.extend(diagnosing);
        Self {
            section: Section::General,
            focus: cx.focus_handle(),
            finder,
            found: None,
            menus: Menus::default(),
            settings_scroll: ScrollHandle::new(),
            form: Form::default(),
            theme_editor,
            diagnostics,
            resetting: None,
            enabling_full_text: None,
            deleting_content: None,
            speed: None,
            index,
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn section(&self) -> Section {
        self.section
    }

    fn status_changed(&mut self, cx: &mut Context<Self>) {
        let waiting =
            cx.global::<super::Machine>().content_estimate.is_none() && !cx.global::<Settings>().index.full_text;
        if self.section == Section::Index && waiting {
            load_estimate(self.index.clone(), cx);
        }
    }

    pub(super) fn found(&self, section: Section) -> usize {
        self.found.as_ref().map_or(0, |found| found.count(section))
    }

    pub(super) fn open(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menus(cx);
        match section {
            Section::General => refresh_autostart(cx),
            Section::Appearance => reload_themes(cx),
            Section::Index => {
                load_drives(cx);
                load_estimate(self.index.clone(), cx);
            }
            Section::Launcher
            | Section::Hotkeys
            | Section::Filters
            | Section::Actions
            | Section::Commands
            | Section::Web
            | Section::Menu
            | Section::Integration
            | Section::About => {}
        }
        self.open_section(section, cx);
        self.land(section, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close_menus(&mut self, cx: &mut Context<Self>) {
        self.menus.close();
        self.theme_editor.update(cx, |editor, cx| editor.close_menus(cx));
    }

    pub(super) fn leave(&mut self, cx: &mut Context<Self>) {
        self.close_menus(cx);
        self.stop_recording(cx);
    }

    pub(super) fn activated(&mut self, active: bool, cx: &mut Context<Self>) {
        self.finder.update(cx, |input, cx| input.set_blinking(active, cx));
        self.set_fields_blinking(active, cx);
        if !active {
            self.stop_recording(cx);
        } else if self.section == Section::Appearance {
            reload_themes(cx);
        }
    }

    pub(super) fn restyle(&mut self, theme: &Theme, cx: &mut Context<Self>) {
        let under = theme.app.background;
        self.finder.update(cx, |input, cx| input.set_under(under, cx));
        self.diagnostics.input().update(cx, |input, cx| input.set_under(under, cx));
        self.theme_editor.update(cx, |editor, cx| {
            editor.restyle(theme, cx);
            editor.sync(cx);
        });
        cx.notify();
    }

    pub(super) fn sync_theme_editor(&mut self, cx: &mut Context<Self>) {
        self.theme_editor.update(cx, |editor, cx| editor.sync(cx));
    }

    pub(super) fn apply_placeholder(&mut self, window: &Window, cx: &mut Context<Self>) {
        let strings = cx.global::<Language>().strings();
        self.theme_editor.update(cx, |editor, cx| editor.set_placeholders(strings, cx));
        self.diagnostics.input().update(cx, |input, _| input.set_placeholder(strings.diagnose_example));
        self.finder.update(cx, |input, cx| {
            input.set_placeholder(strings.settings_search);
            cx.notify();
        });
        self.refind(window, cx);
    }

    fn picker<T>(&self, menu: Menu, choices: Vec<Choice<T>>, current: T, theme: &Theme) -> Picker<T> {
        self.menus.picker(menu, choices, current, theme)
    }

    fn dropdown<T: Clone + PartialEq + 'static>(
        &self,
        menu: Menu,
        picker: Picker<T>,
        apply: impl Fn(T, &mut App) + 'static,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        dropdown(cx.weak_entity(), |this: &mut Self| &mut this.menus, menu, picker, apply, theme, window)
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.theme_editor.update(cx, |editor, cx| editor.confirm(window, cx));
        if !editing && !self.confirm_diagnosis(window, cx) {
            self.step_match(Step::Next, window, cx);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.menus.close() {
            cx.notify();
        } else if self.theme_editor.update(cx, |editor, cx| editor.dismiss(cx)) || self.close_editor(cx) {
        } else if !self.finder.read(cx).text().is_empty() {
            self.finder.update(cx, |input, cx| input.set_text("", cx));
        } else if self.finder.focus_handle(cx).is_focused(window) {
            window.focus(&self.focus, cx);
        } else {
            cx.propagate();
        }
    }

    fn blocks(
        &self,
        section: Section,
        folds: Folds,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let strings = language.strings();
        match section {
            Section::General => self.general(theme, language, window, cx),
            Section::Appearance => self.appearance(folds, theme, language, window, cx),
            Section::Launcher => self.launcher_section(theme, strings, window, cx),
            Section::Hotkeys => self.hotkeys(theme, strings, window, cx),
            Section::Filters => self.filters(theme, strings, window, cx),
            Section::Index => {
                let mut blocks = self.index_section(theme, language, window, cx);
                blocks.push(self.diagnosis_block(theme, language, window, cx));
                blocks
            }
            Section::Actions => self.actions(theme, strings, window, cx),
            Section::Commands => self.commands(theme, strings, window, cx),
            Section::Web => self.web(theme, strings, window, cx),
            Section::Menu => self.favorites(theme, strings, window, cx),
            Section::Integration => self.integration(theme, language, window, cx),
            Section::About => self.about(theme, language, cx),
        }
    }

    fn page(&self, theme: &Theme, language: Language, window: &Window, cx: &Context<Self>) -> Div {
        let section = self.section;
        let blocks = self.blocks(section, Folds::Chosen, theme, language, window, cx);
        let found = self.found.as_ref();
        let editor = self.theme_editor.read(cx);
        let on_screen = found.and_then(|found| found.on_screen(section, |needs| editor.shows_all(needs)));
        let label = section.label(language.strings());
        let laid = lay_out(label, blocks, found.map(|found| &found.needle), on_screen, theme);
        let content = div()
            .id(("settings", section as usize))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.settings_scroll)
            .px(rem(GUTTER))
            .pb(rem(INSET))
            .flex()
            .flex_col()
            .items_center()
            .children(laid.lines);
        let title = div().flex().justify_center().child(column().child(laid.title));
        let jump = found.map_or(0, |found| found.jump);
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(top_bar(title, theme, window))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(glide("settings", self.settings_scroll.clone(), None))
                    .when(self.form.lifted.is_some(), |area| {
                        area.child(edge_scroll("settings-drag", self.settings_scroll.clone()))
                    })
                    .children(self.release_on_mouse_up(cx))
                    .child(scroll_to_child(REVEAL_ID, self.settings_scroll.clone(), laid.target, jump))
                    .child(content)
                    .child(edge_fades(self.settings_scroll.clone(), theme.app.background, GUTTER))
                    .child(divider(Edge::Top, theme.app.divider)),
            )
            .child(self.finder_bar(theme, language, window, cx))
    }
}

fn confirmed<V: 'static>(this: &mut V, slot: fn(&mut V) -> &mut Option<Task<()>>, cx: &mut Context<V>) -> bool {
    if slot(this).take().is_some() {
        return true;
    }
    *slot(this) = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(CONFIRMING).await;
        let _ = this.update(cx, |this, cx| {
            *slot(this) = None;
            cx.notify();
        });
    }));
    cx.notify();
    false
}

pub(super) fn change(cx: &mut App, edit: impl FnOnce(&mut Settings)) {
    cx.update_global::<Settings, _>(|settings, _| edit(settings));
}

fn column() -> Div {
    div().w_full().max_w(rem(SETTINGS_WIDTH)).flex().flex_col()
}

type PathStatus = (Option<SharedString>, Option<AnyElement>);

fn path_rows(
    id: &'static str,
    paths: &[String],
    mut status: impl FnMut(usize, &str) -> PathStatus,
    remove: impl Fn(&mut Settings, usize) + Clone + 'static,
    theme: &Theme,
) -> Vec<Row> {
    paths
        .iter()
        .enumerate()
        .map(|(position, path)| {
            let tip = Tip::new(path.clone());
            let (hint, control) = status(position, path);
            let hint = hint.map(|hint| {
                div()
                    .text_size(rem(DETAIL_SIZE))
                    .line_height(rem(DETAIL_LINE))
                    .text_color(theme.settings.hint)
                    .child(hint)
            });
            let label = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .text_color(theme.settings.label)
                .child(path_label((id, position), SharedString::from(path.clone()), Vec::new(), tip, NAME_LINE))
                .children(hint);
            let button = icon_button(
                ElementId::NamedInteger(SharedString::from(format!("{id}-remove")), position as u64),
                assets::CLOSE,
                theme,
            )
            .on_click({
                let remove = remove.clone();
                move |_, _, cx| change(cx, |settings| remove(settings, position))
            });
            Row::plain(
                div()
                    .relative()
                    .w_full()
                    .min_h(rem(FIELD_ROW))
                    .py(rem(FIELD_PADDING))
                    .flex()
                    .items_center()
                    .gap(rem(ROW_GAP))
                    .child(label)
                    .children(control)
                    .child(button),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_lands_on_whole_pixels_at_common_scales() {
        crate::ui::assert_whole_pixels(&[
            SECTION_TITLE,
            ROW,
            ROW_PADDING,
            ROW_GAP,
            FIELD_ROW,
            FIELD_PADDING,
            FOLD_END,
            SETTINGS_WIDTH,
        ]);
    }
}
