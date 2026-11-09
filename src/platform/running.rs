use super::apps::APPS_FOLDER;
use super::icons::app_shortcuts;
use super::shell::{LinkReader, class_name, top_level_windows};
use super::{ensure_com, window};
use anyhow::{Result, bail};
use parking_lot::Mutex;
use std::path::Path;
use std::time::SystemTime;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, PROPERTYKEY};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, SW_RESTORE, ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::{GUID, PWSTR};

const PATH_LENGTH: usize = 1024;
const APP_ID_LENGTH: usize = 256;
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];
const APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3), pid: 5 };

static TARGETS: Mutex<Vec<Target>> = Mutex::new(Vec::new());

struct Target {
    shortcut: String,
    modified: Option<SystemTime>,
    target: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunningApp {
    window: isize,
}

impl RunningApp {
    fn hwnd(self) -> HWND {
        HWND(self.window as _)
    }
}

pub(crate) fn running_apps() -> Vec<(String, RunningApp)> {
    ensure_com();
    let mut running = Vec::new();
    for window in top_level_windows() {
        let hwnd = HWND(window as _);
        if !switchable(hwnd) {
            continue;
        }
        let instance = RunningApp { window };
        if let Some(path) = process_path(hwnd) {
            running.push((path.to_lowercase(), instance));
        }
        if let Some(id) = window_app_id(hwnd).or_else(|| process_app_id(hwnd)) {
            running.push((format!("{APPS_FOLDER}{id}").to_lowercase(), instance));
        }
    }
    let shortcuts: Vec<(String, RunningApp)> = shortcut_targets()
        .into_iter()
        .filter_map(|(shortcut, target)| {
            let (_, instance) = running.iter().find(|(path, _)| *path == target)?;
            Some((shortcut, *instance))
        })
        .collect();
    running.extend(shortcuts);
    running
}

pub(crate) fn switch_to(instance: RunningApp) -> Result<()> {
    let hwnd = instance.hwnd();
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        bail!("the window is gone");
    }
    if unsafe { IsIconic(hwnd) }.as_bool() {
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    window::bring_to_front(hwnd);
    Ok(())
}

pub(super) fn process_id(hwnd: HWND) -> u32 {
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    process
}

pub(super) fn process_path(hwnd: HWND) -> Option<String> {
    with_process(hwnd, |handle| {
        let mut buffer = [0u16; PATH_LENGTH];
        let mut length = buffer.len() as u32;
        unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length) }
            .ok()?;
        Some(String::from_utf16_lossy(buffer.get(..length as usize)?))
    })
}

fn with_process<T>(hwnd: HWND, read: impl FnOnce(HANDLE) -> Option<T>) -> Option<T> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id(hwnd)) }.ok()?;
    let read = read(handle);
    unsafe {
        let _ = CloseHandle(handle);
    }
    read
}

fn switchable(hwnd: HWND) -> bool {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let owned = unsafe { GetWindow(hwnd, GW_OWNER) }.is_ok_and(|owner| !owner.is_invalid());
    let listed = style & WS_EX_APPWINDOW.0 != 0 || (style & WS_EX_TOOLWINDOW.0 == 0 && !owned);
    listed
        && unsafe { IsWindowVisible(hwnd) }.as_bool()
        && unsafe { GetWindowTextLengthW(hwnd) } > 0
        && !cloaked(hwnd)
        && !window::is_ours(hwnd)
        && !DESKTOP_CLASSES.contains(&class_name(hwnd).as_str())
}

fn cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let read = unsafe {
        DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size_of::<u32>() as u32)
    };
    read.is_ok() && cloaked != 0
}

fn window_app_id(hwnd: HWND) -> Option<String> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(hwnd) }.ok()?;
    let value = unsafe { store.GetValue(&APP_ID) }.ok()?;
    let text = unsafe { PropVariantToStringAlloc(&value) }.ok()?;
    let id = unsafe { text.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(text.0 as _)) };
    id.filter(|id| id.contains(super::apps::PACKAGED))
}

fn process_app_id(hwnd: HWND) -> Option<String> {
    with_process(hwnd, |handle| {
        let mut buffer = [0u16; APP_ID_LENGTH];
        let mut length = buffer.len() as u32;
        unsafe { GetApplicationUserModelId(handle, &mut length, Some(PWSTR(buffer.as_mut_ptr()))) }.ok().ok()?;
        let text = buffer.get(..length as usize)?;
        let end = text.iter().position(|&unit| unit == 0).unwrap_or(text.len());
        Some(String::from_utf16_lossy(text.get(..end)?)).filter(|id| !id.is_empty())
    })
}

fn shortcut_targets() -> Vec<(String, String)> {
    let mut known = std::mem::take(&mut *TARGETS.lock());
    let mut reader = None;
    let targets: Vec<Target> = app_shortcuts()
        .into_iter()
        .map(|shortcut| {
            let modified = std::fs::metadata(&shortcut).and_then(|metadata| metadata.modified()).ok();
            let target = match known.iter().position(|known| known.shortcut == shortcut && known.modified == modified) {
                Some(position) => known.swap_remove(position).target,
                None => reader
                    .get_or_insert_with(LinkReader::new)
                    .as_ref()
                    .and_then(|reader| reader.target(Path::new(&shortcut)))
                    .map(|(path, _)| path.to_lowercase()),
            };
            Target { shortcut, modified, target }
        })
        .collect();
    let pairs =
        targets.iter().filter_map(|known| Some((known.shortcut.to_lowercase(), known.target.clone()?))).collect();
    *TARGETS.lock() = targets;
    pairs
}
