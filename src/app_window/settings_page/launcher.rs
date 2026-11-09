use super::SettingsPage;
use super::form::{Field, Slot};
use super::{Block, Folding, Heading, Menu, Row, change, fold_row, section, setting, setting_row};
use crate::i18n::Strings;
use crate::settings::{Opening, Repeat, Settings, Suggestions};
use crate::theme::{Theme, rem};
use crate::ui::{CTRL, Choice, HINT_GAP, Key, button, hint_keys, shortcut_field, toggle};
use gpui::{Context, Div, Stateful, Window, div, prelude::*};

const ALT: Key = Key::Text("Alt");
const SHIFT: Key = Key::Text("Shift");
const LETTERS: Key = Key::Text("A…Z");

type Taught = (&'static str, &'static str, &'static [&'static [Key]]);

fn taught(strings: &Strings) -> [Taught; 18] {
    [
        (
            strings.keys_select,
            strings.keys_select_hint,
            &[&[Key::Text("↑")], &[Key::Text("↓")], &[ALT, Key::Text("J")], &[ALT, Key::Text("K")]],
        ),
        (strings.keys_half_page, strings.keys_half_page_hint, &[&[ALT, Key::Text("U")], &[ALT, Key::Text("D")]]),
        (strings.keys_ends, strings.keys_ends_hint, &[&[ALT, Key::Text("G")], &[ALT, SHIFT, Key::Text("G")]]),
        (strings.keys_open, strings.keys_open_hint, &[&[Key::Enter], &[CTRL, Key::Text("1…9")]]),
        (strings.keys_reveal, strings.keys_reveal_hint, &[&[CTRL, Key::Enter], &[ALT, Key::Text("R")]]),
        (strings.keys_copy_path, strings.keys_copy_path_hint, &[&[ALT, Key::Text("Y")]]),
        (strings.keys_actions, strings.keys_actions_hint, &[&[CTRL, Key::Text("O")], &[Key::Text("→")]]),
        (strings.keys_enter, strings.keys_enter_hint, &[&[ALT, Key::Text("L")]]),
        (strings.keys_containing, strings.keys_containing_hint, &[&[ALT, Key::Text(".")]]),
        (strings.keys_leave, strings.keys_leave_hint, &[&[ALT, Key::Text("H")]]),
        (strings.keys_mark, strings.keys_mark_hint, &[&[ALT, Key::Text("M")], &[LETTERS]]),
        (strings.keys_jump, strings.keys_jump_hint, &[&[ALT, Key::Text("'")], &[LETTERS]]),
        (strings.keys_preview, strings.keys_preview_hint, &[&[ALT, Key::Text("P")]]),
        (strings.keys_forget, strings.keys_forget_hint, &[&[SHIFT, Key::Text("Del")], &[CTRL, Key::Text("H")]]),
        (strings.keys_dialog, strings.keys_dialog_hint, &[&[CTRL, Key::Text("G")]]),
        (strings.menu_favorites, strings.keys_folders_hint, &[&[CTRL, Key::Text("B")]]),
        (strings.recent_and_open, strings.keys_folders_hint, &[&[CTRL, Key::Text("R")]]),
        (strings.keys_close, strings.keys_close_hint, &[&[Key::Text("Esc")]]),
    ]
}

