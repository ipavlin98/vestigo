mod chord;
mod hooks;
mod mouse;
mod spot;
mod tap;
mod trail;

use super::running::{process_id, process_path};
use super::shell::{Picks, class_name, find_descendant, name_field};
use super::{Event, Host, HostKind, Hotkey, SearchEvent, Watching, read_motion, send, tray, window};
use anyhow::{Context, Result};
use chord::{Chord, Chords};
use hooks::HostEvents;
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tap::CtrlTap;
use trail::Trail;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    MOD_WIN, RegisterHotKey, ToUnicodeEx, UnregisterHotKey, VIRTUAL_KEY, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_LWIN,
    VK_MENU, VK_RCONTROL, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, CallNextHookEx, ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZESTART, FindWindowW, GUITHREADINFO, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
    GetParent, GetShellWindow, GetWindowRect, GetWindowThreadProcessId, HC_ACTION, HHOOK, IsWindowVisible,
    KBDLLHOOKSTRUCT, KillTimer, LLKHF_INJECTED, MSG, MSGFLT_ALLOW, OBJID_WINDOW, PostMessageW, PostQuitMessage,
    RegisterClassW, RegisterWindowMessageW, SPI_SETCLIENTAREAANIMATION, SendMessageW, SetTimer, TranslateMessage,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_HOTKEY, WM_KEYDOWN, WM_SETTINGCHANGE, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("Vestigo.watch");
const TOGGLE_MESSAGE: u32 = WM_APP + 1;
pub(super) const TRAY_MESSAGE: u32 = WM_APP + 2;
const QUIT_MESSAGE: u32 = WM_APP + 3;
const CONFIGURE_MESSAGE: u32 = WM_APP + 4;
const WINDOW_MESSAGE: u32 = WM_APP + 5;
const SETTINGS_MESSAGE: u32 = WM_APP + 6;
const CLICK_MESSAGE: u32 = WM_APP + 9;
const FOLLOW_MESSAGE: u32 = WM_APP + 10;
const FIRST_HOTKEY: i32 = 1;
const RECHECK_TIMER: usize = 1;
const RECHECK_DELAY_MS: u32 = 100;
const DIALOG_WAIT: Duration = Duration::from_secs(5);
const CAPTURE_TIME: Duration = Duration::from_millis(700);
const KEEP_KEYBOARD_STATE: u32 = 4;
const PRESSED: u8 = 0x80;
const TOGGLED: u8 = 0x01;
const MAX_ANCESTORS: usize = 8;
const FILE_LIST_CLASSES: [&str; 3] = ["DirectUIHWND", "SysListView32", "SHELLDLL_DefView"];
const SHELL_CLASSES: [&str; 3] = ["Progman", "WorkerW", "Shell_TrayWnd"];
const DIALOG_VIEWS: [&str; 2] = ["SHELLDLL_DefView", "DUIViewWndClassName"];
const PASSING_CLASSES: [&str; 1] = ["Windows.UI.Core.CoreWindow"];

static WATCH: AtomicIsize = AtomicIsize::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static CONFIG: Mutex<Option<Watching>> = Mutex::new(None);
static PAUSED: AtomicBool = AtomicBool::new(false);
static CTRL_CLICKED: AtomicBool = AtomicBool::new(false);

pub(crate) fn configure(watching: Watching) {
    *CONFIG.lock() = Some(watching);
    let hwnd = watch_window();
    if !hwnd.is_invalid() {
        let _ = unsafe { PostMessageW(Some(hwnd), CONFIGURE_MESSAGE, WPARAM(0), LPARAM(0)) };
    }
}

pub(crate) fn pause_hotkeys(paused: bool) {
    if PAUSED.swap(paused, Ordering::Relaxed) != paused {
        let hwnd = watch_window();
        if !hwnd.is_invalid() {
            let _ = unsafe { PostMessageW(Some(hwnd), CONFIGURE_MESSAGE, WPARAM(0), LPARAM(0)) };
        }
    }
}

