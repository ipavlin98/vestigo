use super::{Host, HostKind, SearchEvent, send};
use crate::settings::Position;
use crate::theme::{Chrome, Corners, zoom_at};
use backdrop::Backdrop;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_COLOR_NONE, DWMWA_EXTENDED_FRAME_BOUNDS,
    DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWINDOWATTRIBUTE, DwmGetWindowAttribute,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, CreatePolygonRgn, CreateRectRgn, DeleteObject, GetMonitorInfoW, HMONITOR, HRGN,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromRect, RDW_INVALIDATE, RDW_UPDATENOW,
    RedrawWindow, SetWindowRgn, WINDING,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_MOUSE, SendInput, SetFocus};
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetClientRect, GetCursorPos, GetForegroundWindow, GetWindowRect, HTCAPTION, HWND_TOPMOST,
    IDC_SIZEALL, IsWindowVisible, KillTimer, LoadCursorW, MA_NOACTIVATE, PostMessageW, RegisterWindowMessageW,
    SET_WINDOW_POS_FLAGS, SW_HIDE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, WM_ENTERSIZEMOVE,
    WM_EXITSIZEMOVE, WM_MOUSEACTIVATE, WM_MOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_PAINT,
    WM_SETCURSOR, WM_TIMER, WS_EX_TOPMOST,
};
use windows::core::{BOOL, w};

mod backdrop;
mod borderless;
mod card;
mod floating;
mod glass;
mod menu;
mod prompt;
mod reveal;
mod shadow;
mod sight;
mod transition;

use shadow::{Look, Shadow};

pub(crate) use borderless::{
    Caption, app_window_hidden, app_window_in_sight, app_window_silhouette, attach_app_window, hide_app_window,
    show_app_window, style_app_window, veil_app_window,
};
pub(crate) use card::{CardLayout, attach_card, place_card};
pub(crate) use menu::{
    MenuAt, MenuSpot, PointerMove, attach_menu, close_menu, hide_menus, place_menu, pointer_move, pointer_on_panel,
};
pub(crate) use prompt::{note_change, note_placing};
pub(crate) use transition::{
    Area, Silhouette, app_window_on_hold, app_window_revealed, attach_transition, begin_transition, end_transition,
    hold_app_window, paint_transition, release_app_window,
};

const BASE_DPI: f32 = 96.0;
const CENTER_TOP: f32 = 0.2;
const FRAMELESS: usize = 1;
const ARC_STEP: f32 = 2.0;
const ARC_REACH: f32 = 1.0;
const FADE_TIME: Duration = Duration::from_millis(200);
const FADE_TIMER: usize = 0x5646;
const FADE_TICK_MS: u32 = 10;

static OURS: AtomicIsize = AtomicIsize::new(0);
static FOLLOW_MESSAGE: AtomicU32 = AtomicU32::new(0);
static FOLLOWING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static PANEL: Cell<Option<RECT>> = const { Cell::new(None) };
    static REGION: Cell<Option<Shape>> = const { Cell::new(None) };
    static GRIP: Cell<Option<RECT>> = const { Cell::new(None) };
    static DRAG_START: Cell<Option<RECT>> = const { Cell::new(None) };
    static OPACITY: Cell<Option<f32>> = const { Cell::new(None) };
    static BLUR: Cell<bool> = const { Cell::new(false) };
    static BACKDROP: RefCell<Option<Backdrop>> = const { RefCell::new(None) };
    static SHADOW: RefCell<Option<Shadow>> = const { RefCell::new(None) };
    static DOCKED: RefCell<Option<(Anchor, Layout)>> = const { RefCell::new(None) };
    static FADE: Cell<Option<Instant>> = const { Cell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entrance {
    Instant,
    Fade,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) panel_top: f32,
    pub(crate) panel_height: f32,
    pub(crate) grip: f32,
    pub(crate) grip_end: f32,
    pub(crate) zoom: f32,
    pub(crate) chrome: Chrome,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    rect: RECT,
    radii: [i32; 4],
}

impl Shape {
    fn new(rect: RECT, corners: Corners, ratio: f32) -> Self {
        let most = (rect.right - rect.left).min(rect.bottom - rect.top).max(0) / 2;
        Self { rect, radii: corners.all().map(|radius| physical(radius, ratio).clamp(0, most)) }
    }

