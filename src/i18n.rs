mod content;
mod en;
mod ru;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum LanguageChoice {
    #[default]
    System,
    English,
    Russian,
}

impl LanguageChoice {
    pub(crate) const ALL: [Self; 3] = [Self::System, Self::English, Self::Russian];

    pub(crate) fn resolve(self, system: Language) -> Language {
        match self {
            Self::System => system,
            Self::English => Language::English,
            Self::Russian => Language::Russian,
        }
    }

    pub(crate) fn label(self, strings: &Strings) -> &'static str {
        match self {
            Self::System => strings.follow_system,
            Self::English => "English",
            Self::Russian => "Русский",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    English,
    Russian,
}

impl gpui::Global for Language {}

impl Language {
    pub(crate) fn strings(self) -> &'static Strings {
        match self {
            Self::English => &en::EN,
            Self::Russian => &ru::RU,
        }
    }

    pub(crate) fn number(self, count: usize) -> String {
        group_digits(count)
    }

    pub(crate) fn items(self, count: usize) -> String {
        self.count(count, ["item", "items"], ["объект", "объекта", "объектов"])
    }

    pub(crate) fn matches(self, count: usize) -> String {
        self.count(count, ["match", "matches"], ["совпадение", "совпадения", "совпадений"])
    }

    pub(crate) fn drive_skipped(self, letter: char) -> String {
        match self {
            Self::English => format!("Drive {letter}: is turned off in the index"),
            Self::Russian => format!("Диск {letter}: выключен в индексе"),
        }
    }

    pub(crate) fn walked(self, count: usize, stamp: Stamp) -> String {
        let (items, date) = (self.items(count), self.date(stamp));
        match self {
            Self::English => format!("{items} · updated {date}"),
            Self::Russian => format!("{items} · обновлено {date}"),
        }
    }

    pub(crate) fn excluded_by(self, folder: &str) -> String {
        match self {
            Self::English => format!("The path is inside the excluded folder {folder}"),
            Self::Russian => format!("Путь внутри исключённой папки {folder}"),
        }
    }

    pub(crate) fn picked(self, count: usize) -> String {
        match self {
            Self::English => format!("{} selected", group_digits(count)),
            Self::Russian => format!("Выбрано: {}", group_digits(count)),
        }
    }

    pub(crate) fn marked(self, letter: char, folder: &str) -> String {
        match self {
            Self::English => format!("Mark {letter}: {folder}"),
            Self::Russian => format!("Метка {letter}: {folder}"),
        }
    }

    pub(crate) fn unmarked(self, letter: char) -> String {
        match self {
            Self::English => format!("No mark {letter} yet: Alt+M, {letter} sets it"),
            Self::Russian => format!("Метки {letter} нет: Alt+M, {letter} ставит её"),
        }
    }

    pub(crate) fn nth_of(self, position: usize, total: usize) -> String {
        let (position, total) = (group_digits(position), group_digits(total));
        match self {
            Self::English => format!("{position} of {total}"),
            Self::Russian => format!("{position} из {total}"),
        }
    }

    pub(crate) fn size(self, bytes: u64) -> String {
        let (units, point) = match self {
            Self::English => (["B", "KB", "MB", "GB", "TB"], "."),
            Self::Russian => (["Б", "КБ", "МБ", "ГБ", "ТБ"], ","),
        };
        let mut value = bytes as f64;
        let mut unit = 0;
        while value >= KIBI && unit + 1 < units.len() {
            value /= KIBI;
            unit += 1;
        }
        let name = units.get(unit).copied().unwrap_or_default();
        if unit == 0 || value >= 10.0 {
            format!("{}{SPACE}{name}", group_digits(value.round() as usize))
        } else {
            format!("{value:.1}{SPACE}{name}").replacen('.', point, 1)
        }
    }

    pub(crate) fn web(self, engine: &str, query: &str) -> String {
        match self {
            Self::English => format!("Search {engine} for “{query}”"),
            Self::Russian => format!("Найти «{query}» в {engine}"),
        }
    }

