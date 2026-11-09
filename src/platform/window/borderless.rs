use super::backdrop::Backdrop;
use super::reveal::{self, Reveal};
use super::shadow::{Look, Shadow};
use super::transition::Silhouette;
use super::{
    BASE_DPI, Shape, bring_to_front, cloak, default_top, drop_frame, glass, handle, outer, paint_now, physical,
    set_attribute, show_move_cursor,
};
use crate::theme::{Chrome, Corners, ShadowLayer, zoom_at};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_TRANSITIONS_FORCEDISABLED, DwmFlush};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, MonitorFromWindow, ScreenToClient,
    SetWindowRgn,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowPlacement, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTCLOSE, HTLEFT,
    HTMAXBUTTON, HTMINBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsIconic, IsWindowVisible, IsZoomed, MINMAXINFO,
    NCCALCSIZE_PARAMS, SM_CXPADDEDBORDER, SM_CXSIZEFRAME, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, SWP_NOACTIVATE,
    SWP_NOZORDER, SetWindowPlacement, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOWPLACEMENT, WM_DESTROY,
    WM_GETMINMAXINFO, WM_NCCALCSIZE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_SETCURSOR, WM_SIZE, WM_WINDOWPOSCHANGED,
    WPF_RESTORETOMAXIMIZED,
};
use windows::core::BOOL;

const BORDERLESS: usize = 3;
const CAPTION_BUTTONS: [u32; 3] = [HTCLOSE, HTMAXBUTTON, HTMINBUTTON];

#[derive(Clone, Copy)]
pub(crate) struct Caption {
    pub(crate) height: f32,
    pub(crate) button: f32,
}

thread_local! {
    static WINDOW: Cell<Option<HWND>> = const { Cell::new(None) };
    static STYLE: RefCell<Option<(Chrome, f32)>> = const { RefCell::new(None) };
    static CAPTION: Cell<Option<(Caption, f32)>> = const { Cell::new(None) };
    static REGION: Cell<Option<Shape>> = const { Cell::new(None) };
    static LEAST: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
    static FIT: Cell<bool> = const { Cell::new(false) };
    static BACKDROP: RefCell<Option<Backdrop>> = const { RefCell::new(None) };
    static SHADOW: RefCell<Option<Shadow>> = const { RefCell::new(None) };
}

pub(crate) fn attach_app_window(window: &gpui::Window, least: gpui::Size<gpui::Pixels>, cx: &mut gpui::App) {
    let Some(hwnd) = handle(window) else { return };
    WINDOW.set(Some(hwnd));
    LEAST.set(Some((f32::from(least.width), f32::from(least.height))));
    FIT.set(true);
    REGION.set(None);
    STYLE.set(None);
    cloak(hwnd, true);
    unsafe {
        set_attribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
        if !SetWindowSubclass(hwnd, Some(borderless), BORDERLESS, 0).as_bool() {
            log::warn!("Couldn't remove the Vestigo window border");
        }
    }
    drop_frame(hwnd, cx);
}

pub(crate) fn app_window_hidden() -> bool {
    WINDOW.get().is_some_and(|hwnd| !unsafe { IsWindowVisible(hwnd) }.as_bool())
}

/// Whether the user can see most of the Vestigo window now.
pub(crate) fn app_window_in_sight() -> bool {
    WINDOW.get().is_some_and(|hwnd| {
        let shown = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
        shown && super::sight::in_sight(hwnd)
    })
}

/// The Vestigo window as it will appear, its least size applied first, so a transition can end
/// exactly on it.
pub(crate) fn app_window_silhouette() -> Option<Silhouette> {
    let hwnd = WINDOW.get()?;
    if FIT.replace(false) {
        fit_least(hwnd);
    }
    let (window, maximized) = match unsafe { IsIconic(hwnd) }.as_bool() {
        true => comes_back_at(hwnd)?,
        false if unsafe { IsZoomed(hwnd) }.as_bool() => (super::client(hwnd), true),
        false => (outer(hwnd), false),
    };
    let radii = REGION.get().filter(|_| !maximized).map_or([0; 4], |region| region.radii);
    Some(Silhouette::new(window, radii, opacity(), !maximized))
}

fn opacity() -> f32 {
    STYLE.with_borrow(|style| style.as_ref().map_or(1.0, |(chrome, _)| chrome.opacity))
}

