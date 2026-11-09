mod browse;
mod flow;
mod icons;
pub(crate) use icons::{Icons, picture, readable};
mod menu;
pub(crate) use menu::{
    Choose, Go, MenuEntry, MenuIcon, MenuItem, bind_keys as bind_menu_keys, close_search as close_search_menu,
    favorite_entries, folder_entries, follow as follow_menu, nothing, open as open_menu, open_at as open_menu_at,
    open_shell as open_shell_menu, recent_folders, shell_entries, shown_at as menu_shown_at, start as start_menus,
};
mod preview;
mod results;
mod scroll;
mod suggest;
mod tip;
mod tokens;
mod tools;
mod trail;
mod widgets;

pub(crate) use browse::{Browse, Browser, cancellable};
pub(crate) use flow::{Look, Mark};
pub(crate) use preview::{PREVIEW_WIDTH, Preview, Surface};
pub(crate) use results::{Arrival, Entry, Extra, Listing, Motion, ResultList, Shown, Tool, action_hints, result_hints};
pub(crate) use scroll::{edge_scroll, glide, glide_rows, scroll_to_child, scroll_to_span};
pub(crate) use suggest::{DialogSources, Sources};
pub(crate) use tip::{Tip, TipAnchor, hide_tips_on_keys, icon_tip};
pub(crate) use tokens::*;
pub(crate) use tools::{
    Subject, action_label, action_paths, action_rows, command_label, extras, perform_action, perform_command,
};
pub(crate) use trail::Trail;
pub(crate) use widgets::*;

use crate::i18n::Language;
use crate::index::{Hit, Index, Snippet, Status};
use crate::platform::{self, Host, RunningApp};
use crate::search::Grammar;
use crate::settings::{ActionSpec, BuiltinAction, Opener, Program, Scope, Settings};
use crate::theme::{BASE_REM, Theme, zoom_at};
use anyhow::Context as _;
use gpui::{
    Action, App, Bounds, Context, Entity, Global, HighlightStyle, KeyBinding, KeyBindingContextPredicate,
    MouseDownEvent, SharedString, StyledText, Subscription, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, actions, px, size,
};
use std::path::Path;
use std::rc::Rc;

const FOLDER_ARGUMENT: &str = "%1";
const HIDDEN_SIZE: f32 = 64.0;

impl Global for Status {}

pub(crate) fn publish_status(index: &Index, cx: &mut App) {
    let status = index.status();
    if cx.try_global::<Status>() != Some(&status) {
        cx.set_global(status);
    }
}

pub(crate) fn paint_changes<T: 'static>(cx: &mut Context<T>) -> Subscription {
    cx.observe_self(|_, _| platform::note_change())
}

pub(crate) fn paint_changes_of<T: 'static, V: 'static>(entity: &Entity<V>, cx: &mut Context<T>) -> Subscription {
    cx.observe(entity, |_, _, _| platform::note_change())
}

pub(crate) fn note_click(event: &MouseDownEvent) {
    if event.modifiers.control {
        platform::note_ctrl_click();
    }
}

pub(crate) fn grammar(cx: &App) -> Grammar {
    Grammar::new(&cx.global::<Settings>().filters, platform::clock()).with_home(std::env::var("USERPROFILE").ok())
}

pub(crate) fn search_placeholder(cx: &App) -> &'static str {
    let strings = cx.global::<Language>().strings();
    if cx.global::<Settings>().index.full_text { strings.placeholder_full_text } else { strings.placeholder }
}

pub(crate) fn zoom(cx: &App) -> f32 {
    cx.global::<Theme>().zoom * cx.global::<Settings>().zoom.factor()
}

pub(crate) fn fit_rem(window: &mut Window, cx: &App) -> bool {
    let rem = px(BASE_REM * zoom_at(zoom(cx), window.scale_factor()));
    let changed = window.rem_size() != rem;
    window.set_rem_size(rem);
    changed
}

pub(crate) fn popup_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(HIDDEN_SIZE), px(HIDDEN_SIZE)), cx))),
        titlebar: None,
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Opaque,
        inactive_frame_interval: None,
        ..Default::default()
    }
}

pub(crate) fn pixel_ratio(window: &Window) -> f32 {
    window.scale_factor() * f32::from(window.rem_size()) / BASE_REM
}

actions!(
    vestigo,
    [
        SelectNext,
        SelectPrevious,
        Confirm,
        Reveal,
        ShowActions,
        EnterFolder,
        EnterContaining,
        LeaveFolder,
        Back,
        Dismiss,
        TogglePreview,
        PickNext,
        PickPrevious
    ]
);

