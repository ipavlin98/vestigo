mod apps;
mod autostart;
mod context_menu;
mod drag;
mod files;
mod hidden;
mod icons;
mod keys;
mod mru;
mod running;
mod shell;
mod tray;
mod watch;
mod web;
mod window;
mod worker;

pub(crate) use apps::{is_store_app, store_app_name, store_apps};
pub(crate) use autostart::{adopt_legacy as adopt_legacy_autostart, enabled as autostart, set as set_autostart};
pub(crate) use context_menu::{ShellItem, load_shell_menu, prepare_shell_menu, run_shell_item};
pub(crate) use drag::{drag, drag_distance};
pub(crate) use files::{explorer_selection, on_shell_thread, open_with, put_files_on_clipboard, recycle, run};
pub(crate) use hidden::{hidden_files_shown, show_hidden_files};
pub(crate) use icons::{IconSource, app_shortcuts, cached_thumbnail, load as load_icon, thumbnail};
pub(crate) use keys::layout_key;
pub(crate) use mru::{Picked, dialog_picks};
pub(crate) use running::{RunningApp, running_apps, switch_to};
pub(crate) use shell::{
    Picks, Purpose, Types, choose_in_dialog, current_folder, desktop_folder, dialog_purpose, downloads_folder,
    explorer_folders, focus_host, host_folder, landing_folders, navigate_explorer, open, properties, recent_items,
    reveal, run_as_admin, shortcut_folder,
};
pub(crate) use watch::{
    Remote, configure, note_ctrl_click, pause_hotkeys, signal_running_instance, start as start_watching,
    stop as stop_watching,
};
pub(crate) use web::{Transfer, download, fetch};
pub(crate) use window::{
    Anchor, Area, Caption, CardLayout, Entrance, Layout, MenuAt, MenuSpot, PointerMove, Silhouette, app_window_hidden,
    app_window_in_sight, app_window_on_hold, app_window_revealed, app_window_silhouette, attach, attach_app_window,
    attach_card, attach_menu, attach_transition, begin_transition, close_menu, end_transition, hide, hide_app_window,
    hide_menus, hold_app_window, note_change, note_placing, paint_transition, panel_silhouette, place, place_card,
    place_menu, pointer_move, pointer_on_panel, release_app_window, room, show, show_app_window, style_app_window,
    veil_app_window, veil_panel,
};
pub(crate) use worker::{enter_background, spawn_extractor};

use crate::i18n::{Language, Stamp};
use crate::index::Drive;
use crate::search::Clock;
use crate::settings::{Position, Shortcut, Triggers};
use futures::channel::mpsc::UnboundedSender;
use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, FILETIME, GetLastError, HANDLE, HWND, SYSTEMTIME,
};
use windows::Win32::Globalization::GetUserDefaultUILanguage;
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenMutexW, OpenProcessToken, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::System::Time::{
    FileTimeToSystemTime, GetTimeZoneInformation, SystemTimeToTzSpecificLocalTime, TIME_ZONE_INFORMATION,
};
use windows::Win32::UI::Shell::{SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, HSTRING, PCWSTR, w};

const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_RAMDISK: u32 = 6;
const LANG_RUSSIAN: u16 = 0x19;
const PRIMARY_LANGUAGE_MASK: u16 = 0x3ff;
const MUTEX_NAME: &str = "Local\\Vestigo.instance";
const PREVIOUS_EXIT_WAIT: Duration = Duration::from_secs(30);
const PREVIOUS_EXIT_POLL: Duration = Duration::from_millis(100);
const FILETIME_TICK_NS: u128 = 100;
const FILETIME_UNIX_EPOCH: u128 = 116_444_736_000_000_000;
const ZONE_STANDARD: u32 = 1;
const ZONE_DAYLIGHT: u32 = 2;
const SECONDS_PER_MINUTE: i64 = 60;