pub(crate) fn note_ctrl_click() {
    CTRL_CLICKED.store(true, Ordering::Relaxed);
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub(crate) fn start() -> Result<()> {
    std::thread::Builder::new()
        .name("watch".into())
        .spawn(|| {
            if let Err(error) = run() {
                log::error!("Watching the desktop failed: {error:#}");
            }
        })
        .context("starting the watch thread")?;
    Ok(())
}

pub(crate) fn stop() {
    let hwnd = watch_window();
    if !hwnd.is_invalid() {
        unsafe { SendMessageW(hwnd, WM_CLOSE, None, None) };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Remote {
    Toggle,
    Window,
    Settings,
    Quit,
}

impl Remote {
    const ALL: [Self; 4] = [Self::Toggle, Self::Window, Self::Settings, Self::Quit];

    pub(crate) fn from_flag(flag: &str) -> Option<Self> {
        match flag {
            "--window" => Some(Self::Window),
            "--settings" => Some(Self::Settings),
            "--quit" => Some(Self::Quit),
            _ => None,
        }
    }

    fn message(self) -> u32 {
        match self {
            Self::Toggle => TOGGLE_MESSAGE,
            Self::Window => WINDOW_MESSAGE,
            Self::Settings => SETTINGS_MESSAGE,
            Self::Quit => QUIT_MESSAGE,
        }
    }
}

pub(crate) fn signal_running_instance(remote: Remote) -> bool {
    match unsafe { FindWindowW(CLASS, None) } {
        Ok(hwnd) if !hwnd.is_invalid() => {
            unsafe { PostMessageW(Some(hwnd), remote.message(), WPARAM(0), LPARAM(0)) }.is_ok()
        }
        _ => false,
    }
}

fn watch_window() -> HWND {
    HWND(WATCH.load(Ordering::Relaxed) as _)
}

fn request_follow() {
    let _ = unsafe { PostMessageW(Some(watch_window()), FOLLOW_MESSAGE, WPARAM(0), LPARAM(0)) };
}

fn run() -> Result<()> {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.context("reading the module handle")?.into();
    let class =
        WNDCLASSW { lpfnWndProc: Some(watch_proc), hInstance: instance, lpszClassName: CLASS, ..Default::default() };
    if unsafe { RegisterClassW(&class) } == 0 {
        anyhow::bail!("registering the watch window class failed");
    }
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_TOOLWINDOW, CLASS, w!("Vestigo"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None)
    }
    .context("creating the watch window")?;
    WATCH.store(hwnd.0 as isize, Ordering::Relaxed);
    read_motion();
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, Ordering::Relaxed);
    unsafe {
        for remote in Remote::ALL {
            let _ = ChangeWindowMessageFilterEx(hwnd, remote.message(), MSGFLT_ALLOW, None);
        }
    }
    with_state(|state| state.reconfigure(hwnd));
    let _hooks = hooks::install(instance)?;
    with_state(|state| state.foreground(unsafe { GetForegroundWindow() }));
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    with_state(|state| {
        state.unregister(hwnd);
        state.mouse = mouse::sync(state.mouse.take(), false, hwnd, CLICK_MESSAGE);
        state.followed.follow(false);
    });
    Ok(())
}

unsafe extern "system" fn watch_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        CLICK_MESSAGE => {
            if let Some(signal) = mouse::Signal::from_message(wparam) {
                with_state(|state| state.click(signal, mouse::point(lparam)));
            }
        }
        WM_HOTKEY => {
            if let Some(Some((hotkey, host))) = with_state(|state| state.hotkey(wparam.0)) {
                send(Event::Hotkey(hotkey, host));
            }
        }
        TOGGLE_MESSAGE => send(SearchEvent::Toggle),
        QUIT_MESSAGE => send(Event::Quit),
        WINDOW_MESSAGE => send(Event::OpenWindow),
        SETTINGS_MESSAGE => send(Event::OpenSettings),
        TRAY_MESSAGE => tray::on_message(hwnd, lparam),
        CONFIGURE_MESSAGE => {
            with_state(|state| {
                state.reconfigure(hwnd);
                state.foreground(unsafe { GetForegroundWindow() });
            });
        }
        WM_SETTINGCHANGE if wparam.0 == SPI_SETCLIENTAREAANIMATION.0 as usize => read_motion(),
        WM_TIMER if wparam.0 == RECHECK_TIMER => {
            with_state(State::recheck);
        }
        FOLLOW_MESSAGE => {
            with_state(|state| state.followed.follow(state.host.is_some()));
        }
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        WM_DESTROY => {
            tray::remove(hwnd);
            unsafe { PostQuitMessage(0) };
        }
        _ if message != 0 && message == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            with_state(|state| state.show_tray(hwnd, true));
        }
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if with_state(|state| state.key(wparam.0 as u32, info)).unwrap_or(false) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    if event == EVENT_SYSTEM_FOREGROUND {
        with_state(|state| state.foreground(hwnd));
    } else if object == OBJID_WINDOW.0 && child == CHILDID_SELF as i32 {
        with_state(|state| state.window_event(event, hwnd));
    }
}

