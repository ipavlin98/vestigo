use super::super::search_page::RESULT_LIMIT;
use super::super::{Machine, set_autostart};
use super::SettingsPage;
use super::form::Field;
use super::theme_editor::cluster;
use super::{Block, Folds, Heading, Menu, ROW_PADDING, Row, change, confirmed, section, setting, setting_with};
use crate::i18n::{Language, LanguageChoice, Strings};
use crate::index::{Focus, Status};
use crate::platform;
use crate::search::Query;
use crate::settings::{self, History, Settings, Triggers, Zoom};
use crate::theme::rem;
use crate::theme::{Appearance, Catalog, Entry, Origin, Theme, ThemeChoice};
use crate::ui::{self, Choice, Command, button, toggle};
use gpui::{AnyElement, AppContext, BorrowAppContext, Context, Hsla, Rgba, SharedString, Window, div, prelude::*};

const SPEED_QUERIES: [&str; 4] = ["a", "report", "setup exe", "*.dll"];
const ACTIVITY_DAYS: usize = 26 * 7;
const AVERAGE_DAYS: usize = 30;
const DAY: i64 = 24 * 60 * 60;
const WEEK: i64 = 7;
const EPOCH_WEEKDAY: i64 = 3;
const LEVELS: f32 = 4.0;
const CELL: f32 = 12.0;
const CELL_GAP: f32 = 4.0;

