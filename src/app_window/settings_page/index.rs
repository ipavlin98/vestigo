use super::super::Machine;
use super::SettingsPage;
use super::{Block, Heading, Menu, PathStatus, Row, change, confirmed, path_rows, section, setting};
use crate::i18n::{Language, Strings};
use crate::index::{ContentEstimate, ContentStatus, FolderStatus, Index, Status, Unread};
use crate::platform;
use crate::settings::{IndexOptions, Rewalk, Settings};
use crate::theme::Theme;
use crate::ui::{Choice, button, toggle};
use gpui::{App, AppContext, BorrowAppContext, Context, ElementId, SharedString, Window, prelude::*};
use std::sync::Arc;

struct Folders {
    id: &'static str,
    title: &'static str,
    hint: &'static str,
    list: fn(&mut IndexOptions) -> &mut Vec<String>,
}

pub(in crate::app_window) fn load_drives(cx: &mut App) {
    cx.spawn(async move |cx| {
        let drives: Vec<char> = cx
            .background_spawn(async move { platform::fixed_drives(false) })
            .await
            .into_iter()
            .map(|drive| drive.letter)
            .collect();
        cx.update(|cx| {
            if cx.global::<Machine>().drives != drives {
                cx.update_global::<Machine, _>(|machine, _| machine.drives = drives);
            }
        });
    })
    .detach();
}

pub(super) fn load_estimate(index: Arc<Index>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let estimate = cx.background_spawn(async move { index.content_estimate() }).await;
        cx.update(|cx| {
            if cx.global::<Machine>().content_estimate != estimate {
                cx.update_global::<Machine, _>(|machine, _| machine.content_estimate = estimate);
            }
        });
    })
    .detach();
}

