use std::ffi::c_void;
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WINDOW_EX_STYLE,
    WS_EX_LAYERED,
};
use windows::core::{BOOL, s, w};

const ACCENT_POLICY: u32 = 19;
const ACCENT_DISABLED: u32 = 0;
const ACCENT_BLUR_BEHIND: u32 = 3;

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    color: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetComposition = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

pub(super) fn blur(hwnd: HWND, on: bool) {
    let Some(set_composition) = composition() else {
        log::warn!("This Windows can't blur behind windows");
        return;
    };
    let mut accent =
        AccentPolicy { state: if on { ACCENT_BLUR_BEHIND } else { ACCENT_DISABLED }, flags: 0, color: 0, animation: 0 };
    let mut data = CompositionData {
        attribute: ACCENT_POLICY,
        data: (&mut accent as *mut AccentPolicy).cast(),
        size: size_of::<AccentPolicy>(),
    };
    if !unsafe { set_composition(hwnd, &mut data) }.as_bool() {
        log::warn!("Couldn't change the window blur");
    }
}

fn composition() -> Option<SetComposition> {
    let user32 = unsafe { GetModuleHandleW(w!("user32.dll")) }.ok()?;
    let address = unsafe { GetProcAddress(user32, s!("SetWindowCompositionAttribute")) }?;
    Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, SetComposition>(address) })
}

pub(super) fn styles(hwnd: HWND) -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32)
}

pub(super) fn restyle(hwnd: HWND, styles: WINDOW_EX_STYLE) {
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, styles.0 as _) };
}

pub(super) fn opacity(hwnd: HWND, opacity: f32) {
    let style = styles(hwnd);
    if opacity >= 1.0 {
        if style.contains(WS_EX_LAYERED) {
            restyle(hwnd, style & !WS_EX_LAYERED);
        }
        return;
    }
    alpha(hwnd, opacity);
}

pub(super) fn alpha(hwnd: HWND, opacity: f32) {
    let style = styles(hwnd);
    if !style.contains(WS_EX_LAYERED) {
        restyle(hwnd, style | WS_EX_LAYERED);
    }
    let alpha = (opacity.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;
    if unsafe { SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA) }.is_err() {
        log::warn!("Couldn't make the window translucent");
    }
}