    fn create(self) -> HRGN {
        let Self { rect, radii } = self;
        if radii == [0; 4] {
            return unsafe { CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom) };
        }
        unsafe { CreatePolygonRgn(&outline(rect, radii), WINDING) }
    }

    fn apply(self, hwnd: HWND) -> bool {
        unsafe {
            let shape = self.create();
            if SetWindowRgn(hwnd, Some(shape), true) == 0 {
                let _ = DeleteObject(shape.into());
                return false;
            }
        }
        true
    }
}

fn outline(rect: RECT, [top_left, top_right, bottom_right, bottom_left]: [i32; 4]) -> Vec<POINT> {
    use std::f32::consts::FRAC_PI_2;
    let corners = [
        (rect.left + top_left, rect.top + top_left, top_left, 2.0),
        (rect.right - top_right, rect.top + top_right, top_right, 3.0),
        (rect.right - bottom_right, rect.bottom - bottom_right, bottom_right, 0.0),
        (rect.left + bottom_left, rect.bottom - bottom_left, bottom_left, 1.0),
    ];
    let mut points = Vec::new();
    for (center_x, center_y, radius, quarter) in corners {
        let steps = ((radius as f32 / ARC_STEP).ceil() as usize).max(1);
        let reach = if radius > 0 { radius as f32 + ARC_REACH } else { 0.0 };
        points.extend((0..=steps).map(|step| {
            let angle = (quarter + step as f32 / steps as f32) * FRAC_PI_2;
            let x = (center_x as f32 + reach * angle.cos()).round() as i32;
            let y = (center_y as f32 + reach * angle.sin()).round() as i32;
            POINT { x: x.clamp(rect.left, rect.right), y: y.clamp(rect.top, rect.bottom) }
        }));
    }
    points
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Anchor {
    Center,
    Current,
    At(Position),
    BelowHost(Host),
    AboveHost(Host),
    InsideHost(Host),
}

fn handle(window: &gpui::Window) -> Option<HWND> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(raw) = handle.as_raw() else { return None };
    Some(HWND(raw.hwnd.get() as _))
}

pub(crate) fn attach(window: &gpui::Window, cx: &mut gpui::App) {
    let Some(hwnd) = handle(window) else { return };
    OURS.store(hwnd.0 as isize, Ordering::Relaxed);
    FOLLOW_MESSAGE.store(unsafe { RegisterWindowMessageW(w!("Vestigo.follow")) }, Ordering::Relaxed);
    let disabled = BOOL::from(true);
    unsafe {
        set_attribute(hwnd, DWMWA_NCRENDERING_POLICY, &DWMNCRP_DISABLED);
        set_attribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &disabled);
        if !SetWindowSubclass(hwnd, Some(frameless), FRAMELESS, 0).as_bool() {
            log::warn!("Couldn't remove the window frame");
        }
    }
    drop_frame(hwnd, cx);
    match Shadow::create(WS_EX_TOPMOST) {
        Ok(created) => SHADOW.set(Some(created)),
        Err(error) => log::warn!("The search window has no shadow: {error:#}"),
    }
}

fn with_shadow(job: impl FnOnce(&mut Shadow)) {
    SHADOW.with(|cell| {
        if let Ok(mut shadow) = cell.try_borrow_mut()
            && let Some(shadow) = shadow.as_mut()
        {
            job(shadow);
        }
    });
}

unsafe extern "system" fn frameless(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        WM_MOUSEACTIVATE if menu::press_stays_in_menu() => return LRESULT(MA_NOACTIVATE as _),
        WM_NCHITTEST if on_grip(hwnd, lparam) => return LRESULT(HTCAPTION as _),
        WM_SETCURSOR if show_move_cursor(lparam) => return LRESULT(1),
        WM_NCLBUTTONDBLCLK if wparam.0 == HTCAPTION as usize => {
            send(SearchEvent::ResetPosition);
            return LRESULT(0);
        }
        WM_TIMER if wparam.0 == FADE_TIMER => {
            step_fade(hwnd);
            return LRESULT(0);
        }
        WM_PAINT => step_fade(hwnd),
        WM_ENTERSIZEMOVE => DRAG_START.set(Some(outer(hwnd))),
        WM_MOVE if DRAG_START.get().is_some() => follow_panel(hwnd),
        WM_EXITSIZEMOVE => dropped(hwnd),
        _ if message != 0 && message == FOLLOW_MESSAGE.load(Ordering::Relaxed) => {
            follow_host(hwnd);
            return LRESULT(0);
        }
        _ => {}
    }
    prompt::forward(hwnd, message, wparam, lparam)
}