const MARKS: std::ops::RangeInclusive<char> = 'a'..='z';

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = vestigo, no_json)]
pub(crate) struct Jump {
    pub(crate) to: Motion,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = vestigo, no_json)]
pub(crate) struct SetMark {
    pub(crate) letter: char,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = vestigo, no_json)]
pub(crate) struct GoToMark {
    pub(crate) letter: char,
}

#[derive(Clone, PartialEq, gpui::Action)]
#[action(namespace = vestigo, no_json)]
pub(crate) struct RunAction {
    pub(crate) index: usize,
}

pub(crate) fn binding<A: Action>(keystrokes: &str, action: A, context: &str, cx: &App) -> Option<KeyBinding> {
    let predicate = KeyBindingContextPredicate::parse(context).ok().map(Rc::new);
    let mapper = cx.keyboard_mapper().as_ref();
    match KeyBinding::load(keystrokes, Box::new(action), predicate, true, None, mapper) {
        Ok(binding) => Some(binding),
        Err(error) => {
            log::warn!("Couldn't bind {keystrokes}: {error}");
            None
        }
    }
}

pub(crate) fn bind_keys(context: &'static str, actions: &[ActionSpec], cx: &mut App) {
    let fixed = [
        binding("down", SelectNext, context, cx),
        binding("tab", SelectNext, context, cx),
        binding("ctrl-n", SelectNext, context, cx),
        binding("alt-j", SelectNext, context, cx),
        binding("up", SelectPrevious, context, cx),
        binding("shift-tab", SelectPrevious, context, cx),
        binding("ctrl-p", SelectPrevious, context, cx),
        binding("alt-k", SelectPrevious, context, cx),
        binding("shift-down", PickNext, context, cx),
        binding("shift-up", PickPrevious, context, cx),
        binding("alt-u", Jump { to: Motion::HalfPageUp }, context, cx),
        binding("alt-d", Jump { to: Motion::HalfPageDown }, context, cx),
        binding("alt-g", Jump { to: Motion::First }, context, cx),
        binding("alt-shift-g", Jump { to: Motion::Last }, context, cx),
        binding("enter", Confirm, context, cx),
        binding("ctrl-enter", Reveal, context, cx),
        binding("alt-r", Reveal, context, cx),
        binding("ctrl-o", ShowActions, context, cx),
        binding("right", ShowActions, context, cx),
        binding("left", Back, context, cx),
        binding("alt-l", EnterFolder, context, cx),
        binding("alt-.", EnterContaining, context, cx),
        binding("alt-h", LeaveFolder, context, cx),
        binding("escape", Dismiss, context, cx),
        binding("alt-p", TogglePreview, context, cx),
    ];
    let marks = MARKS.flat_map(|letter| {
        [
            binding(&format!("alt-m {letter}"), SetMark { letter }, context, cx),
            binding(&format!("alt-' {letter}"), GoToMark { letter }, context, cx),
        ]
    });
    let copy_path = actions
        .iter()
        .position(|action| action.enabled && action.builtin == Some(BuiltinAction::CopyPath))
        .map(|index| binding("alt-y", RunAction { index }, context, cx));
    let custom =
        actions.iter().enumerate().filter(|(_, action)| action.enabled && action.scope != Scope::Explorer).filter_map(
            |(index, action)| {
                let shortcut = action.shortcut.as_ref().filter(|shortcut| shortcut.spares_typing())?;
                let keystroke = shortcut.keystroke();
                match action.builtin {
                    Some(BuiltinAction::Open) => binding(&keystroke, Confirm, context, cx),
                    Some(BuiltinAction::Reveal) => binding(&keystroke, Reveal, context, cx),
                    Some(_) | None => binding(&keystroke, RunAction { index }, context, cx),
                }
            },
        );
    let bindings: Vec<KeyBinding> = fixed.into_iter().chain(marks).chain(copy_path).flatten().chain(custom).collect();
    cx.bind_keys(bindings);
}

pub(crate) enum Command {
    Open(String),
    Reveal(String),
    RunAsAdmin(String),
    Properties(String),
    Navigate(Host, String),
    Choose { host: Host, path: String, keep_name: bool },
    Focus(Host),
    Switch { app: RunningApp, path: String },
    OpenWith(String),
    PutFiles { paths: Vec<String>, cut: bool },
    Recycle(Vec<String>),
    Drag(Vec<String>),
    Run(Program),
    Create { path: String, folder: bool, reveal: bool },
}