impl SettingsPage {
    pub(super) fn launcher_section(
        &self,
        theme: &Theme,
        strings: &Strings,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<Block> {
        let openings = [Opening::Selected, Opening::Kept, Opening::Empty].map(|opening| Choice {
            value: opening,
            label: match opening {
                Opening::Selected => strings.opening_selected,
                Opening::Kept => strings.opening_kept,
                Opening::Empty => strings.opening_empty,
            }
            .into(),
            note: None,
            swatch: None,
        });
        let opening = self.picker(Menu::Opening, openings.into(), cx.global::<Settings>().opening, theme);
        let opening = self.dropdown(
            Menu::Opening,
            opening,
            |opening, cx| change(cx, |settings| settings.opening = opening),
            theme,
            window,
            cx,
        );
        let settings = cx.global::<Settings>();
        let flag = |id: &'static str, on: bool, flip: fn(&mut Settings)| {
            toggle(id, on, theme).on_click(move |_, _, cx| change(cx, flip))
        };
        let source = |id: &'static str, label, hint, pick: fn(&mut Suggestions) -> &mut bool| {
            let on = *pick(&mut settings.suggestions.clone());
            let control = toggle(id, on, theme).on_click(move |_, _, cx| {
                change(cx, |settings| {
                    let flag = pick(&mut settings.suggestions);
                    *flag = !*flag;
                })
            });
            setting(label, hint, control)
        };
        let hidden = settings.suggestions.hidden.len();
        let reset = button("hidden", strings.reset, hidden > 0, theme, |_, _, cx| {
            change(cx, |settings| settings.suggestions.hidden.clear())
        });
        let excluded = self
            .field(Field::ExcludedTypes, theme, window, cx)
            .map(|control| setting(strings.excluded_types, strings.excluded_types_hint, control));
        let mut suggestions = vec![
            source("history", strings.suggest_history, strings.suggest_history_hint, |sources| &mut sources.history),
            source("windows-recent", strings.suggest_windows, strings.suggest_windows_hint, |sources| {
                &mut sources.windows_recent
            }),
            source("downloads", strings.suggest_downloads, strings.suggest_downloads_hint, |sources| {
                &mut sources.downloads
            }),
            source("desktop", strings.suggest_desktop, strings.suggest_desktop_hint, |sources| &mut sources.desktop),
        ];
        suggestions.extend(excluded);
        suggestions.push(setting(strings.hidden_suggestions, strings.hidden_suggestions_hint, reset));
        let window_rows = vec![
            setting(strings.opening, strings.opening_hint, opening),
            setting(
                strings.show_recent,
                strings.show_recent_hint,
                flag("recent", settings.recent, |settings| settings.recent = !settings.recent),
            ),
            setting(
                strings.idle_status,
                strings.idle_status_hint,
                flag("idle-status", settings.idle_status, |settings| settings.idle_status = !settings.idle_status),
            ),
            setting(
                strings.hide_extensions,
                strings.hide_extensions_hint,
                flag("extensions", settings.hide_shortcut_extensions, |settings| {
                    settings.hide_shortcut_extensions = !settings.hide_shortcut_extensions
                }),
            ),
            setting(
                strings.small_icons,
                strings.small_icons_hint,
                flag("small-icons", settings.replace_small_icons, |settings| {
                    settings.replace_small_icons = !settings.replace_small_icons
                }),
            ),
            setting(
                strings.web_suggestions,
                strings.web_suggestions_hint,
                flag("web-suggestions", settings.web_suggestions, |settings| {
                    settings.web_suggestions = !settings.web_suggestions
                }),
            ),
        ];
        vec![section(Heading::Plain, window_rows), section(Heading::Title(strings.suggestion_sources), suggestions)]
    }

    pub(super) fn hotkeys(&self, theme: &Theme, strings: &Strings, window: &Window, cx: &Context<Self>) -> Vec<Block> {
        let settings = cx.global::<Settings>();
        let double_ctrl = toggle("double-ctrl", settings.triggers.double_ctrl, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.triggers.double_ctrl = !settings.triggers.double_ctrl));
        let fullscreen = toggle("fullscreen", settings.hotkeys.fullscreen, theme)
            .on_click(|_, _, cx| change(cx, |settings| settings.hotkeys.fullscreen = !settings.hotkeys.fullscreen));
        let repeats = [Repeat::Expand, Repeat::Close].map(|repeat| Choice {
            value: repeat,
            label: match repeat {
                Repeat::Expand => strings.repeat_expand,
                Repeat::Close => strings.repeat_close,
            }
            .into(),
            note: None,
            swatch: None,
        });
        let repeat = self.picker(Menu::Repeat, repeats.into(), settings.hotkeys.repeat, theme);
        let repeat = self.dropdown(
            Menu::Repeat,
            repeat,
            |repeat, cx| change(cx, |settings| settings.hotkeys.repeat = repeat),
            theme,
            window,
            cx,
        );
        let recorder = |slot: Slot, label, hint| setting(label, hint, self.recorder(slot, strings, theme, window, cx));
        vec![section(
            Heading::Note(strings.shortcut_hint),
            vec![
                setting(strings.double_ctrl, strings.double_ctrl_hint, double_ctrl),
                recorder(Slot::Search, strings.hotkey_search, strings.hotkey_search_hint),
                recorder(Slot::Window, strings.hotkey_window, strings.hotkey_window_hint),
                recorder(Slot::Menu, strings.hotkey_menu, strings.hotkey_menu_hint),
                setting(strings.repeat, strings.repeat_hint, repeat),
                setting(strings.fullscreen, strings.fullscreen_hint, fullscreen),
                self.window_keys(strings, theme, window, cx),
            ],
        )]
    }

    fn window_keys(&self, strings: &Strings, theme: &Theme, window: &Window, cx: &Context<Self>) -> Row {
        let open = self.form.keys_shown;
        let inner = if open {
            taught(strings)
                .into_iter()
                .map(|(label, hint, combos)| {
                    let keys = div()
                        .flex_none()
                        .flex()
                        .gap(rem(HINT_GAP))
                        .children(combos.iter().map(|keys| hint_keys(keys, theme, window)));
                    setting_row(label, vec![hint.into()], vec![keys.into_any_element()])
                })
                .collect()
        } else {
            Vec::new()
        };
        let folding = Folding {
            id: "window-keys".into(),
            open,
            controls: Vec::new(),
            toggle: Box::new(cx.listener(|this, _, _, cx| {
                this.form.keys_shown = !this.form.keys_shown;
                cx.notify();
            })),
            inner,
            unfold: None,
        };
        fold_row((strings.window_keys.into(), strings.window_keys_hint.into()), folding)
    }

    pub(super) fn recorder(
        &self,
        slot: Slot,
        strings: &Strings,
        theme: &Theme,
        window: &Window,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let settings = cx.global::<Settings>();
        let recording = (self.form.recording == Some(slot)).then_some(strings.shortcut_prompt);
        let id = gpui::ElementId::Name(format!("shortcut-{slot:?}").into());
        let field = shortcut_field(id, slot.get(settings), recording, strings.shortcut_empty, theme, window)
            .on_click(cx.listener(move |this, _, _, cx| this.start_recording(slot, cx)));
        match recording {
            Some(_) => field.on_mouse_down_out(cx.listener(|this, _, _, cx| this.stop_recording(cx))),
            None => field,
        }
    }
}