fn show_move_cursor(lparam: LPARAM) -> bool {
    if (lparam.0 & 0xFFFF) as u32 != HTCAPTION {
        return false;
    }
    let Ok(cursor) = (unsafe { LoadCursorW(None, IDC_SIZEALL) }) else { return false };
    unsafe { SetCursor(Some(cursor)) };
    true
}

fn on_grip(hwnd: HWND, lparam: LPARAM) -> bool {
    let Some(grip) = GRIP.get() else { return false };
    let window = outer(hwnd);
    let (x, y) = (i32::from(lparam.0 as i16) - window.left, i32::from((lparam.0 >> 16) as i16) - window.top);
    grip.left <= x && x < grip.right && grip.top <= y && y < grip.bottom
}

fn on_screen(hwnd: HWND, region: Shape) -> RECT {
    let window = outer(hwnd);
    RECT {
        left: window.left + region.rect.left,
        top: window.top + region.rect.top,
        right: window.left + region.rect.right,
        bottom: window.top + region.rect.bottom,
    }
}

fn follow_panel(hwnd: HWND) {
    let Some(region) = REGION.get() else { return };
    let panel = on_screen(hwnd, region);
    PANEL.set(Some(panel));
    with_shadow(|shadow| shadow.follow(hwnd, panel));
    sync_backdrop(hwnd, region);
    card::follow();
}

fn sync_backdrop(hwnd: HWND, region: Shape) {
    BACKDROP.with(|cell| {
        let Ok(mut backdrop) = cell.try_borrow_mut() else { return };
        if !BLUR.get() {
            if let Some(backdrop) = backdrop.as_ref() {
                backdrop.hide();
            }
            return;
        }
        if let Some(backdrop) = Backdrop::ensure(&mut backdrop, WS_EX_TOPMOST, "search window") {
            backdrop.place(hwnd, on_screen(hwnd, region), region);
        }
    });
}

fn dropped(hwnd: HWND) {
    let Some(start) = DRAG_START.take() else { return };
    let window = outer(hwnd);
    if (window.left, window.top) != (start.left, start.top) {
        follow_panel(hwnd);
        send(SearchEvent::Moved(Position { x: window.left, y: window.top }));
    }
}

pub(super) fn host_moved() {
    let Some(hwnd) = ours() else { return };
    if FOLLOWING.swap(true, Ordering::AcqRel) {
        return;
    }
    let message = FOLLOW_MESSAGE.load(Ordering::Relaxed);
    if unsafe { PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0)) }.is_err() {
        FOLLOWING.store(false, Ordering::Release);
    }
}

fn follow_host(hwnd: HWND) {
    FOLLOWING.store(false, Ordering::Release);
    let Some((anchor, layout)) = DOCKED.with_borrow(Clone::clone) else { return };
    let (target, _) = position(hwnd, anchor, &layout);
    let current = outer(hwnd);
    if (target.right - target.left, target.bottom - target.top)
        != (current.right - current.left, current.bottom - current.top)
    {
        place(anchor, layout);
    } else if (target.left, target.top) != (current.left, current.top) {
        unsafe {
            let flags = SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(hwnd, None, target.left, target.top, 0, 0, flags);
        }
        follow_panel(hwnd);
    } else {
        return;
    }
    send(SearchEvent::HostMoved);
}

unsafe fn set_attribute<T>(hwnd: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) {
    unsafe {
        let _ = DwmSetWindowAttribute(hwnd, attribute, (value as *const T).cast(), size_of::<T>() as u32);
    }
}

pub(super) fn is_ours(hwnd: HWND) -> bool {
    hwnd.0 as isize == OURS.load(Ordering::Relaxed) || menu::is_menu(hwnd)
}

