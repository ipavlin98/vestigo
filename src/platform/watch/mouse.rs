use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetSystemMetrics, HC_ACTION, HHOOK, MSLLHOOKSTRUCT, PostMessageW, SM_CXDOUBLECLK, SM_CYDOUBLECLK,
    SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MBUTTONUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Button {
    Left,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Pressed,
    Clicked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Signal {
    pub(super) button: Button,
    pub(super) stage: Stage,
}

impl Signal {
    const ALL: [Self; 4] = [
        Self { button: Button::Left, stage: Stage::Pressed },
        Self { button: Button::Left, stage: Stage::Clicked },
        Self { button: Button::Middle, stage: Stage::Pressed },
        Self { button: Button::Middle, stage: Stage::Clicked },
    ];

    pub(super) fn from_message(wparam: WPARAM) -> Option<Self> {
        Self::ALL.get(wparam.0).copied()
    }

    fn code(self) -> usize {
        Self::ALL.iter().position(|signal| *signal == self).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Press {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) time: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Limits {
    pub(super) time: u32,
    pub(super) width: i32,
    pub(super) height: i32,
}

#[derive(Default)]
pub(super) struct Clicks {
    last: Option<Press>,
}

impl Clicks {
    pub(super) fn press(&mut self, press: Press, limits: Limits) -> bool {
        let double = self.last.is_some_and(|last| {
            press.time.wrapping_sub(last.time) <= limits.time
                && (press.x - last.x).abs() <= limits.width / 2
                && (press.y - last.y).abs() <= limits.height / 2
        });
        self.last = if double { None } else { Some(press) };
        double
    }
}

thread_local! {
    static CLICKS: RefCell<Clicks> = RefCell::new(Clicks::default());
    static TARGET: RefCell<(HWND, u32)> = const { RefCell::new((HWND(std::ptr::null_mut()), 0)) };
}

pub(super) fn sync(hook: Option<HHOOK>, wanted: bool, target: HWND, message: u32) -> Option<HHOOK> {
    TARGET.with(|cell| {
        if let Ok(mut current) = cell.try_borrow_mut() {
            *current = (target, message);
        }
    });
    match (hook, wanted) {
        (Some(hook), true) => Some(hook),
        (Some(hook), false) => {
            let _ = unsafe { UnhookWindowsHookEx(hook) };
            None
        }
        (None, false) => None,
        (None, true) => {
            let instance = unsafe { GetModuleHandleW(None) }.ok()?;
            match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(instance.into()), 0) } {
                Ok(hook) => Some(hook),
                Err(error) => {
                    log::warn!("Couldn't watch mouse clicks for the favorites menu: {error}");
                    None
                }
            }
        }
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let signal = match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                let press = Press { x: info.pt.x, y: info.pt.y, time: info.time };
                let double = CLICKS.with(|clicks| {
                    clicks.try_borrow_mut().map(|mut clicks| clicks.press(press, limits())).unwrap_or(false)
                });
                Some(Signal { button: Button::Left, stage: if double { Stage::Clicked } else { Stage::Pressed } })
            }
            WM_MBUTTONDOWN => Some(Signal { button: Button::Middle, stage: Stage::Pressed }),
            WM_MBUTTONUP => Some(Signal { button: Button::Middle, stage: Stage::Clicked }),
            _ => None,
        };
        if let Some(signal) = signal {
            post(signal, info.pt);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

pub(super) fn limits() -> Limits {
    unsafe {
        Limits {
            time: GetDoubleClickTime(),
            width: GetSystemMetrics(SM_CXDOUBLECLK),
            height: GetSystemMetrics(SM_CYDOUBLECLK),
        }
    }
}

fn post(signal: Signal, at: POINT) {
    let Some((target, message)) = TARGET.with(|cell| cell.try_borrow().ok().map(|target| *target)) else { return };
    if target.is_invalid() {
        return;
    }
    let _ = unsafe { PostMessageW(Some(target), message, WPARAM(signal.code()), pack(at)) };
}

fn pack(at: POINT) -> LPARAM {
    LPARAM(((at.y as i16 as u16 as u32) << 16 | at.x as i16 as u16 as u32) as _)
}

pub(super) fn point(lparam: LPARAM) -> POINT {
    let packed = lparam.0 as u32;
    POINT { x: i32::from(packed as u16 as i16), y: i32::from((packed >> 16) as u16 as i16) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { time: 500, width: 4, height: 4 };

    fn press(x: i32, y: i32, time: u32) -> Press {
        Press { x, y, time }
    }

    #[test]
    fn two_quick_presses_on_one_spot_are_a_double_click() {
        let mut clicks = Clicks::default();
        assert!(!clicks.press(press(100, 100, 1_000), LIMITS));
        assert!(clicks.press(press(101, 99, 1_300), LIMITS));
        assert!(!clicks.press(press(101, 99, 1_400), LIMITS));
    }

    #[test]
    fn slow_or_distant_presses_are_not() {
        let mut clicks = Clicks::default();
        assert!(!clicks.press(press(100, 100, 1_000), LIMITS));
        assert!(!clicks.press(press(100, 100, 1_600), LIMITS));
        assert!(!clicks.press(press(110, 100, 1_700), LIMITS));
        assert!(clicks.press(press(110, 100, 1_750), LIMITS));
    }

    #[test]
    fn signals_survive_posting() {
        for signal in Signal::ALL {
            assert_eq!(Signal::from_message(WPARAM(signal.code())), Some(signal));
        }
        assert_eq!(Signal::from_message(WPARAM(Signal::ALL.len())), None);
    }

    #[test]
    fn points_survive_packing() {
        let at = POINT { x: -1_920, y: 1_080 };
        assert_eq!(point(pack(at)), at);
    }
}