/// Where a minimized window returns to and whether it returns maximized.
fn comes_back_at(hwnd: HWND) -> Option<(RECT, bool)> {
    let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe { GetWindowPlacement(hwnd, &mut placement) }.ok()?;
    let normal = placement.rcNormalPosition;
    let monitor = unsafe { MonitorFromRect(&normal, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool().then_some(())?;
    if (placement.flags & WPF_RESTORETOMAXIMIZED).0 != 0 {
        return Some((info.rcWork, true));
    }
    let (x, y) = (info.rcWork.left - info.rcMonitor.left, info.rcWork.top - info.rcMonitor.top);
    let on_screen =
        RECT { left: normal.left + x, top: normal.top + y, right: normal.right + x, bottom: normal.bottom + y };
    Some((on_screen, false))
}

pub(crate) fn show_app_window() {
    if reveal::step(Reveal::ask).waits() {
        return;
    }
    let Some(hwnd) = WINDOW.get() else { return };
    let minimized = unsafe { IsIconic(hwnd) }.as_bool();
    if minimized || !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        reveal::step(Reveal::appear);
        cloak(hwnd, true);
        unsafe {
            let _ = ShowWindow(hwnd, if minimized { SW_RESTORE } else { SW_SHOW });
        }
    }
    if FIT.replace(false) {
        fit_least(hwnd);
    }
    paint_now(hwnd);
    let _ = unsafe { DwmFlush() };
    cloak(hwnd, false);
    if reveal::step(Reveal::appeared) == Reveal::Appearing {
        refresh_chrome();
    }
    if !bring_to_front(hwnd) {
        log::warn!("Windows kept the Vestigo window in the background");
    }
}

pub(super) fn refresh_chrome() {
    if let Some(hwnd) = WINDOW.get() {
        sync_chrome(hwnd);
    }
}

pub(super) fn await_reveal() {
    veil_app_window(0.0);
    refresh_chrome();
}

pub(super) fn end_reveal() {
    veil_app_window(opacity());
}

/// Sets the window's own opacity while a transition's shape hands over to it.
pub(crate) fn veil_app_window(share: f32) {
    if let Some(hwnd) = WINDOW.get().filter(|_| opacity() < 1.0) {
        glass::alpha(hwnd, share);
    }
}

pub(crate) fn hide_app_window() {
    let Some(hwnd) = WINDOW.get() else { return };
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

pub(crate) fn style_app_window(chrome: Chrome, zoom: f32, caption: Caption) {
    let Some(hwnd) = WINDOW.get() else { return };
    CAPTION.set(Some((caption, zoom)));
    let opacity = chrome.opacity;
    let previous = STYLE.replace(Some((chrome, zoom)));
    if previous.map(|(chrome, _)| chrome.opacity) != Some(opacity) {
        glass::opacity(hwnd, opacity);
    }
    shape(hwnd);
    sync_chrome(hwnd);
}

fn sync_chrome(hwnd: HWND) {
    let style =
        STYLE.with_borrow(|style| style.as_ref().map(|(chrome, zoom)| (chrome.blur, chrome.shadow.clone(), *zoom)));
    let Some((blur, layers, zoom)) = style else { return };
    let shown = unsafe { IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() };
    sync_backdrop(hwnd, blur, shown);
    sync_shadow(hwnd, if shown { layers } else { Vec::new() }, ratio(hwnd, zoom));
}

fn sync_shadow(hwnd: HWND, layers: Vec<ShadowLayer>, ratio: f32) {
    let region = REGION.get().filter(|_| !unsafe { IsZoomed(hwnd) }.as_bool());
    let Some(region) = region else {
        SHADOW.with_borrow(|shadow| shadow.as_ref().map(Shadow::hide));
        return;
    };
    SHADOW.with(|cell| {
        let Ok(mut shadow) = cell.try_borrow_mut() else { return };
        if shadow.is_none() && !layers.is_empty() {
            match Shadow::create(WINDOW_EX_STYLE::default()) {
                Ok(created) => *shadow = Some(created),
                Err(error) => log::warn!("The Vestigo window has no shadow: {error:#}"),
            }
        }
        if let Some(shadow) = shadow.as_mut() {
            let look = Look { layers, radii: region.radii.map(|radius| radius as f32) };
            shadow.fade(if reveal::current().withholds_shadow() { 0.0 } else { 1.0 });
            shadow.place(hwnd, outer(hwnd), ratio, &look);
        }
    });
}

fn sync_backdrop(hwnd: HWND, blur: bool, shown: bool) {
    BACKDROP.with(|cell| {
        let Ok(mut backdrop) = cell.try_borrow_mut() else { return };
        if !blur {
            *backdrop = None;
            return;
        }
        if !shown {
            if let Some(backdrop) = backdrop.as_ref() {
                backdrop.hide();
            }
            return;
        }
        if let Some(backdrop) = Backdrop::ensure(&mut backdrop, WINDOW_EX_STYLE::default(), "Vestigo window") {
            let window = outer(hwnd);
            let flat = Shape::new(window, Corners::default(), 1.0);
            backdrop.place(hwnd, window, REGION.get().unwrap_or(flat));
        }
    });
}

fn fit_least(hwnd: HWND) {
    let Some((width, height)) = least(hwnd) else { return };
    let Some(info) = monitor_info(hwnd).filter(|_| !unsafe { IsZoomed(hwnd) }.as_bool()) else { return };
    let spot = default_spot(info.rcWork, [width, height]);
    unsafe {
        let _ = SetWindowPos(hwnd, None, spot.left, spot.top, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

fn restore_default(hwnd: HWND) -> bool {
    let Some((width, height)) = least(hwnd) else { return false };
    let mut placement = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    let Some(info) = monitor_info(hwnd) else { return false };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
        return false;
    }
    placement.rcNormalPosition = default_placement(info.rcWork, info.rcMonitor, [width, height]);
    placement.showCmd = SW_SHOWNORMAL.0 as u32;
    unsafe { SetWindowPlacement(hwnd, &placement) }.is_ok()
}

fn monitor_info(hwnd: HWND) -> Option<MONITORINFO> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool().then_some(info)
}

/// The window on screen as it first appears: centred across the work area, its top where the
/// search bar's is by default, lower only when the window wouldn't fit.
fn default_spot(work: RECT, [width, height]: [i32; 2]) -> RECT {
    let (room_x, room_y) = (work.right - work.left, work.bottom - work.top);
    let (width, height) = (width.min(room_x), height.min(room_y));
    let left = work.left + (room_x - width) / 2;
    let top = default_top(work).min(work.bottom - height);
    RECT { left, top, right: left + width, bottom: top + height }
}

/// `default_spot` in the workspace coordinates of a window placement.
fn default_placement(work: RECT, monitor: RECT, size: [i32; 2]) -> RECT {
    let spot = default_spot(work, size);
    let (x, y) = (work.left - monitor.left, work.top - monitor.top);
    RECT { left: spot.left - x, top: spot.top - y, right: spot.right - x, bottom: spot.bottom - y }
}

fn least(hwnd: HWND) -> Option<(i32, i32)> {
    let (width, height) = LEAST.get()?;
    let scale = scale(hwnd);
    Some((physical(width, scale), physical(height, scale)))
}

fn scale(hwnd: HWND) -> f32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.0,
        dpi => dpi as f32 / BASE_DPI,
    }
}

fn ratio(hwnd: HWND, zoom: f32) -> f32 {
    let scale = scale(hwnd);
    scale * zoom_at(zoom, scale)
}

fn shape(hwnd: HWND) {
    let style = STYLE.with_borrow(|style| style.as_ref().map(|(chrome, zoom)| (chrome.corners, *zoom)));
    let Some((corners, zoom)) = style else { return };
    let window = outer(hwnd);
    let bounds = RECT { left: 0, top: 0, right: window.right - window.left, bottom: window.bottom - window.top };
    let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    let wanted = (!maximized).then(|| Shape::new(bounds, corners, ratio(hwnd, zoom)));
    if REGION.replace(wanted) == wanted {
        return;
    }
    match wanted {
        Some(region) => {
            if !region.apply(hwnd) {
                REGION.set(None);
                log::warn!("Couldn't round the Vestigo window");
            }
        }
        None => unsafe {
            SetWindowRgn(hwnd, None, true);
        },
    }
}

unsafe extern "system" fn borderless(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => {
            if unsafe { IsZoomed(hwnd) }.as_bool() && lparam.0 != 0 {
                let inset = system_metric(hwnd, SM_CXSIZEFRAME) + system_metric(hwnd, SM_CXPADDEDBORDER);
                let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
                let client = &mut params.rgrc[0];
                client.left += inset;
                client.top += inset;
                client.right -= inset;
                client.bottom -= inset;
            }
            return LRESULT(0);
        }
        WM_NCHITTEST => {
            if let Some(area) = resize_edge(hwnd, lparam).or_else(|| caption_area(hwnd, lparam)) {
                return LRESULT(area as _);
            }
        }
        WM_GETMINMAXINFO if lparam.0 != 0 && WINDOW.get() == Some(hwnd) => {
            if let Some((width, height)) = least(hwnd) {
                let limits = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                limits.ptMinTrackSize = POINT { x: width, y: height };
                return LRESULT(0);
            }
        }
        WM_SETCURSOR if show_move_cursor(lparam) => return LRESULT(1),
        WM_NCLBUTTONDBLCLK
            if wparam.0 == HTCAPTION as usize
                && WINDOW.get() == Some(hwnd)
                && unsafe { IsZoomed(hwnd) }.as_bool()
                && restore_default(hwnd) =>
        {
            return LRESULT(0);
        }
        WM_SIZE if WINDOW.get() == Some(hwnd) => {
            shape(hwnd);
            sync_chrome(hwnd);
        }
        WM_WINDOWPOSCHANGED if WINDOW.get() == Some(hwnd) => sync_chrome(hwnd),
        WM_DESTROY if WINDOW.get() == Some(hwnd) => {
            BACKDROP.with(|cell| {
                if let Ok(mut backdrop) = cell.try_borrow_mut() {
                    *backdrop = None;
                }
            });
            SHADOW.with(|cell| {
                if let Ok(mut shadow) = cell.try_borrow_mut() {
                    *shadow = None;
                }
            });
            WINDOW.set(None);
        }

        _ => {}
    }
    super::prompt::forward(hwnd, message, wparam, lparam)
}