fn ours() -> Option<HWND> {
    let hwnd = HWND(OURS.load(Ordering::Relaxed) as _);
    (!hwnd.is_invalid()).then_some(hwnd)
}

pub(crate) fn place(anchor: Anchor, layout: Layout) {
    prompt::placed();
    let Some(hwnd) = ours() else { return };
    let (window, scale) = position(hwnd, anchor, &layout);
    let docked = matches!(anchor, Anchor::BelowHost(_) | Anchor::AboveHost(_) | Anchor::InsideHost(_));
    DOCKED.set(docked.then(|| (anchor, layout.clone())));
    let ratio = scale * zoom_at(layout.zoom, scale);
    let top = window.top + physical(layout.panel_top, ratio);
    let panel =
        RECT { left: window.left, top, right: window.right, bottom: top + physical(layout.panel_height, ratio) };
    let bounds = RECT {
        left: 0,
        top: panel.top - window.top,
        right: panel.right - panel.left,
        bottom: panel.bottom - window.top,
    };
    let chrome = layout.chrome;
    let region = Shape::new(bounds, chrome.corners, ratio);
    let look = Look { layers: chrome.shadow, radii: region.radii.map(|radius| radius as f32) };
    let grip = physical(layout.grip, ratio);
    GRIP.set((grip > 0).then_some(RECT {
        left: 0,
        top: bounds.bottom - grip,
        right: bounds.right - physical(layout.grip_end, ratio),
        bottom: bounds.bottom,
    }));
    if OPACITY.replace(Some(chrome.opacity)) != Some(chrome.opacity) {
        glass::opacity(hwnd, chrome.opacity);
    }
    BLUR.set(chrome.blur);

    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let shrinks = visible && !PANEL.get().is_some_and(|old| contains(panel, old));
    PANEL.set(Some(panel));
    if shrinks {
        with_shadow(|shadow| shadow.place(hwnd, panel, ratio, &look));
    }
    let (width, height) = (window.right - window.left, window.bottom - window.top);
    let under_menu = if visible && menu::shown() { SWP_NOZORDER } else { SET_WINDOW_POS_FLAGS(0) };
    unsafe {
        let flags = SWP_NOACTIVATE | under_menu;
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), window.left, window.top, width, height, flags);
    }
    if shrinks {
        sync_backdrop(hwnd, region);
        set_region(hwnd, region);
    } else {
        set_region(hwnd, region);
        sync_backdrop(hwnd, region);
    }
    paint_now(hwnd);
    if !shrinks {
        with_shadow(|shadow| shadow.place(hwnd, panel, ratio, &look));
    }
    card::follow();
}

fn position(hwnd: HWND, anchor: Anchor, layout: &Layout) -> (RECT, f32) {
    let target = match anchor {
        Anchor::Center => None,
        Anchor::Current => Some(outer(hwnd)),
        Anchor::At(position) => {
            Some(RECT { left: position.x, top: position.y, right: position.x + 1, bottom: position.y + 1 })
        }
        Anchor::BelowHost(host) | Anchor::AboveHost(host) => Some(frame(host.hwnd())),
        Anchor::InsideHost(host) => Some(inside(host)),
    };
    let monitor = match target {
        Some(rect) => unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) },
        None => unsafe { MonitorFromPoint(cursor(), MONITOR_DEFAULTTONEAREST) },
    };
    let scale = scale(monitor);
    let ratio = scale * zoom_at(layout.zoom, scale);
    let work = work_area(monitor);
    let (w, h) = (physical(layout.width, ratio), physical(layout.height, ratio));
    let (x, y) = match (anchor, target) {
        (Anchor::Current | Anchor::At(_), Some(rect)) => (rect.left, rect.top),
        (Anchor::BelowHost(_), Some(rect)) => (rect.left + (rect.right - rect.left - w) / 2, rect.bottom),
        (Anchor::AboveHost(_), Some(rect)) => (rect.left + (rect.right - rect.left - w) / 2, rect.top - h),
        (Anchor::InsideHost(_), Some(rect)) => (rect.right - w, rect.bottom - h),
        _ => (work.left + (work.right - work.left - w) / 2, default_top(work)),
    };
    let x = x.clamp(work.left, (work.right - w).max(work.left));
    let y = match anchor {
        Anchor::BelowHost(_) | Anchor::AboveHost(_) => {
            let panel = (physical(layout.panel_top, ratio), physical(layout.panel_height, ratio));
            panel_on_screen(y, panel, work)
        }
        Anchor::Center | Anchor::Current | Anchor::At(_) | Anchor::InsideHost(_) => {
            y.clamp(work.top, (work.bottom - h).max(work.top))
        }
    };
    (RECT { left: x, top: y, right: x + w, bottom: y + h }, scale)
}