    pub(crate) fn theme_kept(self, theme: &str) -> String {
        match self {
            Self::English => format!("Applied at once and kept for “{theme}”"),
            Self::Russian => format!("Применяются сразу и хранятся для темы «{theme}»"),
        }
    }

    pub(crate) fn new_theme_hint(self, theme: &str) -> String {
        match self {
            Self::English => format!("Saves this look as a theme of its own, and “{theme}” goes back to how it was"),
            Self::Russian => format!("Сохраняет этот вид отдельной темой, а «{theme}» возвращается к исходному"),
        }
    }

    pub(crate) fn theme_saved(self, theme: &str) -> String {
        match self {
            Self::English => format!("Saved as “{theme}” in the themes folder"),
            Self::Russian => format!("Сохранено как «{theme}» в папке тем"),
        }
    }

    pub(crate) fn layers(self, count: usize) -> String {
        match (self, count) {
            (Self::English, 0) => "No shadow".to_owned(),
            (Self::Russian, 0) => "Без тени".to_owned(),
            _ => self.count(count, ["layer", "layers"], ["слой", "слоя", "слоёв"]),
        }
    }

    pub(crate) fn layer(self, number: usize) -> String {
        match self {
            Self::English => format!("Layer {number}"),
            Self::Russian => format!("Слой {number}"),
        }
    }

    pub(crate) fn changed(self, count: usize) -> String {
        match self {
            Self::English => format!("{count} changed"),
            Self::Russian => format!("изменено: {count}"),
        }
    }

    pub(crate) fn api_hint(self, port: u16) -> String {
        match self {
            Self::English => format!("Search from scripts: http://127.0.0.1:{port}/api/v1/search?q=report"),
            Self::Russian => format!("Поиск из скриптов: http://127.0.0.1:{port}/api/v1/search?q=отчёт"),
        }
    }

    pub(crate) fn update_available(self, version: &str) -> String {
        match self {
            Self::English => format!("Version {version} is available"),
            Self::Russian => format!("Доступна версия {version}"),
        }
    }

    pub(crate) fn update_announced(self, version: &str) -> String {
        match self {
            Self::English => format!("Vestigo {version} is available"),
            Self::Russian => format!("Доступен Vestigo {version}"),
        }
    }

    pub(crate) fn update_check_failed(self, reason: &str) -> String {
        match self {
            Self::English => format!("Couldn't check for updates: {reason}"),
            Self::Russian => format!("Не удалось проверить обновления: {reason}"),
        }
    }

    pub(crate) fn update_failed(self, reason: &str) -> String {
        match self {
            Self::English => format!("Couldn't update: {reason}"),
            Self::Russian => format!("Не удалось обновить: {reason}"),
        }
    }

    pub(crate) fn milliseconds(self, time: std::time::Duration) -> String {
        let value = self.decimal(time.as_secs_f64() * 1000.0);
        match self {
            Self::English => format!("{value}{SPACE}ms per search"),
            Self::Russian => format!("{value}{SPACE}мс на поиск"),
        }
    }

    pub(crate) fn decimal(self, value: f64) -> String {
        let text = format!("{value:.1}");
        match self {
            Self::English => text,
            Self::Russian => text.replacen('.', ",", 1),
        }
    }

    pub(crate) fn date(self, stamp: Stamp) -> String {
        let Stamp { year, month, day, hour, minute } = stamp;
        match self {
            Self::English => format!("{year}-{month:02}-{day:02} {hour:02}:{minute:02}"),
            Self::Russian => format!("{day:02}.{month:02}.{year} {hour:02}:{minute:02}"),
        }
    }

    fn count(self, count: usize, [one, many]: [&str; 2], [one_ru, few_ru, many_ru]: [&str; 3]) -> String {
        let word = match self {
            Self::English if count == 1 => one,
            Self::English => many,
            Self::Russian => match (count % 10, count % 100) {
                (1, rest) if rest != 11 => one_ru,
                (2..=4, rest) if !(12..=14).contains(&rest) => few_ru,
                _ => many_ru,
            },
        };
        format!("{}{SPACE}{word}", group_digits(count))
    }
}