fn with_state<T>(job: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|state| state.try_borrow_mut().ok().map(|mut state| job(&mut state)))
}

#[derive(Default)]
struct State {
    config: Watching,
    mouse: Option<HHOOK>,
    chords: Chords,
    registered: Vec<Hotkey>,
    tray: Option<bool>,
    host: Option<Host>,
    trail: Trail,
    blocked: bool,
    ctrl: CtrlTap,
    capture: Option<Capture>,
    waiting: Option<Waiting>,
    followed: HostEvents,
}

#[derive(Clone, Copy)]
struct Waiting {
    window: isize,
    until: Instant,
}

#[derive(Clone, Copy)]
struct Capture {
    host: Host,
    until: Instant,
}

impl State {
    fn reconfigure(&mut self, hwnd: HWND) {
        if let Some(config) = CONFIG.lock().take() {
            self.config = config;
        }
        self.unregister(hwnd);
        let paused = PAUSED.load(Ordering::Relaxed);
        self.chords = if paused { Chords::default() } else { Chords::new(&self.config.bindings) };
        for (position, (chord, hotkey)) in self.chords.global.iter().enumerate() {
            let id = FIRST_HOTKEY + position as i32;
            let modifiers = modifiers(*chord) | MOD_NOREPEAT;
            if let Err(error) = unsafe { RegisterHotKey(Some(hwnd), id, modifiers, u32::from(chord.key)) } {
                log::warn!("A shortcut for {hotkey:?} is taken by another program ({error})");
            }
            self.registered.push(*hotkey);
        }
        let clicks = self.config.double_click || self.config.middle_click;
        self.mouse = mouse::sync(self.mouse.take(), clicks, hwnd, CLICK_MESSAGE);
        self.show_tray(hwnd, false);
    }

    fn click(&self, signal: mouse::Signal, at: POINT) {
        let wanted = match signal.button {
            mouse::Button::Left => self.config.double_click,
            mouse::Button::Middle => self.config.middle_click,
        };
        if !wanted || self.blocked {
            return;
        }
        match signal.stage {
            mouse::Stage::Pressed => spot::probe(at),
            mouse::Stage::Clicked => spot::open(at, mouse::limits()),
        }
    }

    fn show_tray(&mut self, hwnd: HWND, recreated: bool) {
        let wanted = self.config.tray;
        if wanted && (recreated || self.tray != Some(true)) {
            tray::add(hwnd);
        } else if !wanted && self.tray == Some(true) {
            tray::remove(hwnd);
        }
        self.tray = Some(wanted);
    }

    fn unregister(&mut self, hwnd: HWND) {
        for position in 0..self.registered.len() {
            let _ = unsafe { UnregisterHotKey(Some(hwnd), FIRST_HOTKEY + position as i32) };
        }
        self.registered.clear();
    }

    fn hotkey(&self, id: usize) -> Option<(Hotkey, Option<Host>)> {
        let hotkey = *self.registered.get(id.checked_sub(FIRST_HOTKEY as usize)?)?;
        let foreground = unsafe { GetForegroundWindow() };
        let guarded = self.blocked || self.config.fullscreen && fullscreen(foreground);
        let host = self.host.filter(|host| host.hwnd() == foreground);
        (!guarded).then_some((hotkey, host))
    }

    fn foreground(&mut self, hwnd: HWND) {
        if hwnd.is_invalid() || window::is_ours(hwnd) {
            return;
        }
        self.blocked = !self.config.disabled.is_empty()
            && process_name(hwnd)
                .is_some_and(|name| self.config.disabled.iter().any(|app| app.eq_ignore_ascii_case(&name)));
        let host = classify(hwnd, &self.config).filter(|_| !self.blocked).map(|kind| Host::new(hwnd, kind));
        self.wait_for_dialog(hwnd, host.is_none() && !self.blocked && class_name(hwnd) == "#32770");
        if host != self.host && self.host.is_some() {
            send(SearchEvent::Detach);
        }
        if host.is_some() != self.host.is_some() {
            request_follow();
        }
        self.host = host;
        if let Some(host) = host.filter(|host| host.kind == HostKind::Dialog) {
            let picks = name_field(hwnd).map_or(Picks::Files, |(_, picks)| picks);
            send(SearchEvent::Attach { host, explorer: self.trail.explorer_host(), picks });
        }
        match class_name(hwnd).as_str() {
            "CabinetWClass" | "ExploreWClass" => self.trail.explorer(Host::new(hwnd, HostKind::Explorer)),
            class if PASSING_CLASSES.contains(&class) => {}
            _ => {
                let process = process_id(hwnd);
                if process != process_id(unsafe { GetShellWindow() }) {
                    self.trail.app(process);
                }
            }
        }
    }

