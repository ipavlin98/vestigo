use super::shell::{class_name, find_descendant, top_level_windows};
use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegGetValueW, RegSetKeyValueW};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_COMMAND};
use windows::core::{PCWSTR, w};

const ADVANCED: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced");
const HIDDEN: PCWSTR = w!("Hidden");
const SHOWN: u32 = 1;
const CONCEALED: u32 = 2;
const REFRESH: usize = 0x7103;
const VIEW: &str = "SHELLDLL_DefView";
const VIEW_HOSTS: [&str; 4] = ["CabinetWClass", "ExploreWClass", "Progman", "WorkerW"];

pub(crate) fn hidden_files_shown() -> bool {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            ADVANCED,
            HIDDEN,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    read.is_ok() && value == SHOWN
}

pub(crate) fn show_hidden_files(shown: bool) -> Result<()> {
    let value = if shown { SHOWN } else { CONCEALED };
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            ADVANCED,
            HIDDEN,
            REG_DWORD.0,
            Some((&value as *const u32).cast()),
            size_of::<u32>() as u32,
        )
    }
    .ok()
    .context("changing the hidden files setting")?;
    refresh_views();
    Ok(())
}

fn refresh_views() {
    for window in top_level_windows() {
        let window = HWND(window as _);
        if !VIEW_HOSTS.contains(&class_name(window).as_str()) {
            continue;
        }
        find_descendant(window, &mut |child| {
            if class_name(child) == VIEW {
                let _ = unsafe { PostMessageW(Some(child), WM_COMMAND, WPARAM(REFRESH), LPARAM(0)) };
            }
            false
        });
    }
}