impl SettingsPage {
    pub(super) fn general(&self, theme: &Theme, language: Language, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let strings = language.strings();
        let current = cx.global::<Settings>().language;
        let languages = LanguageChoice::ALL.map(|choice| Choice {
            value: choice,
            label: choice.label(strings).into(),
            note: None,
            swatch: None,
        });
        let languages = self.picker(Menu::Language, languages.into(), current, theme);
        let languages = self.dropdown(
            Menu::Language,
            languages,
            |choice, cx| change(cx, |settings| settings.language = choice),
            theme,
            window,
            cx,
        );
        let settings = cx.global::<Settings>();
        let history_empty = cx.global::<History>().is_empty();
        let autostart = cx.global::<Machine>().autostart;
        let autostart_toggle = toggle("autostart", autostart == Some(true), theme).on_click(move |_, _, cx| {
            if let Some(on) = autostart {
                set_autostart(!on, cx);
            }
        });
        let tray = toggle("tray", settings.tray, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.tray = !settings.tray));
        let clear = button("history", strings.clear, !history_empty, theme, |_, _, cx| {
            cx.update_global::<History, _>(|history, _| history.clear())
        });
        let reset = button("position", strings.reset, settings.position.is_some(), theme, |_, _, cx| {
            change(cx, |settings| settings.position = None)
        });
        let everything = if self.resetting.is_some() { strings.reset_confirm } else { strings.reset };
        let everything = button(
            "settings-reset",
            everything,
            *settings != Settings::default(),
            theme,
            cx.listener(|this, _, _, cx| this.reset_settings(cx)),
        );
        vec![section(
            Heading::Plain,
            vec![
                setting_with(strings.language, Vec::new(), languages),
                setting(strings.autostart, strings.autostart_hint, autostart_toggle),
                setting(strings.tray, strings.tray_hint, tray),
                setting(strings.history, strings.history_hint, clear),
                setting(strings.position, strings.position_hint, reset),
                setting(strings.reset_settings, strings.reset_settings_hint, everything),
            ],
        )]
    }

    fn measure_speed(&mut self, cx: &mut Context<Self>) {
        let index = self.index.clone();
        let grammar = ui::grammar(cx);
        cx.spawn(async move |this, cx| {
            let speed = cx
                .background_spawn(async move {
                    let queries: Vec<Query> =
                        SPEED_QUERIES.iter().filter_map(|text| Query::parse(text, &grammar)).collect();
                    let started = std::time::Instant::now();
                    for query in &queries {
                        index.search(query, &Default::default(), &Focus::Programs, RESULT_LIMIT);
                    }
                    started.elapsed() / queries.len().max(1) as u32
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.speed = Some(speed);
                cx.notify();
            });
        })
        .detach();
    }

    fn reset_settings(&mut self, cx: &mut Context<Self>) {
        if !confirmed(self, |this| &mut this.resetting, cx) {
            return;
        }
        self.close_menus(cx);
        self.close_editor(cx);
        change(cx, |settings| *settings = Settings::default());
        self.rebuild_fields(cx);
    }

    pub(super) fn appearance(
        &self,
        folds: Folds,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let strings = language.strings();
        let settings = cx.global::<Settings>();
        let catalog = cx.global::<Catalog>();
        let themes = std::iter::once(Choice {
            value: ThemeChoice::System,
            label: strings.follow_system.into(),
            note: None,
            swatch: None,
        })
        .chain(catalog.entries().iter().map(|entry| Choice {
            value: ThemeChoice::Named(entry.id.clone()),
            label: theme_label(entry, strings),
            note: None,
            swatch: None,
        }))
        .collect();
        let shown = catalog.current(&settings.theme, Appearance::of(cx.window_appearance()));
        let title = shown.map(|entry| theme_label(entry, strings)).unwrap_or_default();
        let deletable = shown.filter(|entry| entry.deletable()).map(|entry| entry.id.clone());
        let theme_hints = std::iter::once(SharedString::from(strings.themes_folder_hint))
            .chain(catalog.failures().iter().map(|failure| SharedString::from(failure.clone())))
            .collect();
        let chosen = match &settings.theme {
            ThemeChoice::Named(id) if catalog.entries().iter().any(|entry| &entry.id == id) => settings.theme.clone(),
            ThemeChoice::Named(_) | ThemeChoice::System => ThemeChoice::System,
        };
        let themes = self.picker(Menu::Theme, themes, chosen, theme);
        let themes = self.dropdown(
            Menu::Theme,
            themes,
            |choice, cx| change(cx, |settings| settings.theme = choice),
            theme,
            window,
            cx,
        );
        let editor = self.theme_editor.read(cx);
        let delete = deletable.map(|id| {
            let label = if editor.deleting() { strings.delete_confirm } else { strings.delete };
            button(
                "theme-delete",
                label,
                true,
                theme,
                cx.listener(move |this, _, _, cx| {
                    this.theme_editor.update(cx, |editor, cx| editor.delete_theme(id.clone(), cx));
                }),
            )
            .into_any_element()
        });
        let themes = cluster([delete, Some(themes.into_any_element())]);
        let theme_outcome = editor.delete_failed().then(|| SharedString::from(strings.theme_delete_failed));
        let zooms = Zoom::ALL.map(|zoom| Choice {
            value: zoom,
            label: format!("{}%", zoom.percent()).into(),
            note: None,
            swatch: None,
        });
        let zooms = self.picker(Menu::Zoom, zooms.into(), settings.zoom, theme);
        let zooms =
            self.dropdown(Menu::Zoom, zooms, |zoom, cx| change(cx, |settings| settings.zoom = zoom), theme, window, cx);
        let open_themes = button(
            "themes",
            strings.open,
            true,
            theme,
            cx.listener(|_, _, _, cx| {
                let dir = settings::themes_dir();
                if let Err(error) = std::fs::create_dir_all(&dir) {
                    log::warn!("Couldn't create {}: {error}", dir.display());
                }
                ui::perform(Command::Open(dir.to_string_lossy().into_owned()), cx);
            }),
        );
        let editing = editor.section(&title, folds, theme, language, window, cx);
        let rows = vec![
            setting_with(strings.theme, theme_outcome.into_iter().collect(), themes),
            setting_with(strings.themes_folder, theme_hints, open_themes),
            setting(strings.zoom, strings.zoom_hint, zooms),
        ];
        std::iter::once(section(Heading::Plain, rows)).chain(editing).collect()
    }

    pub(super) fn integration(
        &self,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let strings = language.strings();
        let settings = cx.global::<Settings>();
        let trigger = |id: &'static str, label, hint, pick: fn(&mut Triggers) -> &mut bool| {
            let mut triggers = settings.triggers;
            let on = *pick(&mut triggers);
            let control = toggle(id, on, theme).on_click(move |_, _, cx| {
                change(cx, |settings| {
                    let flag = pick(&mut settings.triggers);
                    *flag = !*flag;
                })
            });
            setting(label, hint, control)
        };
        let open_dialogs = toggle("open-dialogs", settings.suggestions.open_dialogs, theme).on_click(|_, _, cx| {
            change(cx, |settings| settings.suggestions.open_dialogs = !settings.suggestions.open_dialogs)
        });
        let crumbs = toggle("path-crumbs", settings.path_crumbs, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.path_crumbs = !settings.path_crumbs));
        let api = toggle("api", settings.api.enabled, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.api.enabled = !settings.api.enabled));
        let api_hint = language.api_hint(settings.api.port);
        let field = |field: Field, hint: &'static str| -> Option<Row> {
            let control = self.field(field, theme, window, cx)?;
            Some(setting(field.label(strings), hint, control))
        };
        let programs = [
            field(Field::DisabledApps, strings.disabled_apps_hint),
            field(Field::OpenerPath, strings.opener_hint),
            field(Field::OpenerArguments, strings.opener_arguments_hint),
        ];
        let port = field(Field::ApiPort, strings.api_port_hint);
        vec![
            section(
                Heading::Plain,
                vec![
                    trigger("explorer", strings.explorer, strings.explorer_hint, |triggers| &mut triggers.explorer),
                    trigger("desktop", strings.desktop, strings.desktop_hint, |triggers| &mut triggers.desktop),
                    trigger("dialogs", strings.dialogs, strings.dialogs_hint, |triggers| &mut triggers.dialogs),
                    setting(strings.open_dialog_suggestions, strings.open_dialog_suggestions_hint, open_dialogs),
                    setting(strings.path_crumbs, strings.path_crumbs_hint, crumbs),
                ],
            ),
            section(Heading::Title(strings.section_system), programs.into_iter().flatten().collect()),
            section(
                Heading::Title(strings.automation),
                std::iter::once(setting_with(strings.api, vec![api_hint.into()], api)).chain(port).collect(),
            ),
        ]
    }

    pub(super) fn about(&self, theme: &Theme, language: Language, cx: &Context<Self>) -> Vec<Block> {
        let strings = language.strings();
        let status = *cx.global::<Status>();
        let version = SharedString::from(env!("CARGO_PKG_VERSION"));
        let value = |text: SharedString| div().flex_none().text_color(theme.settings.value).child(text);
        let speed = self.speed.map(|speed| language.milliseconds(speed)).unwrap_or_default();
        let measure = button(
            "measure",
            strings.measure,
            !status.scanning,
            theme,
            cx.listener(|this, _, _, cx| this.measure_speed(cx)),
        );
        let clock = platform::clock();
        let days = cx.global::<History>().usage().days(clock.now, clock.offset, ACTIVITY_DAYS);
        let month: u32 = days.iter().rev().take(AVERAGE_DAYS).sum();
        let average = language.decimal(f64::from(month) / AVERAGE_DAYS as f64);
        let today = (clock.now + clock.offset).div_euclid(DAY);
        vec![
            section(
                Heading::Plain,
                vec![
                    setting_with(strings.version, Vec::new(), value(version)),
                    setting_with(strings.indexed, Vec::new(), value(language.items(status.entries).into())),
                    setting_with(strings.search_speed, vec![speed.into()], measure),
                    setting(strings.opens_per_day, strings.opens_per_day_hint, value(average.into())),
                ],
            ),
            section(Heading::Title(strings.activity), vec![Row::plain(activity(&days, today, theme))]),
        ]
    }
}

