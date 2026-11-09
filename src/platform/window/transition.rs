use super::reveal::{self, Reveal};
use super::{borderless, cloak, drop_frame, glass, handle, hide_panel_shadow, outer, paint_now, set_attribute};
use std::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED,
    DwmFlush,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, MA_NOACTIVATE, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WM_MOUSEACTIVATE,
    WM_NCCALCSIZE, WM_NCHITTEST, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

const OVERLAY: usize = 5;
const HIT_TRANSPARENT: isize = -1;

/// A rectangle on screen in device pixels with its corner radii (top left, top right, bottom
/// right, bottom left), its window opacity and whether it casts a shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Silhouette {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
    pub(crate) radii: [i32; 4],
    pub(crate) opacity: f32,
    pub(crate) casts_shadow: bool,
}

impl Silhouette {
    pub(super) fn new(rect: RECT, radii: [i32; 4], opacity: f32, casts_shadow: bool) -> Self {
        Self { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, radii, opacity, casts_shadow }
    }
}

/// Where the transition window covers the screen, in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

thread_local! {
    static WINDOW: Cell<Option<HWND>> = const { Cell::new(None) };
}

/// Turns the window into the transition overlay: topmost, never activated, letting the pointer
/// through to the windows of its thread beneath it, its pixels blended by their own alpha.
pub(crate) fn attach_transition(window: &gpui::Window, cx: &mut gpui::App) {
    let Some(hwnd) = handle(window) else { return };
    unsafe {
        set_attribute(hwnd, DWMWA_NCRENDERING_POLICY, &DWMNCRP_DISABLED);
        set_attribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
        glass::restyle(hwnd, glass::styles(hwnd) | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW);
        if !SetWindowSubclass(hwnd, Some(overlay), OVERLAY, 0).as_bool() {
            log::warn!("Couldn't remove the frame of the transition window");
        }
    }
    glass::alpha(hwnd, 1.0);
    drop_frame(hwnd, cx);
    WINDOW.set(Some(hwnd));
}

/// Keeps the Vestigo window from appearing until `release_app_window`: a reveal asked for meanwhile
/// waits and happens on release.
pub(crate) fn hold_app_window() {
    reveal::step(Reveal::hold);
}

/// The overlay rests on the window: the window may come forward under it, without a shadow of its
/// own and, if translucent, unseen, so the two never add up.
pub(crate) fn release_app_window() {
    let before = reveal::step(Reveal::release);
    if before.waits() {
        borderless::await_reveal();
    }
    if before == Reveal::Asked {
        super::show_app_window();
    }
}

pub(crate) fn app_window_on_hold() -> bool {
    reveal::current().waits()
}

pub(crate) fn app_window_revealed() -> bool {
    reveal::current().revealed()
}

/// Shows the overlay over `area` with its first frame painted.
pub(crate) fn begin_transition(area: Area) -> bool {
    let Some(hwnd) = WINDOW.get() else { return false };
    let rect = RECT { left: area.left, top: area.top, right: area.right, bottom: area.bottom };
    cloak(hwnd, true);
    let place = || unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .is_ok()
    };
    let placed = place() && (outer(hwnd) == rect || place());
    paint_now(hwnd);
    let _ = unsafe { DwmFlush() };
    hide_panel_shadow();
    cloak(hwnd, false);
    placed
}

pub(crate) fn paint_transition() {
    if let Some(hwnd) = WINDOW.get() {
        paint_now(hwnd);
    }
}

/// Hands the shadow from the overlay to the window within one composition.
pub(crate) fn end_transition() {
    release_app_window();
    borderless::end_reveal();
    let _ = unsafe { DwmFlush() };
    reveal::step(Reveal::end);
    borderless::refresh_chrome();
    if let Some(hwnd) = WINDOW.get() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe extern "system" fn overlay(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => LRESULT(0),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as _),
        WM_NCHITTEST => LRESULT(HIT_TRANSPARENT),
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}
