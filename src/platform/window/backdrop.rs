use super::{Shape, glass, set_attribute};
use anyhow::{Context, Result};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DWMWA_TRANSITIONS_FORCEDISABLED;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, IsWindowVisible, RegisterClassW, SW_HIDE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

const CLASS: PCWSTR = w!("Vestigo.backdrop");

pub(super) struct Backdrop {
    hwnd: HWND,
    region: Option<Shape>,
}

impl Backdrop {
    fn create(extra: WINDOW_EX_STYLE) -> Result<Self> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.context("reading the module handle")?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(backdrop_proc),
            hInstance: instance,
            lpszClassName: CLASS,
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) };
        let style = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP | extra;
        let hwnd =
            unsafe { CreateWindowExW(style, CLASS, w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None) }
                .context("creating the blur window")?;
        unsafe { set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true)) };
        glass::blur(hwnd, true);
        Ok(Self { hwnd, region: None })
    }

    pub(super) fn ensure<'a>(slot: &'a mut Option<Self>, extra: WINDOW_EX_STYLE, what: &str) -> Option<&'a mut Self> {
        if slot.is_none() {
            match Self::create(extra) {
                Ok(created) => *slot = Some(created),
                Err(error) => {
                    log::warn!("The {what} can't blur: {error:#}");
                    return None;
                }
            }
        }
        slot.as_mut()
    }

    pub(super) fn place(&mut self, owner: HWND, frame: RECT, shape: Shape) {
        let region = Shape { rect: frame_size(frame), ..shape };
        if self.region != Some(region) {
            region.apply(self.hwnd);
            self.region = Some(region);
        }
        let mut flags = SWP_NOACTIVATE;
        if unsafe { IsWindowVisible(owner) }.as_bool() {
            flags |= SWP_SHOWWINDOW;
        }
        let (width, height) = (frame.right - frame.left, frame.bottom - frame.top);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(owner), frame.left, frame.top, width, height, flags);
        }
    }

    pub(super) fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn frame_size(frame: RECT) -> RECT {
    RECT { left: 0, top: 0, right: frame.right - frame.left, bottom: frame.bottom - frame.top }
}

unsafe extern "system" fn backdrop_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
