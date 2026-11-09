use super::theme_editor::cluster;
use super::{Block, Heading, SettingsPage, section, setting_row};
use crate::i18n::Language;
use crate::index::Diagnosis;
use crate::input::{Changed, TextInput};
use crate::settings::Settings;
use crate::theme::Theme;
use crate::ui::{self, FIELD_WIDTH, button, text_field};
use gpui::{AppContext, Context, Entity, Focusable, SharedString, Subscription, Task, Window, prelude::*};
use std::path::Path;

pub(in crate::app_window) struct Diagnostics {
    input: Entity<TextInput>,
    outcome: Option<(String, Diagnosis)>,
    task: Option<Task<()>>,
}

impl Diagnostics {
    pub(in crate::app_window) fn new(cx: &mut Context<SettingsPage>) -> (Self, Vec<Subscription>) {
        let input = cx.new(TextInput::field);
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, _: &Changed, cx| {
                this.diagnostics.outcome = None;
                this.diagnostics.task = None;
                cx.notify();
            }),
            ui::paint_changes_of(&input, cx),
        ];
        (Self { input, outcome: None, task: None }, subscriptions)
    }

    pub(in crate::app_window) fn input(&self) -> &Entity<TextInput> {
        &self.input
    }
}

impl SettingsPage {
    pub(super) fn diagnosis_block(
        &self,
        theme: &Theme,
        language: Language,
        window: &Window,
        cx: &Context<Self>,
    ) -> Block {
        let strings = language.strings();
        let input = self.diagnostics.input.clone();
        let typed = !input.read(cx).text().trim().is_empty();
        let focused = input.focus_handle(cx).is_focused(window);
        let check = button("diagnose", strings.check, typed, theme, cx.listener(|this, _, _, cx| this.diagnose(cx)));
        let controls = cluster([
            Some(text_field(input, focused, FIELD_WIDTH, theme).into_any_element()),
            Some(check.into_any_element()),
        ]);
        let mut rows = vec![setting_row(strings.diagnose, vec![strings.diagnose_hint.into()], vec![controls])];
        if let Some((path, diagnosis)) = &self.diagnostics.outcome {
            let (verdict, fixes) = explain(path, diagnosis, cx.global::<Settings>(), language);
            rows.push(setting_row(verdict, fixes, Vec::new()));
        }
        section(Heading::Title(strings.diagnosis), rows)
    }

    pub(super) fn confirm_diagnosis(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let focused = self.diagnostics.input.focus_handle(cx).is_focused(window);
        if focused {
            self.diagnose(cx);
        }
        focused
    }

    fn diagnose(&mut self, cx: &mut Context<Self>) {
        let path = self.diagnostics.input.read(cx).text().trim().trim_matches('"').to_owned();
        if path.is_empty() {
            return;
        }
        let index = self.index.clone();
        self.diagnostics.task = Some(cx.spawn(async move |this, cx| {
            let checked = path.clone();
            let diagnosis = cx
                .background_spawn(async move {
                    let exists = std::fs::symlink_metadata(&checked).is_ok();
                    index.diagnose(&checked, exists)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.diagnostics.outcome = Some((path, diagnosis));
                cx.notify();
            });
        }));
    }
}

fn explain(
    path: &str,
    diagnosis: &Diagnosis,
    settings: &Settings,
    language: Language,
) -> (SharedString, Vec<SharedString>) {
    let strings = language.strings();
    let pair = |what: &str, how: &str| (SharedString::from(what.to_owned()), vec![SharedString::from(how.to_owned())]);
    match diagnosis {
        Diagnosis::Indexed { hidden, low } => {
            let mut fixes: Vec<SharedString> = Vec::new();
            fixes.extend(hidden.then_some(strings.found_hidden_hint.into()));
            fixes.extend(low.then_some(strings.found_low_hint.into()));
            if fixes.is_empty() {
                fixes.push(strings.found_indexed_hint.into());
            }
            (strings.found_indexed.into(), fixes)
        }
        Diagnosis::Missing => pair(strings.found_missing, strings.found_missing_hint),
        Diagnosis::Gone => pair(strings.found_gone, strings.found_gone_hint),
        Diagnosis::Uncovered => match skipped_drive(path, settings) {
            Some(letter) => pair(&language.drive_skipped(letter), strings.found_drive_off_hint),
            None => pair(strings.found_uncovered, strings.found_uncovered_hint),
        },
        Diagnosis::Scanning => pair(strings.found_scanning, strings.found_scanning_hint),
        Diagnosis::System => pair(strings.found_system, strings.found_system_hint),
        Diagnosis::Recycled => pair(strings.found_recycled, strings.found_recycled_hint),
        Diagnosis::Excluded(folder) => pair(&language.excluded_by(folder), strings.found_excluded_hint),
        Diagnosis::Unseen { live: true } => pair(strings.found_unseen, strings.found_unseen_hint),
        Diagnosis::Unseen { live: false } => pair(strings.found_walked, strings.found_walked_hint),
    }
}

fn skipped_drive(path: &str, settings: &Settings) -> Option<char> {
    let letter = path.chars().next()?.to_ascii_uppercase();
    let rooted = Path::new(path).is_absolute() && path.get(1..2) == Some(":");
    (rooted && settings.index.skipped_drives.iter().any(|known| known.eq_ignore_ascii_case(&letter))).then_some(letter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_off_the_index_blames_a_drive_only_when_that_drive_is_turned_off() {
        let mut settings = Settings::default();
        settings.index.skipped_drives.push('D');
        assert_eq!(skipped_drive("d:\\Фото\\a.jpg", &settings), Some('D'));
        assert_eq!(skipped_drive("E:\\a.jpg", &settings), None);
        assert_eq!(skipped_drive("\\\\server\\d\\a.jpg", &settings), None);
        let (verdict, _) = explain("d:\\a.jpg", &Diagnosis::Uncovered, &settings, Language::English);
        assert_eq!(verdict.as_ref(), "Drive D: is turned off in the index");
    }
}