    fn wait_for_dialog(&mut self, hwnd: HWND, unfinished: bool) {
        let window = hwnd.0 as isize;
        if !unfinished {
            self.stop_waiting();
        } else if self.waiting.is_none_or(|waiting| waiting.window != window) {
            self.waiting = Some(Waiting { window, until: Instant::now() + DIALOG_WAIT });
            unsafe { SetTimer(Some(watch_window()), RECHECK_TIMER, RECHECK_DELAY_MS, None) };
        }
    }

    fn stop_waiting(&mut self) {
        if self.waiting.take().is_some() {
            let _ = unsafe { KillTimer(Some(watch_window()), RECHECK_TIMER) };
        }
    }

    fn recheck(&mut self) {
        match self.waiting {
            Some(waiting) if Instant::now() < waiting.until => self.foreground(HWND(waiting.window as _)),
            _ => self.stop_waiting(),
        }
    }

    fn window_event(&mut self, event: u32, hwnd: HWND) {
        let Some(host) = self.host.filter(|host| host.hwnd() == hwnd) else { return };
        match event {
            EVENT_OBJECT_LOCATIONCHANGE => window::host_moved(),
            EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE | EVENT_SYSTEM_MINIMIZESTART => {
                self.host = None;
                self.capture = None;
                request_follow();
                send(SearchEvent::Detach);
            }
            _ => log::trace!("Ignoring event {event:#x} for {host:?}"),
        }
    }

    fn key(&mut self, message: u32, info: &KBDLLHOOKSTRUCT) -> bool {
        if info.flags.0 & LLKHF_INJECTED.0 != 0 || self.blocked {
            return false;
        }
        let key = VIRTUAL_KEY(info.vkCode as u16);
        let down = message == WM_KEYDOWN || message == WM_SYSKEYDOWN;
        if matches!(key, VK_LCONTROL | VK_RCONTROL | VK_CONTROL) {
            if CTRL_CLICKED.swap(false, Ordering::Relaxed) {
                self.ctrl.interrupt();
            }
            let summoned = self.ctrl.press(down, Instant::now())
                && self.config.triggers.double_ctrl
                && !PAUSED.load(Ordering::Relaxed);
            if summoned && !(self.config.fullscreen && fullscreen(unsafe { GetForegroundWindow() })) {
                send(SearchEvent::Summon);
            }
            return false;
        }
        if !down {
            return false;
        }
        self.ctrl.interrupt();
        self.explorer_hotkey(info) || self.dialog_key(info) || self.typed(info)
    }