/// Where the search bar and the Vestigo window put their top edge until the user moves them, so
/// the bar grows into the window without a jump at the top.
fn default_top(work: RECT) -> i32 {
    work.top + ((work.bottom - work.top) as f32 * CENTER_TOP).round() as i32
}

fn panel_on_screen(top: i32, (panel_top, panel_height): (i32, i32), work: RECT) -> i32 {
    let lowest = work.top - panel_top;
    top.clamp(lowest, (work.bottom - panel_top - panel_height).max(lowest))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Room {
    pub(crate) below: f32,
    pub(crate) above: f32,
}

pub(crate) fn room(host: Host, zoom: f32) -> Room {
    let rect = frame(host.hwnd());
    let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST) };
    let scale = scale(monitor);
    let ratio = scale * zoom_at(zoom, scale);
    let work = work_area(monitor);
    Room { below: (work.bottom - rect.bottom) as f32 / ratio, above: (rect.top - work.top) as f32 / ratio }
}

fn inside(host: Host) -> RECT {
    match host.kind {
        HostKind::Desktop => work_area(unsafe { MonitorFromPoint(cursor(), MONITOR_DEFAULTTONEAREST) }),
        HostKind::Explorer | HostKind::Dialog => client(host.hwnd()),
    }
}

fn contains(outer: RECT, inner: RECT) -> bool {
    outer.left <= inner.left && outer.top <= inner.top && outer.right >= inner.right && outer.bottom >= inner.bottom
}

fn set_region(hwnd: HWND, region: Shape) {
    if REGION.get() == Some(region) {
        return;
    }
    if !region.apply(hwnd) {
        log::warn!("Couldn't limit the search window to its panel");
        return;
    }
    REGION.set(Some(region));
}

pub(crate) fn show(activate: bool, entrance: Entrance) -> bool {
    let Some(hwnd) = ours() else { return false };
    let hidden = !unsafe { IsWindowVisible(hwnd) }.as_bool();
    if hidden {
        cloak(hwnd, true);
        match entrance {
            Entrance::Fade => start_fade(hwnd),
            Entrance::Instant => {}
        }
    }
    unsafe {
        let _ = ShowWindow(hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
    }
    paint_now(hwnd);
    if let Some(region) = REGION.get() {
        sync_backdrop(hwnd, region);
    }
    if hidden {
        cloak(hwnd, false);
    }
    with_shadow(|shadow| shadow.show(hwnd));
    !activate || bring_to_front(hwnd)
}

fn start_fade(hwnd: HWND) {
    FADE.set(Some(Instant::now()));
    glass::alpha(hwnd, 0.0);
    with_shadow(|shadow| shadow.fade(0.0));
    unsafe { SetTimer(Some(hwnd), FADE_TIMER, FADE_TICK_MS, None) };
}

fn step_fade(hwnd: HWND) {
    let Some(start) = FADE.get() else { return };
    let progress = start.elapsed().as_secs_f32() / FADE_TIME.as_secs_f32();
    if progress >= 1.0 {
        finish_fade(hwnd);
        return;
    }
    let eased = 1.0 - (1.0 - progress).powi(3);
    glass::alpha(hwnd, OPACITY.get().unwrap_or(1.0) * eased);
    with_shadow(|shadow| shadow.fade(eased));
}

fn finish_fade(hwnd: HWND) {
    if FADE.take().is_none() {
        return;
    }
    let _ = unsafe { KillTimer(Some(hwnd), FADE_TIMER) };
    glass::alpha(hwnd, OPACITY.get().unwrap_or(1.0));
    with_shadow(|shadow| shadow.fade(1.0));
}

fn cloak(hwnd: HWND, cloaked: bool) {
    unsafe { set_attribute(hwnd, DWMWA_CLOAK, &BOOL::from(cloaked)) };
}

/// Makes Windows recalculate the frame the subclass removed. Its `WM_SIZE` reaches GPUI, so it runs
/// once the GPUI update that opened the window is over.
pub(super) fn drop_frame(hwnd: HWND, cx: &mut gpui::App) {
    cx.spawn(async move |_| {
        let unchanged = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED;
        unsafe {
            let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, unchanged);
        }
    })
    .detach();
}