impl Command {
    fn needs_interface_thread(&self) -> bool {
        match self {
            Self::Drag(_) | Self::Focus(_) => true,
            Self::Open(_)
            | Self::Reveal(_)
            | Self::RunAsAdmin(_)
            | Self::Properties(_)
            | Self::Navigate(..)
            | Self::Choose { .. }
            | Self::Switch { .. }
            | Self::OpenWith(_)
            | Self::PutFiles { .. }
            | Self::Recycle(_)
            | Self::Run(_)
            | Self::Create { .. } => false,
        }
    }

    fn run(self, opener: Option<&Opener>) -> anyhow::Result<()> {
        match self {
            Self::Open(path) => match opener.filter(|_| Path::new(&path).is_dir()) {
                Some(opener) => platform::run(&opened(opener, &path)),
                None => platform::open(&path),
            },
            Self::Reveal(path) => match (opener, Path::new(&path).parent().and_then(Path::to_str)) {
                (Some(opener), Some(parent)) => platform::run(&opened(opener, parent)),
                (Some(_), None) | (None, _) => platform::reveal(&path),
            },
            Self::RunAsAdmin(path) => platform::run_as_admin(&path),
            Self::Properties(path) => platform::properties(&path),
            Self::Navigate(host, path) => platform::navigate_explorer(host, &path),
            Self::Choose { host, path, keep_name } => platform::choose_in_dialog(host, &path, keep_name),
            Self::Focus(host) => {
                platform::focus_host(host);
                Ok(())
            }
            Self::Switch { app, path } => platform::switch_to(app).or_else(|error| {
                log::debug!("Opening {path} again: {error:#}");
                platform::open(&path)
            }),
            Self::OpenWith(path) => platform::open_with(&path),
            Self::PutFiles { paths, cut } => platform::put_files_on_clipboard(&paths, cut),
            Self::Recycle(paths) => platform::recycle(&paths),
            Self::Drag(paths) => platform::drag(&paths),
            Self::Run(program) => platform::run(&program),
            Self::Create { path, folder, reveal } => {
                if folder {
                    std::fs::create_dir(&path)
                } else {
                    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map(drop)
                }
                .with_context(|| format!("creating {path}"))?;
                if reveal { platform::reveal(&path) } else { Ok(()) }
            }
        }
    }
}

pub(crate) fn perform(command: Command, cx: &mut App) {
    let opener = cx.try_global::<Settings>().map(|settings| settings.opener.clone());
    let opener = opener.filter(|opener| !opener.path.trim().is_empty());
    let here = command.needs_interface_thread();
    let job = move || {
        if let Err(error) = command.run(opener.as_ref()) {
            log::warn!("{error:#}");
        }
    };
    cx.spawn(async move |_| if here { job() } else { platform::on_shell_thread(job) }).detach();
}

fn opened(opener: &Opener, folder: &str) -> Program {
    let arguments = opener.arguments.trim();
    let arguments =
        if arguments.is_empty() { format!("\"{folder}\"") } else { arguments.replace(FOLDER_ARGUMENT, folder) };
    Program { path: opener.path.trim().to_owned(), arguments, ..Program::default() }
}

pub(crate) fn highlighted_name(hit: &Hit, hide_shortcut_extensions: bool, theme: &Theme) -> StyledText {
    let label = hit.label(hide_shortcut_extensions);
    let end = label.len();
    let style = HighlightStyle { color: Some(theme.results.matched), ..Default::default() };
    StyledText::new(SharedString::from(label.to_owned())).with_highlights(
        hit.highlights.iter().filter(|range| range.start < end).map(|range| (range.start..range.end.min(end), style)),
    )
}

pub(crate) fn snippet_text(snippet: &Snippet, theme: &Theme) -> StyledText {
    let style = HighlightStyle { color: Some(theme.results.matched), ..Default::default() };
    StyledText::new(SharedString::from(snippet.text.clone()))
        .with_highlights(snippet.ranges.iter().map(|range| (range.clone(), style)))
}