pub(crate) type Words = &'static [(&'static str, &'static str)];

pub(crate) fn word(words: Words, key: &str) -> Option<&'static str> {
    words.iter().find(|(known, _)| *known == key).map(|(_, word)| *word)
}

pub(crate) struct Strings {
    pub(crate) placeholder: &'static str,
    pub(crate) placeholder_full_text: &'static str,
    pub(crate) placeholder_docked: &'static str,
    pub(crate) placeholder_actions: &'static str,
    pub(crate) recent: &'static str,
    pub(crate) open_folders: &'static str,
    pub(crate) no_results: &'static str,
    pub(crate) empty_folder: &'static str,
    pub(crate) indexing: &'static str,
    pub(crate) reading_details: &'static str,
    pub(crate) open: &'static str,
    pub(crate) go_to: &'static str,
    pub(crate) choose: &'static str,
    pub(crate) reveal: &'static str,
    pub(crate) tray_open: &'static str,
    pub(crate) tray_window: &'static str,
    pub(crate) tray_settings: &'static str,
    pub(crate) tray_quit: &'static str,
    pub(crate) follow_system: &'static str,
    pub(crate) theme_light: &'static str,
    pub(crate) theme_dark: &'static str,
    pub(crate) tooltip: &'static str,
    pub(crate) kind_all: &'static str,
    pub(crate) kind_folders: &'static str,
    pub(crate) kind_files: &'static str,
    pub(crate) kind_apps: &'static str,
    pub(crate) kind_documents: &'static str,
    pub(crate) kind_images: &'static str,
    pub(crate) kind_audio: &'static str,
    pub(crate) kind_video: &'static str,
    pub(crate) kind_archives: &'static str,
    pub(crate) settings: &'static str,
    pub(crate) column_name: &'static str,
    pub(crate) column_folder: &'static str,
    pub(crate) column_size: &'static str,
    pub(crate) column_modified: &'static str,
    pub(crate) shown_first: &'static str,
    pub(crate) section_appearance: &'static str,
    pub(crate) section_launcher: &'static str,
    pub(crate) section_shortcuts: &'static str,
    pub(crate) section_integration: &'static str,
    pub(crate) section_system: &'static str,
    pub(crate) theme: &'static str,
    pub(crate) themes_folder: &'static str,
    pub(crate) themes_folder_hint: &'static str,
    pub(crate) zoom: &'static str,
    pub(crate) zoom_hint: &'static str,
    pub(crate) theme_delete_failed: &'static str,
    pub(crate) theme_saving: &'static str,
    pub(crate) theme_changes: &'static str,
    pub(crate) theme_untouched: &'static str,
    pub(crate) reset_confirm: &'static str,
    pub(crate) new_theme: &'static str,
    pub(crate) theme_name: &'static str,
    pub(crate) save: &'static str,
    pub(crate) theme_save_failed: &'static str,
    pub(crate) palette: &'static str,
    pub(crate) shape: &'static str,
    pub(crate) font: &'static str,
    pub(crate) theme_zoom: &'static str,
    pub(crate) theme_zoom_hint: &'static str,
    pub(crate) radius_large: &'static str,
    pub(crate) radius_small: &'static str,
    pub(crate) radius_toggle: &'static str,
    pub(crate) windows_section: &'static str,
    pub(crate) elements: &'static str,
    pub(crate) custom_color: &'static str,
    pub(crate) corners: &'static str,
    pub(crate) corners_apart: &'static str,
    pub(crate) corner_top_left: &'static str,
    pub(crate) corner_top_right: &'static str,
    pub(crate) corner_bottom_right: &'static str,
    pub(crate) corner_bottom_left: &'static str,
    pub(crate) opacity: &'static str,
    pub(crate) blur: &'static str,
    pub(crate) shadow: &'static str,
    pub(crate) shadow_color: &'static str,
    pub(crate) offset_x: &'static str,
    pub(crate) offset_y: &'static str,
    pub(crate) pane_launcher: &'static str,
    pub(crate) pane_launcher_hint: &'static str,
    pub(crate) pane_dialog: &'static str,
    pub(crate) pane_dialog_hint: &'static str,
    pub(crate) pane_explorer: &'static str,
    pub(crate) pane_explorer_hint: &'static str,
    pub(crate) pane_app: &'static str,
    pub(crate) pane_app_hint: &'static str,
    pub(crate) theme_colors: &'static str,
    pub(crate) theme_form: &'static str,
    pub(crate) background_color: &'static str,
    pub(crate) background_hint: &'static str,
    pub(crate) text_color: &'static str,
    pub(crate) text_hint: &'static str,
    pub(crate) accent_color: &'static str,
    pub(crate) accent_hint: &'static str,
    pub(crate) font_hint: &'static str,
    pub(crate) rounding: &'static str,
    pub(crate) rounding_hint: &'static str,
    pub(crate) rounding_square: &'static str,
    pub(crate) rounding_slight: &'static str,
    pub(crate) rounding_medium: &'static str,
    pub(crate) rounding_round: &'static str,
    pub(crate) transparency: &'static str,
    pub(crate) transparency_hint: &'static str,
    pub(crate) clarity_solid: &'static str,
    pub(crate) clarity_light: &'static str,
    pub(crate) clarity_medium: &'static str,
    pub(crate) clarity_strong: &'static str,
    pub(crate) as_in_theme: &'static str,
    pub(crate) own_setting: &'static str,
    pub(crate) window_blur: &'static str,
    pub(crate) window_blur_hint: &'static str,
    pub(crate) window_shadows: &'static str,
    pub(crate) window_shadows_hint: &'static str,
    pub(crate) fine_tuning: &'static str,
    pub(crate) fine_tuning_hint: &'static str,
    pub(crate) palette_intro: &'static str,
    pub(crate) elements_intro: &'static str,
    pub(crate) shadow_blur: &'static str,
    pub(crate) shadow_spread: &'static str,
    pub(crate) theme_role_hints: Words,
    pub(crate) theme_group_hints: Words,
    pub(crate) theme_roles: Words,
    pub(crate) theme_groups: Words,
    pub(crate) theme_fields: Words,
    pub(crate) language: &'static str,
    pub(crate) show_recent: &'static str,
    pub(crate) opening: &'static str,
    pub(crate) opening_hint: &'static str,
    pub(crate) opening_selected: &'static str,
    pub(crate) opening_kept: &'static str,
    pub(crate) opening_empty: &'static str,
    pub(crate) show_recent_hint: &'static str,
    pub(crate) idle_status: &'static str,
    pub(crate) idle_status_hint: &'static str,
    pub(crate) hide_extensions: &'static str,
    pub(crate) hide_extensions_hint: &'static str,
    pub(crate) small_icons: &'static str,
    pub(crate) small_icons_hint: &'static str,
    pub(crate) history: &'static str,
    pub(crate) history_hint: &'static str,
    pub(crate) clear: &'static str,
    pub(crate) position: &'static str,
    pub(crate) position_hint: &'static str,
    pub(crate) reset_settings: &'static str,
    pub(crate) reset_settings_hint: &'static str,
    pub(crate) reset: &'static str,
    pub(crate) double_ctrl: &'static str,
    pub(crate) double_ctrl_hint: &'static str,
    pub(crate) menu_recent: &'static str,
    pub(crate) menu_favorites: &'static str,
    pub(crate) menu_nothing: &'static str,
    pub(crate) menu_no_favorites: &'static str,
    pub(crate) recent_and_open: &'static str,
    pub(crate) menu_commands: &'static str,
    pub(crate) menu_copy_folder_path: &'static str,
    pub(crate) menu_show_hidden: &'static str,
    pub(crate) menu_hide_hidden: &'static str,
    pub(crate) explorer: &'static str,
    pub(crate) explorer_hint: &'static str,
    pub(crate) dialogs: &'static str,
    pub(crate) dialogs_hint: &'static str,
    pub(crate) path_crumbs: &'static str,
    pub(crate) path_crumbs_hint: &'static str,
    pub(crate) open_dialog_suggestions: &'static str,
    pub(crate) open_dialog_suggestions_hint: &'static str,
    pub(crate) autostart: &'static str,
    pub(crate) autostart_hint: &'static str,
    pub(crate) index: &'static str,
    pub(crate) action_open_with: &'static str,
    pub(crate) action_admin: &'static str,
    pub(crate) action_copy: &'static str,
    pub(crate) action_cut: &'static str,
    pub(crate) action_copy_path: &'static str,
    pub(crate) action_copy_name: &'static str,
    pub(crate) action_delete: &'static str,
    pub(crate) action_properties: &'static str,
    pub(crate) action_context_menu: &'static str,
    pub(crate) command_settings: &'static str,
    pub(crate) command_rebuild: &'static str,
    pub(crate) command_parent: &'static str,
    pub(crate) command_new_folder: &'static str,
    pub(crate) command_new_file: &'static str,
    pub(crate) command_cmd: &'static str,
    pub(crate) command_cmd_admin: &'static str,
    pub(crate) command_powershell: &'static str,
    pub(crate) command_powershell_admin: &'static str,
    pub(crate) command_terminal: &'static str,
    pub(crate) terminal_needed: &'static str,
    pub(crate) command_shutdown: &'static str,
    pub(crate) command_restart: &'static str,
    pub(crate) command_uninstall: &'static str,
    pub(crate) command_connections: &'static str,
    pub(crate) command_hosts: &'static str,
    pub(crate) actions_status: &'static str,
    pub(crate) context_menu: &'static str,
    pub(crate) new_name_hint: &'static str,
    pub(crate) command_hint: &'static str,
    pub(crate) argument_hint: &'static str,
    pub(crate) run: &'static str,
    pub(crate) back: &'static str,
    pub(crate) section_general: &'static str,
    pub(crate) section_filters: &'static str,
    pub(crate) section_commands: &'static str,
    pub(crate) section_web: &'static str,
    pub(crate) section_menu: &'static str,
    pub(crate) section_about: &'static str,
    pub(crate) back_to_search: &'static str,
    pub(crate) settings_search: &'static str,
    pub(crate) nothing_found: &'static str,
    pub(crate) tray: &'static str,
    pub(crate) tray_hint: &'static str,
    pub(crate) suggest_history: &'static str,
    pub(crate) suggest_history_hint: &'static str,
    pub(crate) suggest_windows: &'static str,
    pub(crate) suggest_windows_hint: &'static str,
    pub(crate) suggest_downloads: &'static str,
    pub(crate) suggest_downloads_hint: &'static str,
    pub(crate) suggest_desktop: &'static str,
    pub(crate) suggest_desktop_hint: &'static str,
    pub(crate) excluded_types: &'static str,
    pub(crate) excluded_types_hint: &'static str,
    pub(crate) hidden_suggestions: &'static str,
    pub(crate) hidden_suggestions_hint: &'static str,
    pub(crate) web_suggestions: &'static str,
    pub(crate) web_suggestions_hint: &'static str,
    pub(crate) default_web: &'static str,
    pub(crate) default_web_hint: &'static str,
    pub(crate) hotkey_search: &'static str,
    pub(crate) hotkey_search_hint: &'static str,
    pub(crate) hotkey_window: &'static str,
    pub(crate) hotkey_window_hint: &'static str,
    pub(crate) hotkey_menu: &'static str,
    pub(crate) hotkey_menu_hint: &'static str,
    pub(crate) repeat: &'static str,
    pub(crate) repeat_hint: &'static str,
    pub(crate) repeat_expand: &'static str,
    pub(crate) repeat_close: &'static str,
    pub(crate) fullscreen: &'static str,
    pub(crate) fullscreen_hint: &'static str,
    pub(crate) shortcut_empty: &'static str,
    pub(crate) shortcut_prompt: &'static str,
    pub(crate) shortcut_hint: &'static str,
    pub(crate) window_keys: &'static str,
    pub(crate) window_keys_hint: &'static str,
    pub(crate) keys_select: &'static str,
    pub(crate) keys_select_hint: &'static str,
    pub(crate) keys_half_page: &'static str,
    pub(crate) keys_half_page_hint: &'static str,
    pub(crate) keys_ends: &'static str,
    pub(crate) keys_ends_hint: &'static str,
    pub(crate) keys_open: &'static str,
    pub(crate) keys_open_hint: &'static str,
    pub(crate) keys_reveal: &'static str,
    pub(crate) keys_reveal_hint: &'static str,
    pub(crate) keys_copy_path: &'static str,
    pub(crate) keys_copy_path_hint: &'static str,
    pub(crate) keys_actions: &'static str,
    pub(crate) keys_actions_hint: &'static str,
    pub(crate) keys_enter: &'static str,
    pub(crate) keys_enter_hint: &'static str,
    pub(crate) keys_containing: &'static str,
    pub(crate) keys_containing_hint: &'static str,
    pub(crate) keys_leave: &'static str,
    pub(crate) keys_leave_hint: &'static str,
    pub(crate) keys_mark: &'static str,
    pub(crate) keys_mark_hint: &'static str,
    pub(crate) keys_jump: &'static str,
    pub(crate) keys_jump_hint: &'static str,
    pub(crate) keys_preview: &'static str,
    pub(crate) keys_preview_hint: &'static str,
    pub(crate) keys_forget: &'static str,
    pub(crate) keys_forget_hint: &'static str,
    pub(crate) keys_dialog: &'static str,
    pub(crate) keys_dialog_hint: &'static str,
    pub(crate) keys_folders_hint: &'static str,
    pub(crate) keys_close: &'static str,
    pub(crate) keys_close_hint: &'static str,
    pub(crate) filters_hint: &'static str,
    pub(crate) field_name: &'static str,
    pub(crate) field_keyword: &'static str,
    pub(crate) field_extensions: &'static str,
    pub(crate) field_folder: &'static str,
    pub(crate) field_path: &'static str,
    pub(crate) field_arguments: &'static str,
    pub(crate) field_working: &'static str,
    pub(crate) field_url: &'static str,
    pub(crate) field_shortcut: &'static str,
    pub(crate) field_scope: &'static str,
    pub(crate) field_uninstaller: &'static str,
    pub(crate) uninstaller_classic: &'static str,
    pub(crate) uninstaller_settings: &'static str,
    pub(crate) field_hidden: &'static str,
    pub(crate) field_admin: &'static str,
    pub(crate) scope_vestigo: &'static str,
    pub(crate) scope_explorer: &'static str,
    pub(crate) scope_anywhere: &'static str,
    pub(crate) add: &'static str,
    pub(crate) delete: &'static str,
    pub(crate) add_filter: &'static str,
    pub(crate) add_action: &'static str,
    pub(crate) add_command: &'static str,
    pub(crate) add_web: &'static str,
    pub(crate) add_favorite: &'static str,
    pub(crate) new_filter: &'static str,
    pub(crate) new_action: &'static str,
    pub(crate) new_command: &'static str,
    pub(crate) new_web: &'static str,
    pub(crate) action_variables: &'static str,
    pub(crate) command_variables: &'static str,
    pub(crate) web_variables: &'static str,
    pub(crate) actions_hint: &'static str,
    pub(crate) commands_hint: &'static str,
    pub(crate) web_hint: &'static str,
    pub(crate) favorites_hint: &'static str,
    pub(crate) menu_tools_hint: &'static str,
    pub(crate) menu_tools_hidden_hint: &'static str,
    pub(crate) menu_tools_hidden_link: &'static str,
    pub(crate) menu_hidden_files: &'static str,
    pub(crate) menu_tool_off: &'static str,
    pub(crate) double_click: &'static str,
    pub(crate) double_click_hint: &'static str,
    pub(crate) middle_click: &'static str,
    pub(crate) middle_click_hint: &'static str,
    pub(crate) desktop: &'static str,
    pub(crate) desktop_hint: &'static str,
    pub(crate) disabled_apps: &'static str,
    pub(crate) disabled_apps_hint: &'static str,
    pub(crate) opener: &'static str,
    pub(crate) opener_hint: &'static str,
    pub(crate) opener_arguments: &'static str,
    pub(crate) opener_arguments_hint: &'static str,
    pub(crate) api: &'static str,
    pub(crate) api_port: &'static str,
    pub(crate) api_port_hint: &'static str,
    pub(crate) example_extensions: &'static str,
    pub(crate) example_folder: &'static str,
    pub(crate) example_path: &'static str,
    pub(crate) example_arguments: &'static str,
    pub(crate) example_working: &'static str,
    pub(crate) example_url: &'static str,
    pub(crate) example_types: &'static str,
    pub(crate) example_apps: &'static str,
    pub(crate) example_opener: &'static str,
    pub(crate) example_opener_arguments: &'static str,
    pub(crate) drives: &'static str,
    pub(crate) drive_off: &'static str,
    pub(crate) index_folders: &'static str,
    pub(crate) index_folders_hint: &'static str,
    pub(crate) rewalk: &'static str,
    pub(crate) rewalk_hint: &'static str,
    pub(crate) rewalk_never: &'static str,
    pub(crate) rewalk_quarter_hour: &'static str,
    pub(crate) rewalk_half_hour: &'static str,
    pub(crate) rewalk_hour: &'static str,
    pub(crate) rewalk_six_hours: &'static str,
    pub(crate) rewalk_day: &'static str,
    pub(crate) refresh_folder: &'static str,
    pub(crate) folder_in_drive: &'static str,
    pub(crate) excluded_folders: &'static str,
    pub(crate) excluded_folders_hint: &'static str,
    pub(crate) high_folders: &'static str,
    pub(crate) high_folders_hint: &'static str,
    pub(crate) low_folders: &'static str,
    pub(crate) low_folders_hint: &'static str,
    pub(crate) add_folder: &'static str,
    pub(crate) hidden_low: &'static str,
    pub(crate) hidden_low_hint: &'static str,
    pub(crate) rebuild_index: &'static str,
    pub(crate) rebuild_hint: &'static str,
    pub(crate) rebuild: &'static str,
    pub(crate) section_content: &'static str,
    pub(crate) full_text: &'static str,
    pub(crate) full_text_enable: &'static str,
    pub(crate) full_text_enable_confirm: &'static str,
    pub(crate) full_text_disable: &'static str,
    pub(crate) full_text_counting: &'static str,
    pub(crate) full_text_usage: &'static str,
    pub(crate) full_text_off_status: &'static str,
    pub(crate) full_text_short: &'static str,
    pub(crate) content_cache: &'static str,
    pub(crate) content_cache_none: &'static str,
    pub(crate) delete_confirm: &'static str,
    pub(crate) unread_denied: &'static str,
    pub(crate) unread_empty: &'static str,
    pub(crate) unread_binary: &'static str,
    pub(crate) unread_encrypted: &'static str,
    pub(crate) unread_failed: &'static str,
    pub(crate) diagnosis: &'static str,
    pub(crate) diagnose: &'static str,
    pub(crate) diagnose_hint: &'static str,
    pub(crate) diagnose_example: &'static str,
    pub(crate) check: &'static str,
    pub(crate) found_indexed: &'static str,
    pub(crate) found_indexed_hint: &'static str,
    pub(crate) found_hidden_hint: &'static str,
    pub(crate) found_low_hint: &'static str,
    pub(crate) found_missing: &'static str,
    pub(crate) found_missing_hint: &'static str,
    pub(crate) found_gone: &'static str,
    pub(crate) found_gone_hint: &'static str,
    pub(crate) found_uncovered: &'static str,
    pub(crate) found_uncovered_hint: &'static str,
    pub(crate) found_drive_off_hint: &'static str,
    pub(crate) found_scanning: &'static str,
    pub(crate) found_scanning_hint: &'static str,
    pub(crate) found_system: &'static str,
    pub(crate) found_system_hint: &'static str,
    pub(crate) found_recycled: &'static str,
    pub(crate) found_recycled_hint: &'static str,
    pub(crate) found_excluded_hint: &'static str,
    pub(crate) found_unseen: &'static str,
    pub(crate) found_unseen_hint: &'static str,
    pub(crate) found_walked: &'static str,
    pub(crate) found_walked_hint: &'static str,
    pub(crate) version: &'static str,
    pub(crate) indexed: &'static str,
    pub(crate) search_speed: &'static str,
    pub(crate) measure: &'static str,
    pub(crate) opens_per_day: &'static str,
    pub(crate) opens_per_day_hint: &'static str,
    pub(crate) updates: &'static str,
    pub(crate) updates_hint: &'static str,
    pub(crate) check_updates: &'static str,
    pub(crate) install_update: &'static str,
    pub(crate) update_checking: &'static str,
    pub(crate) update_latest: &'static str,
    pub(crate) update_downloading: &'static str,
    pub(crate) update_offline: &'static str,
    pub(crate) update_timed_out: &'static str,
    pub(crate) update_damaged: &'static str,
    pub(crate) auto_updates: &'static str,
    pub(crate) auto_updates_hint: &'static str,
    pub(crate) update_notice: &'static str,
    pub(crate) activity: &'static str,
    pub(crate) suggestion_sources: &'static str,
    pub(crate) automation: &'static str,
    pub(crate) period_any: &'static str,
    pub(crate) period_today: &'static str,
    pub(crate) period_week: &'static str,
    pub(crate) period_month: &'static str,
    pub(crate) period_year: &'static str,
    pub(crate) period_older: &'static str,
    pub(crate) preview_empty: &'static str,
    pub(crate) preview_size: &'static str,
    pub(crate) preview_created: &'static str,
    pub(crate) preview_modified: &'static str,
}

