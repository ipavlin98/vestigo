use super::backdrop::Backdrop;
use super::shadow::{Look, Shadow};
use super::{Shape, cloak, drop_frame, glass, handle, outer, paint_now, prompt, set_attribute};
use crate::theme::Chrome;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY, DWMWA_TRANSITIONS_FORCEDISABLED,
};
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, IsWindowVisible, MA_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowPos, ShowWindow,
    WM_MOUSEACTIVATE, WM_NCCALCSIZE, WS_EX_NOACTIVATE, WS_EX_TOPMOST,
};
use windows::core::BOOL;

const FLOATING: usize = 4;
const QUIET: usize = 0;
const ACTIVATES: usize = 1;

pub(super) struct Floating {
    hwnd: HWND,
    what: &'static str,
    region: Option<Shape>,
    opacity: Option<f32>,
    shadow: Option<Shadow>,
    backdrop: Option<Backdrop>,
}

impl Floating {
    pub(super) fn attach(
        window: &gpui::Window,
        activates: bool,
        what: &'static str,
        cx: &mut gpui::App,
    ) -> Option<Self> {
        let hwnd = handle(window)?;
        unsafe {
            set_attribute(hwnd, DWMWA_NCRENDERING_POLICY, &DWMNCRP_DISABLED);
            set_attribute(hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
            set_attribute(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED, &BOOL::from(true));
            if !activates {
                glass::restyle(hwnd, glass::styles(hwnd) | WS_EX_NOACTIVATE);
            }
            let data = if activates { ACTIVATES } else { QUIET };
            if !SetWindowSubclass(hwnd, Some(floating), FLOATING, data).as_bool() {
                log::warn!("Couldn't remove the frame of the {what}");
            }
        }
        drop_frame(hwnd, cx);
        let shadow =
            Shadow::create(WS_EX_TOPMOST).inspect_err(|error| log::warn!("The {what} has no shadow: {error:#}")).ok();
        Some(Self { hwnd, what, region: None, opacity: None, shadow, backdrop: None })
    }

    pub(super) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub(super) fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd) }.as_bool()
    }

    pub(super) fn place(&mut self, rect: RECT, ratio: f32, chrome: &Chrome, shade: Option<Shape>) {
        let hwnd = self.hwnd;
        let bounds = RECT { left: 0, top: 0, right: rect.right - rect.left, bottom: rect.bottom - rect.top };
        let region = Shape::new(bounds, chrome.corners, ratio);
        let hidden = !self.visible();
        if hidden {
            cloak(hwnd, true);
        }
        if self.opacity.replace(chrome.opacity) != Some(chrome.opacity) {
            glass::opacity(hwnd, chrome.opacity);
        }
        let size_and_raise = || unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                rect.left,
                rect.top,
                bounds.right,
                bounds.bottom,
                SWP_NOACTIVATE,
            );
        };
        size_and_raise();
        if outer(hwnd) != rect {
            size_and_raise();
        }
        self.set_region(region);
        if hidden {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
        }
        paint_now(hwnd);
        self.sync_backdrop(chrome.blur, rect, region);
        if hidden {
            cloak(hwnd, false);
        }
        let shade = shade.unwrap_or(Shape { rect, radii: region.radii });
        let look = Look { layers: chrome.shadow.clone(), radii: shade.radii.map(|radius| radius as f32) };
        if let Some(shadow) = self.shadow.as_mut() {
            shadow.place(hwnd, shade.rect, ratio, &look);
            shadow.show(hwnd);
        }
    }

    pub(super) fn hide(&self) {
        if let Some(shadow) = self.shadow.as_ref() {
            shadow.hide();
        }
        if let Some(backdrop) = self.backdrop.as_ref() {
            backdrop.hide();
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    fn set_region(&mut self, region: Shape) {
        if self.region == Some(region) {
            return;
        }
        if !region.apply(self.hwnd) {
            log::warn!("Couldn't round the {}", self.what);
            return;
        }
        self.region = Some(region);
    }

    fn sync_backdrop(&mut self, blur: bool, rect: RECT, region: Shape) {
        if !blur {
            if let Some(backdrop) = self.backdrop.as_ref() {
                backdrop.hide();
            }
            return;
        }
        if let Some(backdrop) = Backdrop::ensure(&mut self.backdrop, WS_EX_TOPMOST, self.what) {
            backdrop.place(self.hwnd, rect, region);
        }
    }
}

unsafe extern "system" fn floating(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    match message {
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        WM_MOUSEACTIVATE if data == QUIET => return LRESULT(MA_NOACTIVATE as _),
        _ => {}
    }
    prompt::forward(hwnd, message, wparam, lparam)
}
