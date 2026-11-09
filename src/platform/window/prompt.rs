use std::cell::Cell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Shell::DefSubclassProc;
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, WM_KEYFIRST, WM_KEYLAST, WM_MOUSEFIRST, WM_MOUSELAST, WM_MOUSEMOVE, WM_NCMOUSELEAVE,
    WM_NCMOUSEMOVE, WM_NCXBUTTONDBLCLK, WindowFromPoint,
};

const AWAY: LPARAM = LPARAM(u32::MAX as _);

thread_local! {
    static CHANGED: Cell<bool> = const { Cell::new(false) };
    static PLACING: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn note_change() {
    CHANGED.set(true);
}

pub(crate) fn note_placing() {
    PLACING.set(true);
}

pub(super) fn placed() {
    PLACING.set(false);
}

pub(super) fn forward(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if !is_input(message) {
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    CHANGED.set(false);
    if (message == WM_MOUSELEAVE || message == WM_NCMOUSELEAVE) && !under_cursor(hwnd) {
        unsafe { DefSubclassProc(hwnd, WM_MOUSEMOVE, WPARAM(0), AWAY) };
    }
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if CHANGED.take() && !PLACING.get() {
        super::paint_now(hwnd);
    }
    result
}

fn under_cursor(hwnd: HWND) -> bool {
    unsafe { GetAncestor(WindowFromPoint(super::cursor()), GA_ROOT) == hwnd }
}

fn is_input(message: u32) -> bool {
    (WM_MOUSEFIRST..=WM_MOUSELAST).contains(&message)
        || (WM_NCMOUSEMOVE..=WM_NCXBUTTONDBLCLK).contains(&message)
        || (WM_KEYFIRST..=WM_KEYLAST).contains(&message)
        || message == WM_MOUSELEAVE
        || message == WM_NCMOUSELEAVE
}