fn resize_edge(hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    if unsafe { IsZoomed(hwnd) }.as_bool() {
        return None;
    }
    let window = outer(hwnd);
    let band = system_metric(hwnd, SM_CXSIZEFRAME);
    let (x, y) = (i32::from(lparam.0 as i16), i32::from((lparam.0 >> 16) as i16));
    let left = x < window.left + band;
    let right = x >= window.right - band;
    let top = y < window.top + band;
    let bottom = y >= window.bottom - band;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(HTTOPLEFT),
        (true, _, _, true) => Some(HTTOPRIGHT),
        (_, true, true, _) => Some(HTBOTTOMLEFT),
        (_, true, _, true) => Some(HTBOTTOMRIGHT),
        (true, _, _, _) => Some(HTTOP),
        (_, true, _, _) => Some(HTBOTTOM),
        (_, _, true, _) => Some(HTLEFT),
        (_, _, _, true) => Some(HTRIGHT),
        (false, false, false, false) => None,
    }
}

fn caption_area(hwnd: HWND, lparam: LPARAM) -> Option<u32> {
    let (caption, zoom) = CAPTION.get()?;
    let mut point = POINT { x: i32::from(lparam.0 as i16), y: i32::from((lparam.0 >> 16) as i16) };
    let mut client = RECT::default();
    unsafe { ScreenToClient(hwnd, &mut point).as_bool() && GetClientRect(hwnd, &mut client).is_ok() }.then_some(())?;
    let ratio = ratio(hwnd, zoom);
    if point.y >= physical(caption.height, ratio) {
        return Some(HTCLIENT);
    }
    let from_right = (client.right - 1 - point.x).max(0) / physical(caption.button, ratio).max(1);
    Some(usize::try_from(from_right).ok().and_then(|index| CAPTION_BUTTONS.get(index)).copied().unwrap_or(HTCAPTION))
}