    fn explorer_hotkey(&mut self, info: &KBDLLHOOKSTRUCT) -> bool {
        if self.chords.explorer.is_empty() {
            return false;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let Some(host) = self.host.filter(|host| host.hwnd() == foreground && host.kind != HostKind::Dialog) else {
            return false;
        };
        let Some(hotkey) = self.chords.in_explorer(pressed(info)) else { return false };
        self.capture = None;
        send(Event::Hotkey(hotkey, Some(host)));
        true
    }

    fn dialog_key(&mut self, info: &KBDLLHOOKSTRUCT) -> bool {
        let foreground = unsafe { GetForegroundWindow() };
        let Some(host) = self.host.filter(|host| host.hwnd() == foreground && host.kind == HostKind::Dialog) else {
            return false;
        };
        let Some(key) = pressed(info).in_dialog() else { return false };
        self.capture = None;
        send(SearchEvent::DialogKey { host, key });
        true
    }

    fn typed(&mut self, info: &KBDLLHOOKSTRUCT) -> bool {
        if modifiers_held() {
            return false;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let now = Instant::now();
        if let Some(capture) = self.capture.as_mut() {
            if capture.until > now && capture.host.hwnd() == foreground {
                let Some(text) = translate(info, foreground).filter(|text| printable(text)) else {
                    return false;
                };
                capture.until = now + CAPTURE_TIME;
                send(SearchEvent::Typed { host: capture.host, text, first: false });
                return true;
            }
            self.capture = None;
        }
        let Some(host) = self.host.filter(|host| host.hwnd() == foreground) else { return false };
        if !file_list_focused(foreground) {
            return false;
        }
        let Some(text) = translate(info, foreground).filter(|text| printable(text) && !text.starts_with(' ')) else {
            return false;
        };
        self.capture = Some(Capture { host, until: now + CAPTURE_TIME });
        send(SearchEvent::Typed { host, text, first: true });
        true
    }
}

fn pressed(info: &KBDLLHOOKSTRUCT) -> Chord {
    let held = |key: VIRTUAL_KEY| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0;
    Chord {
        key: info.vkCode as u16,
        ctrl: held(VK_CONTROL),
        alt: held(VK_MENU),
        shift: held(VK_SHIFT),
        win: held(VK_LWIN) || held(VK_RWIN),
    }
}

fn modifiers(chord: Chord) -> HOT_KEY_MODIFIERS {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for (on, flag) in [(chord.ctrl, MOD_CONTROL), (chord.alt, MOD_ALT), (chord.shift, MOD_SHIFT), (chord.win, MOD_WIN)]
    {
        if on {
            modifiers |= flag;
        }
    }
    modifiers
}

fn classify(hwnd: HWND, config: &Watching) -> Option<HostKind> {
    let triggers = config.triggers;
    match class_name(hwnd).as_str() {
        "CabinetWClass" | "ExploreWClass" if triggers.explorer => Some(HostKind::Explorer),
        "Progman" | "WorkerW" if triggers.desktop && has_file_list(hwnd) => Some(HostKind::Desktop),
        "#32770" if triggers.dialogs && is_file_dialog(hwnd) => Some(HostKind::Dialog),
        _ => None,
    }
}

fn fullscreen(hwnd: HWND) -> bool {
    if hwnd.is_invalid() || SHELL_CLASSES.contains(&class_name(hwnd).as_str()) {
        return false;
    }
    let mut rect = RECT::default();
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        if GetWindowRect(hwnd, &mut rect).is_err()
            || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool()
        {
            return false;
        }
    }
    let screen = info.rcMonitor;
    rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom
}

fn process_name(hwnd: HWND) -> Option<String> {
    process_path(hwnd)?.rsplit('\\').next().map(str::to_owned)
}

fn has_file_list(hwnd: HWND) -> bool {
    find_descendant(hwnd, &mut |child| class_name(child) == "SHELLDLL_DefView").is_some()
}

fn is_file_dialog(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) }.as_bool()
        && name_field(hwnd).is_some()
        && find_descendant(hwnd, &mut |child| DIALOG_VIEWS.contains(&class_name(child).as_str())).is_some()
}

fn file_list_focused(foreground: HWND) -> bool {
    let thread = unsafe { GetWindowThreadProcessId(foreground, None) };
    let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() || info.hwndFocus.is_invalid() {
        return false;
    }
    if !info.hwndCaret.is_invalid() || !FILE_LIST_CLASSES.contains(&class_name(info.hwndFocus).as_str()) {
        return false;
    }
    let mut current = info.hwndFocus;
    for _ in 0..MAX_ANCESTORS {
        if class_name(current) == "SHELLDLL_DefView" {
            return true;
        }
        match unsafe { GetParent(current) } {
            Ok(parent) if !parent.is_invalid() => current = parent,
            _ => return false,
        }
    }
    false
}

fn modifiers_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN].iter().any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0)
}

fn translate(info: &KBDLLHOOKSTRUCT, foreground: HWND) -> Option<String> {
    let thread = unsafe { GetWindowThreadProcessId(foreground, None) };
    let layout = unsafe { GetKeyboardLayout(thread) };
    let mut keys = [0u8; 256];
    if unsafe { GetAsyncKeyState(i32::from(VK_SHIFT.0)) } < 0
        && let Some(shift) = keys.get_mut(usize::from(VK_SHIFT.0))
    {
        *shift = PRESSED;
    }
    if unsafe { GetKeyState(i32::from(VK_CAPITAL.0)) } & 1 != 0
        && let Some(caps) = keys.get_mut(usize::from(VK_CAPITAL.0))
    {
        *caps = TOGGLED;
    }
    let mut buffer = [0u16; 8];
    let count =
        unsafe { ToUnicodeEx(info.vkCode, info.scanCode, &keys, &mut buffer, KEEP_KEYBOARD_STATE, Some(layout)) };
    let units = buffer.get(..usize::try_from(count).ok().filter(|&count| count > 0)?)?;
    String::from_utf16(units).ok()
}

fn printable(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|ch| !ch.is_control())
}