static EVENTS: OnceLock<UnboundedSender<Event>> = OnceLock::new();
static MOTION_REDUCED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static COM_READY: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKind {
    Explorer,
    Desktop,
    Dialog,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Clicked {
    pub(crate) host: Host,
    pub(crate) folder: Option<String>,
    pub(crate) hidden_shown: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Host {
    window: isize,
    pub(crate) kind: HostKind,
}

impl Host {
    fn new(window: HWND, kind: HostKind) -> Self {
        Self { window: window.0 as isize, kind }
    }

    fn hwnd(self) -> HWND {
        HWND(self.window as _)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hotkey {
    Search,
    Window,
    Menu,
    Action(usize),
    Command(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) shortcut: Shortcut,
    pub(crate) hotkey: Hotkey,
    pub(crate) explorer: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Watching {
    pub(crate) triggers: Triggers,
    pub(crate) bindings: Vec<Binding>,
    pub(crate) fullscreen: bool,
    pub(crate) tray: bool,
    pub(crate) disabled: Vec<String>,
    pub(crate) double_click: bool,
    pub(crate) middle_click: bool,
}

#[derive(Debug)]
pub(crate) enum Event {
    Search(SearchEvent),
    Hotkey(Hotkey, Option<Host>),
    FolderMenu(Clicked),
    IndexChanged,
    IndexCounted,
    Rebuild,
    DeleteContentCache,
    OpenWindow,
    OpenSettings,
    CheckUpdates,
    InstallUpdate,
    ShowUpdate,
    Quit,
}

#[derive(Debug)]
pub(crate) enum SearchEvent {
    Toggle,
    Summon,
    Typed { host: Host, text: String, first: bool },
    Attach { host: Host, explorer: Option<Host>, picks: Picks },
    DialogKey { host: Host, key: DialogKey },
    Detach,
    HostMoved,
    Moved(Position),
    ResetPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogKey {
    Switch,
    Row(usize),
}

impl From<SearchEvent> for Event {
    fn from(event: SearchEvent) -> Self {
        Self::Search(event)
    }
}

pub(crate) fn connect(sender: UnboundedSender<Event>) {
    if EVENTS.set(sender).is_err() {
        log::warn!("The event channel is already connected");
    }
}

pub(crate) fn send(event: impl Into<Event>) {
    if let Some(sender) = EVENTS.get()
        && sender.unbounded_send(event.into()).is_err()
    {
        log::debug!("The interface is gone, dropping an event");
    }
}

pub(crate) fn motion_reduced() -> bool {
    MOTION_REDUCED.load(Ordering::Relaxed)
}

fn read_motion() {
    let mut animated = BOOL(1);
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut animated as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if let Err(error) = read {
        log::warn!("Couldn't read the animation effects setting: {error}");
    }
    MOTION_REDUCED.store(!animated.as_bool(), Ordering::Relaxed);
}

pub(crate) struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) fn single_instance() -> Option<Instance> {
    unsafe {
        let mutex = CreateMutexW(None, false, &HSTRING::from(MUTEX_NAME)).ok()?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            return None;
        }
        Some(Instance(mutex))
    }
}

pub(crate) fn await_previous_exit() {
    let deadline = Instant::now() + PREVIOUS_EXIT_WAIT;
    while previous_running() && Instant::now() < deadline {
        std::thread::sleep(PREVIOUS_EXIT_POLL);
    }
}

fn previous_running() -> bool {
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &HSTRING::from(MUTEX_NAME)) } {
        Ok(mutex) => {
            let _ = unsafe { CloseHandle(mutex) };
            true
        }
        Err(error) => error.code() != ERROR_FILE_NOT_FOUND.to_hresult(),
    }
}

pub(crate) fn announce(title: &str, text: &str) -> bool {
    tray::announce(title, text)
}

pub(crate) fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0;
        let queried = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        let _ = CloseHandle(token);
        queried.is_ok() && elevation.TokenIsElevated != 0
    }
}

pub(crate) fn relaunch_elevated(arguments: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let exe = HSTRING::from(exe.as_os_str());
    let parameters = HSTRING::from(arguments);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.is_ok()
}

pub(crate) fn system_language() -> Language {
    let language = unsafe { GetUserDefaultUILanguage() };
    if language & PRIMARY_LANGUAGE_MASK == LANG_RUSSIAN { Language::Russian } else { Language::English }
}

pub(crate) fn fixed_drives(journal: bool) -> Vec<Drive> {
    let mask = unsafe { GetLogicalDrives() };
    (b'A'..=b'Z')
        .enumerate()
        .filter(|(bit, _)| mask & (1 << bit) != 0)
        .map(|(_, letter)| char::from(letter))
        .filter_map(|letter| {
            let root = HSTRING::from(format!("{letter}:\\"));
            if unsafe { GetDriveTypeW(&root) } != DRIVE_FIXED {
                return None;
            }
            let mut file_system = [0u16; 32];
            let known = unsafe { GetVolumeInformationW(&root, None, None, None, None, Some(&mut file_system)) }.is_ok();
            let ntfs = known && String::from_utf16_lossy(&file_system).trim_end_matches('\0') == "NTFS";
            Some(Drive { letter, journal: journal && ntfs })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    Quick,
    Slow,
    Gone,
}

pub(crate) fn reach(path: &str) -> Reach {
    let Some(letter) = path.chars().next().filter(|letter| letter.is_ascii_alphabetic() && path.get(1..2) == Some(":"))
    else {
        return Reach::Slow;
    };
    match unsafe { GetDriveTypeW(&HSTRING::from(format!("{letter}:\\"))) } {
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK => Reach::Quick,
        DRIVE_NO_ROOT_DIR => Reach::Gone,
        _ => Reach::Slow,
    }
}

pub(crate) fn local_time(time: SystemTime) -> Option<Stamp> {
    let ticks = time.duration_since(UNIX_EPOCH).ok()?.as_nanos() / FILETIME_TICK_NS + FILETIME_UNIX_EPOCH;
    let ticks = u64::try_from(ticks).ok()?;
    let file_time = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(Stamp {
        year: local.wYear,
        month: local.wMonth as u8,
        day: local.wDay as u8,
        hour: local.wHour as u8,
        minute: local.wMinute as u8,
    })
}

pub(crate) fn clock() -> Clock {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
    let mut zone = TIME_ZONE_INFORMATION::default();
    let bias = match unsafe { GetTimeZoneInformation(&mut zone) } {
        ZONE_STANDARD => zone.Bias + zone.StandardBias,
        ZONE_DAYLIGHT => zone.Bias + zone.DaylightBias,
        _ => zone.Bias,
    };
    Clock { now, offset: -i64::from(bias) * SECONDS_PER_MINUTE }
}

pub(crate) fn sync_tray(language: Language) {
    tray::sync(language);
}

fn ensure_com() {
    COM_READY.with(|ready| {
        if !ready.get() {
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            ready.set(true);
        }
    });
}