#[cfg(test)]
pub(crate) fn assert_whole_pixels(lengths: &[f32]) {
    use crate::theme::GRID;
    const SCALES: [f32; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
    for length in lengths {
        assert_eq!(length % GRID, 0.0, "{length} is off the grid");
    }
    for (length, scale) in lengths.iter().flat_map(|length| SCALES.map(|scale| (length, scale))) {
        assert_eq!((length * scale).fract(), 0.0, "{length} at {scale}");
    }
}

#[cfg(test)]
mod tests {
    use super::Command;
    use std::path::Path;

    #[test]
    fn only_a_drag_and_a_focus_change_stay_on_the_interface_thread() {
        assert!(Command::Drag(Vec::new()).needs_interface_thread());
        assert!(!Command::Open("\\\\server\\share".to_owned()).needs_interface_thread());
        assert!(!Command::Recycle(Vec::new()).needs_interface_thread());
        assert!(!Command::Create { path: "D:\\new".to_owned(), folder: true, reveal: false }.needs_interface_thread());
    }

    #[test]
    fn tokens_land_on_whole_pixels_at_common_scales() {
        super::assert_whole_pixels(&super::tokens::LENGTHS);
    }

    #[test]
    fn layout_lengths_are_named_constants() {
        let mut found = Vec::new();
        literal_lengths(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut found);
        assert!(found.is_empty(), "use a ui token or a named constant on the 4 px grid instead of: {found:#?}");
    }

    fn literal_lengths(dir: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("source folder").flatten() {
            let path = entry.path();
            if path.is_dir() {
                literal_lengths(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text = std::fs::read_to_string(&path).expect("source file");
                for (number, line) in text.lines().enumerate() {
                    if has_literal_length(line) {
                        found.push(format!("{}:{}: {}", path.display(), number + 1, line.trim()));
                    }
                }
            }
        }
    }

    fn has_literal_length(line: &str) -> bool {
        ["px(", "rem("].iter().any(|call| {
            line.match_indices(call).any(|(start, call)| {
                let argument = line.get(start + call.len()..).unwrap_or_default().trim_start();
                let argument = argument.strip_prefix('-').unwrap_or(argument);
                let zero = argument.starts_with("0.") || argument.starts_with("0)");
                argument.starts_with(|first: char| first.is_ascii_digit()) && !zero
            })
        })
    }

    #[test]
    fn interface_constants_sit_on_the_grid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        for layer in ["ui", "launcher", "app_window"] {
            files.push(root.join(format!("{layer}.rs")));
            for entry in std::fs::read_dir(root.join(layer)).expect("layer folder").flatten() {
                files.push(entry.path());
            }
        }
        let mut off_grid = Vec::new();
        for file in files.iter().filter(|file| file.extension().is_some_and(|extension| extension == "rs")) {
            let text = std::fs::read_to_string(file).expect("source file");
            off_grid
                .extend(text.lines().filter_map(off_grid_constant).map(|line| format!("{}: {line}", file.display())));
        }
        assert!(off_grid.is_empty(), "interface lengths must be multiples of 4 px: {off_grid:#?}");
    }

    fn off_grid_constant(line: &str) -> Option<&str> {
        const EXEMPT: [&str; 6] = ["_SIZE", "_SHARE", "_EPSILON", "_DEVICE", "_OPACITY", "RADIUS"];
        let declaration = line.trim();
        let rest = declaration.strip_prefix("pub(crate) ").or(declaration.strip_prefix("pub(super) "));
        let (name, value) = rest.unwrap_or(declaration).strip_prefix("const ")?.split_once(": f32 = ")?;
        let value: f32 = value.strip_suffix(';')?.parse().ok()?;
        let exempt = EXEMPT.iter().any(|suffix| name.ends_with(suffix));
        (!exempt && value % 4.0 != 0.0).then_some(declaration)
    }

    #[test]
    fn off_grid_constants_are_spotted() {
        assert!(off_grid_constant("const ROW: f32 = 37.0;").is_some());
        assert!(off_grid_constant("    pub(crate) const GAP: f32 = 6.0;").is_some());
        assert!(off_grid_constant("const ROW: f32 = 36.0;").is_none());
        assert!(off_grid_constant("pub(crate) const NAME_SIZE: f32 = 14.0;").is_none());
        assert!(off_grid_constant("const FOLDER_SHARE: f32 = 3.0;").is_none());
    }

    #[test]
    fn literal_lengths_are_spotted() {
        let call = "px(";
        assert!(has_literal_length(&format!(".h({call}36.0))")));
        assert!(has_literal_length(&format!("size({call}GAP), {call}2.))")));
        assert!(has_literal_length(&format!(".mx(rem(-{}))", 4)));
        assert!(!has_literal_length(".h(px(ROW)).flex_basis(px(0.))"));
        assert!(!has_literal_length(".h(rem(ROW)).mx(rem(-MARGIN))"));
        assert!(!has_literal_length("let lines = px(LINE) * 2.0;"));
    }
}