fn activity(days: &[u32], today: i64, theme: &Theme) -> AnyElement {
    let colors = theme.activity;
    let busiest = days.iter().copied().max().unwrap_or_default().max(1);
    let weekday = (today + EPOCH_WEEKDAY).rem_euclid(WEEK) as usize;
    let padding = WEEK as usize - 1 - weekday;
    let cells: Vec<Option<u32>> = days.iter().copied().map(Some).chain(std::iter::repeat_n(None, padding)).collect();
    let skipped = cells.len() % WEEK as usize;
    let weeks = cells.get(skipped..).unwrap_or_default().chunks(WEEK as usize).map(|week| {
        div().flex().flex_col().gap(rem(CELL_GAP)).children(week.iter().map(|day| {
            let cell = div().size(rem(CELL)).rounded(rem(theme.radius.small));
            match day {
                Some(0) => cell.bg(colors.empty),
                Some(opens) => cell.bg(mix(colors.empty, colors.full, level(*opens, busiest))),
                None => cell,
            }
        }))
    });
    div().py(rem(ROW_PADDING)).flex().gap(rem(CELL_GAP)).children(weeks).into_any_element()
}

fn level(opens: u32, busiest: u32) -> f32 {
    let share = opens as f32 / busiest as f32;
    (share * LEVELS).ceil() / LEVELS
}

fn mix(from: Hsla, to: Hsla, share: f32) -> Hsla {
    let (from, to) = (from.to_rgb(), to.to_rgb());
    let blend = |from: f32, to: f32| from + (to - from) * share;
    Hsla::from(Rgba { r: blend(from.r, to.r), g: blend(from.g, to.g), b: blend(from.b, to.b), a: 1.0 })
}

fn theme_label(entry: &Entry, strings: &Strings) -> SharedString {
    match entry.origin {
        Origin::System(Appearance::Light) => strings.theme_light.into(),
        Origin::System(Appearance::Dark) => strings.theme_dark.into(),
        Origin::Example | Origin::File => entry.name.clone().into(),
    }
}