fn system_metric(hwnd: HWND, metric: windows::Win32::UI::WindowsAndMessaging::SYSTEM_METRICS_INDEX) -> i32 {
    unsafe { GetSystemMetricsForDpi(metric, GetDpiForWindow(hwnd)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn the_default_window_is_centred_across_and_tops_where_the_search_bar_does() {
        let work = RECT { bottom: 1040, ..SCREEN };
        let rect = default_placement(work, SCREEN, [1000, 600]);
        assert_eq!(rect, RECT { left: 460, top: 208, right: 1460, bottom: 808 });
    }

    #[test]
    fn the_placement_is_counted_from_the_work_area_past_a_side_taskbar() {
        let work = RECT { left: 60, ..SCREEN };
        let rect = default_placement(work, SCREEN, [1000, 600]);
        assert_eq!((rect.left, rect.right), (430, 1430), "on screen it lands at 490, centred in the work area");
    }

    #[test]
    fn the_default_window_stays_on_its_own_monitor() {
        let second = RECT { left: 1920, top: 0, right: 3840, bottom: 1080 };
        let rect = default_placement(second, second, [1000, 600]);
        assert_eq!((rect.left, rect.top), (2380, 216));
    }

    #[test]
    fn the_default_window_never_outgrows_the_work_area() {
        let rect = default_placement(RECT { bottom: 1040, ..SCREEN }, SCREEN, [2000, 1200]);
        assert_eq!(rect, RECT { left: 0, top: 0, right: 1920, bottom: 1040 });
    }

    #[test]
    fn a_tall_default_window_rises_only_as_far_as_it_must() {
        let rect = default_placement(RECT { bottom: 1040, ..SCREEN }, SCREEN, [1000, 900]);
        assert_eq!((rect.top, rect.bottom), (140, 1040));
    }

    #[test]
    fn a_top_taskbar_moves_the_default_window_with_the_search_bar() {
        let work = RECT { top: 40, ..SCREEN };
        let rect = default_spot(work, [1000, 600]);
        assert_eq!(rect.top, 248, "the bar tops at 40 + 20 % of 1040");
        assert_eq!(default_placement(work, SCREEN, [1000, 600]).top, 208, "workspace coordinates start below the bar");
    }
}