impl SettingsPage {
    pub(super) fn index_section(
        &self,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let strings = language.strings();
        let settings = cx.global::<Settings>();
        let indexed = self.index.drives();
        let drives = cx.global::<Machine>().drives.clone();
        let drives = drives.into_iter().map(|letter| {
            let on = !settings.index.skipped_drives.contains(&letter);
            let hint = match indexed.iter().find(|drive| drive.letter == letter) {
                Some(drive) => language.items(drive.entries),
                None if on => strings.indexing.to_owned(),
                None => strings.drive_off.to_owned(),
            };
            let control = toggle(("drive", letter as usize), on, theme).on_click(move |_, _, cx| {
                change(cx, |settings| {
                    let skipped = &mut settings.index.skipped_drives;
                    match skipped.iter().position(|&known| known == letter) {
                        Some(position) => drop(skipped.remove(position)),
                        None => skipped.push(letter),
                    }
                })
            });
            super::setting_with(format!("{letter}:"), vec![SharedString::from(hint)], control)
        });
        let hidden_low = toggle("hidden-low", settings.index.hidden_low, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.index.hidden_low = !settings.index.hidden_low));
        let rebuild =
            button("rebuild", strings.rebuild, true, theme, |_, _, _| platform::send(platform::Event::Rebuild));
        let extra = Folders {
            id: "index-folder",
            title: strings.index_folders,
            hint: strings.index_folders_hint,
            list: |index| &mut index.folders,
        };
        let lists = [
            Folders {
                id: "excluded-folder",
                title: strings.excluded_folders,
                hint: strings.excluded_folders_hint,
                list: |index| &mut index.excluded,
            },
            Folders {
                id: "high-folder",
                title: strings.high_folders,
                hint: strings.high_folders_hint,
                list: |index| &mut index.high,
            },
            Folders {
                id: "low-folder",
                title: strings.low_folders,
                hint: strings.low_folders_hint,
                list: |index| &mut index.low,
            },
        ];
        let mut extra_rows =
            self.folder_list(&extra, settings, self.folder_status(language, theme), strings, theme, cx);
        extra_rows.push(self.rewalk(settings, strings, theme, window, cx));
        let mut sections = vec![
            section(Heading::Title(strings.drives), drives.collect()),
            section(Heading::Title(extra.title), extra_rows),
            self.content_block(settings, theme, language, cx),
        ];
        for folders in lists {
            let rows = self.folder_list(&folders, settings, |_, _| (None, None), strings, theme, cx);
            sections.push(section(Heading::Title(folders.title), rows));
        }
        sections.push(section(
            Heading::Title(strings.section_system),
            vec![
                setting(strings.hidden_low, strings.hidden_low_hint, hidden_low),
                setting(strings.rebuild_index, strings.rebuild_hint, rebuild),
            ],
        ));
        sections
    }

    fn content_block(&self, settings: &Settings, theme: &Theme, language: Language, cx: &Context<Self>) -> Block {
        let strings = language.strings();
        let status = *cx.global::<Status>();
        let on = settings.index.full_text;
        let (label, hint) = match (on, status.content) {
            (false, _) => {
                let label = match self.enabling_full_text {
                    Some(_) => strings.full_text_enable_confirm,
                    None => strings.full_text_enable,
                };
                let estimate = cx.global::<Machine>().content_estimate;
                let hint = estimate.map(|ContentEstimate { files, text, cache, seconds }| {
                    language.content_estimate(files, text, cache, seconds)
                });
                (label, hint.unwrap_or_else(|| strings.full_text_counting.to_owned()))
            }
            (true, ContentStatus::Building { done, total, left }) if total > 0 => {
                (strings.full_text_disable, language.content_building(done, total, left))
            }
            (true, ContentStatus::Ready { files, built, .. }) => {
                (strings.full_text_disable, language.content_ready(files, status.content_cache, built))
            }
            (true, ContentStatus::Building { .. } | ContentStatus::Off) => {
                (strings.full_text_disable, strings.indexing.to_owned())
            }
        };
        let switch =
            button("full-text", label, true, theme, cx.listener(move |this, _, _, cx| this.switch_full_text(on, cx)));
        let unread = match status.content {
            ContentStatus::Ready { unread, .. } if on => unread_hint(unread, language),
            ContentStatus::Off | ContentStatus::Building { .. } | ContentStatus::Ready { .. } => {
                SharedString::default()
            }
        };
        let cache = status.content_cache;
        let cache_hint = match (cache, on) {
            (0, _) => strings.content_cache_none.to_owned(),
            (_, true) => language.content_cache_kept(cache),
            (_, false) => language.content_cache_idle(cache),
        };
        let delete_label = if self.deleting_content.is_some() { strings.delete_confirm } else { strings.delete };
        let delete = button(
            "content-cache",
            delete_label,
            cache > 0,
            theme,
            cx.listener(|this, _, _, cx| this.delete_content(cx)),
        );
        section(
            Heading::Title(strings.section_content),
            vec![
                super::setting_with(
                    strings.full_text,
                    vec![SharedString::from(hint), unread, strings.full_text_usage.into()],
                    switch,
                ),
                super::setting_with(strings.content_cache, vec![SharedString::from(cache_hint)], delete),
            ],
        )
    }

    fn switch_full_text(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on && !confirmed(self, |this| &mut this.enabling_full_text, cx) {
            return;
        }
        change(cx, |settings| settings.index.full_text = !on);
    }

    fn delete_content(&mut self, cx: &mut Context<Self>) {
        if confirmed(self, |this| &mut this.deleting_content, cx) {
            platform::send(platform::Event::DeleteContentCache);
        }
    }

    fn folder_list(
        &self,
        folders: &Folders,
        settings: &Settings,
        status: impl FnMut(usize, &str) -> PathStatus,
        strings: &Strings,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> Vec<Row> {
        let list = folders.list;
        let mut options = settings.index.clone();
        let mut rows = path_rows(folders.id, list(&mut options), status, remover(list), theme);
        let add = button(
            (folders.id, usize::MAX),
            strings.add,
            true,
            theme,
            cx.listener(move |this, _, _, cx| {
                this.pick_folder_into(list, cx);
            }),
        );
        rows.push(setting(strings.add_folder, folders.hint, add));
        rows
    }

    fn folder_status(&self, language: Language, theme: &Theme) -> impl FnMut(usize, &str) -> PathStatus {
        let strings = language.strings();
        let statuses = self.index.folders();
        let index = self.index.clone();
        move |position, path| {
            let status = statuses.iter().find(|status| status.path == path);
            let hint = match status {
                Some(FolderStatus { walking: true, .. }) => strings.indexing.to_owned(),
                Some(FolderStatus { entries, walked: Some(walked), .. }) => match platform::local_time(*walked) {
                    Some(stamp) => language.walked(*entries, stamp),
                    None => language.items(*entries),
                },
                Some(FolderStatus { entries, walked: None, .. }) => language.items(*entries),
                None => strings.folder_in_drive.to_owned(),
            };
            let refresh = status.map(|status| {
                let index = index.clone();
                let path = status.path.clone();
                let id = ElementId::NamedInteger("index-folder-refresh".into(), position as u64);
                button(id, strings.refresh_folder, !status.walking, theme, move |_, _, _| index.refresh_folder(&path))
                    .into_any_element()
            });
            (Some(SharedString::from(hint)), refresh)
        }
    }

    fn rewalk(
        &self,
        settings: &Settings,
        strings: &Strings,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Row {
        let choices = Rewalk::ALL.map(|rewalk| Choice {
            value: rewalk,
            label: match rewalk {
                Rewalk::Never => strings.rewalk_never,
                Rewalk::QuarterHour => strings.rewalk_quarter_hour,
                Rewalk::HalfHour => strings.rewalk_half_hour,
                Rewalk::Hour => strings.rewalk_hour,
                Rewalk::SixHours => strings.rewalk_six_hours,
                Rewalk::Day => strings.rewalk_day,
            }
            .into(),
            note: None,
            swatch: None,
        });
        let picker = self.picker(Menu::Rewalk, choices.into(), settings.index.rewalk, theme);
        let rewalk = self.dropdown(
            Menu::Rewalk,
            picker,
            |rewalk, cx| change(cx, |settings| settings.index.rewalk = rewalk),
            theme,
            window,
            cx,
        );
        setting(strings.rewalk, strings.rewalk_hint, rewalk)
    }

    fn pick_folder_into(&mut self, list: fn(&mut IndexOptions) -> &mut Vec<String>, cx: &mut Context<Self>) {
        self.pick_folder(
            move |settings, path| {
                let folders = list(&mut settings.index);
                if !folders.iter().any(|known| known.eq_ignore_ascii_case(&path)) {
                    folders.push(path);
                }
            },
            cx,
        );
    }
}

fn unread_hint(unread: Unread, language: Language) -> SharedString {
    let strings = language.strings();
    let Unread { denied, empty, binary, encrypted, failed } = unread;
    let reasons = [
        (denied, strings.unread_denied),
        (empty, strings.unread_empty),
        (binary, strings.unread_binary),
        (encrypted, strings.unread_encrypted),
        (failed, strings.unread_failed),
    ];
    let reasons: Vec<(usize, &str)> = reasons.into_iter().filter(|(count, _)| *count > 0).collect();
    language.content_unread(&reasons).into()
}

fn remover(list: fn(&mut IndexOptions) -> &mut Vec<String>) -> impl Fn(&mut Settings, usize) + Clone + 'static {
    move |settings, position| {
        let folders = list(&mut settings.index);
        if position < folders.len() {
            folders.remove(position);
        }
    }
}
