use super::{keyboard_proc, win_event};
use anyhow::{Context, Result};
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZESTART, HHOOK, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};

const HOST_EVENTS: [(u32, u32); 3] = [
    (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZESTART),
    (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
    (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
];

pub(super) struct Hooks {
    keyboard: HHOOK,
    foreground: Option<HWINEVENTHOOK>,
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.keyboard);
        }
        unhook(self.foreground.take());
    }
}

pub(super) fn install(instance: HINSTANCE) -> Result<Hooks> {
    let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), Some(instance), 0) }
        .context("installing the keyboard hook")?;
    let foreground = hook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND);
    Ok(Hooks { keyboard, foreground })
}

#[derive(Default)]
pub(super) struct HostEvents(Vec<HWINEVENTHOOK>);

impl HostEvents {
    pub(super) fn follow(&mut self, host: bool) {
        match (host, self.0.is_empty()) {
            (true, true) => self.0 = HOST_EVENTS.iter().filter_map(|&(first, last)| hook(first, last)).collect(),
            (false, false) => unhook(self.0.drain(..)),
            (true, false) | (false, true) => {}
        }
    }
}

fn hook(first: u32, last: u32) -> Option<HWINEVENTHOOK> {
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let hook = unsafe { SetWinEventHook(first, last, None, Some(win_event), 0, 0, flags) };
    (!hook.is_invalid()).then_some(hook)
}

fn unhook(hooks: impl IntoIterator<Item = HWINEVENTHOOK>) {
    for hook in hooks {
        let _ = unsafe { UnhookWinEvent(hook) };
    }
}