fn paint_now(hwnd: HWND) {
    unsafe {
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_UPDATENOW);
    }
}

/// Hides the search window's shadow while a transition casts it instead.
fn hide_panel_shadow() {
    with_shadow(|shadow| shadow.hide());
}

/// The search window's panel on screen while it is shown.
pub(crate) fn panel_silhouette() -> Option<Silhouette> {
    let hwnd = ours()?;
    unsafe { IsWindowVisible(hwnd) }.as_bool().then_some(())?;
    let panel = PANEL.get()?;
    let radii = REGION.get().map_or([0; 4], |region| region.radii);
    Some(Silhouette::new(panel, radii, OPACITY.get().unwrap_or(1.0), true))
}

/// Sets the search window's own opacity while a transition's shape takes it over; a hide restores it.
pub(crate) fn veil_panel(share: f32) {
    if let Some(hwnd) = ours().filter(|_| OPACITY.get().is_some_and(|opacity| opacity < 1.0)) {
        glass::alpha(hwnd, share);
    }
}

pub(crate) fn hide() {
    DOCKED.set(None);
    card::place_card(None);
    with_shadow(|shadow| shadow.hide());
    BACKDROP.with(|cell| {
        if let Ok(backdrop) = cell.try_borrow()
            && let Some(backdrop) = backdrop.as_ref()
        {
            backdrop.hide();
        }
    });
    if let Some(hwnd) = ours() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        finish_fade(hwnd);
        if let Some(opacity) = OPACITY.get().filter(|opacity| *opacity < 1.0) {
            glass::alpha(hwnd, opacity);
        }
    }
}

pub(super) fn bring_to_front(hwnd: HWND) -> bool {
    let nudge = INPUT { r#type: INPUT_MOUSE, ..Default::default() };
    unsafe {
        SendInput(&[nudge], size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        let _ = SetFocus(Some(hwnd));
        GetForegroundWindow() == hwnd
    }
}

fn outer(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut rect);
    }
    rect
}

fn client(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    unsafe {
        if GetClientRect(hwnd, &mut rect).is_err() || !ClientToScreen(hwnd, &mut origin).as_bool() {
            return frame(hwnd);
        }
    }
    RECT { left: origin.x, top: origin.y, right: origin.x + rect.right, bottom: origin.y + rect.bottom }
}

fn frame(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    let visible = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if visible.is_err() {
        return outer(hwnd);
    }
    rect
}

fn cursor() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

fn scale(monitor: HMONITOR) -> f32 {
    let (mut x, mut y) = (0u32, 0u32);
    match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => x as f32 / BASE_DPI,
        _ => 1.0,
    }
}

fn work_area(monitor: HMONITOR) -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() { info.rcWork } else { RECT::default() }
}

fn physical(value: f32, ratio: f32) -> i32 {
    (value * ratio).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT { left: 0, top: 0, right: 2000, bottom: 1000 };

    #[test]
    fn a_bar_below_a_dialog_stays_flush_when_its_empty_rows_would_leave_the_screen() {
        assert_eq!(panel_on_screen(600, (0, 50), WORK), 600);
    }

    #[test]
    fn a_bar_above_a_dialog_stays_flush_when_its_empty_rows_would_leave_the_screen() {
        assert_eq!(panel_on_screen(-300, (400, 50), WORK), -300);
    }

    #[test]
    fn a_bar_that_would_leave_the_screen_is_pulled_back_onto_it() {
        assert_eq!(panel_on_screen(980, (0, 50), WORK), 950);
        assert_eq!(panel_on_screen(-420, (400, 50), WORK), -400);
    }
}