const SPACE: char = '\u{A0}';
const KIBI: f64 = 1024.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub(crate) year: u16,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
}

fn group_digits(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 * SPACE.len_utf8());
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(SPACE);
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_counts_use_plural_forms() {
        let ru = Language::Russian;
        assert_eq!(ru.items(1), "1\u{A0}объект");
        assert_eq!(ru.items(3), "3\u{A0}объекта");
        assert_eq!(ru.items(11), "11\u{A0}объектов");
        assert_eq!(ru.items(22), "22\u{A0}объекта");
        assert_eq!(ru.matches(112), "112\u{A0}совпадений");
        assert_eq!(ru.matches(21), "21\u{A0}совпадение");
        assert_eq!(ru.at_least(1250), "1\u{A0}250+\u{A0}совпадений");
        assert_eq!(
            ru.content_unread(&[(1250, "нет доступа"), (3, "ошибка")]),
            "Не прочитано: нет доступа — 1\u{A0}250, ошибка — 3"
        );
        assert_eq!(ru.content_unread(&[]), "");
        assert_eq!(ru.items(1_234_567), "1\u{A0}234\u{A0}567\u{A0}объектов");
    }

    #[test]
    fn sizes_pick_a_unit_and_a_decimal_mark() {
        assert_eq!(Language::English.size(512), "512\u{A0}B");
        assert_eq!(Language::English.size(1536), "1.5\u{A0}KB");
        assert_eq!(Language::Russian.size(1536), "1,5\u{A0}КБ");
        assert_eq!(Language::Russian.size(25 * 1024 * 1024), "25\u{A0}МБ");
        assert_eq!(Language::English.size(1023 * 1024 * 1024), "1\u{A0}023\u{A0}MB");
    }

    #[test]
    fn dates_follow_the_language() {
        let stamp = Stamp { year: 2026, month: 3, day: 7, hour: 9, minute: 5 };
        assert_eq!(Language::English.date(stamp), "2026-03-07 09:05");
        assert_eq!(Language::Russian.date(stamp), "07.03.2026 09:05");
    }

    #[test]
    fn english_counts_group_thousands() {
        assert_eq!(Language::English.items(1), "1\u{A0}item");
        assert_eq!(Language::English.matches(1), "1\u{A0}match");
        assert_eq!(Language::English.matches(12_345), "12\u{A0}345\u{A0}matches");
    }
}
