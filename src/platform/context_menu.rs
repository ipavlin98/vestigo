mod items;

pub(crate) use items::{ShellItem, load as load_shell_menu, prepare as prepare_shell_menu, run as run_shell_item};

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, IContextMenu,
    IContextMenu2, IContextMenu3, ILFree, IShellFolder, SHBindToParent, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{CreatePopupMenu, DestroyMenu, HMENU, SW_SHOWNORMAL, WM_INITMENUPOPUP};
use windows::core::{HSTRING, Interface, PCSTR, PCWSTR};

const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7FFF;
const UNICODE: u32 = 0x4000;

struct Item(*mut ITEMIDLIST);

impl Drop for Item {
    fn drop(&mut self) {
        unsafe { ILFree(Some(self.0)) };
    }
}

struct Menu {
    shell: IContextMenu,
    popup: HMENU,
    extended: bool,
}

impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.popup);
        }
    }
}

impl Menu {
    fn of(owner: HWND, path: &str) -> Result<Self> {
        let mut item = std::ptr::null_mut();
        unsafe { SHParseDisplayName(&HSTRING::from(path), None, &mut item, 0, None) }
            .with_context(|| format!("finding {path} in the shell"))?;
        let item = Item(item);
        let mut child = std::ptr::null_mut();
        let folder: IShellFolder =
            unsafe { SHBindToParent(item.0, Some(&mut child)) }.context("opening the parent folder")?;
        let shell: IContextMenu = unsafe { folder.GetUIObjectOf(owner, &[child.cast_const()], None) }
            .context("asking for the context menu")?;
        let popup = unsafe { CreatePopupMenu() }.context("creating the menu")?;
        let menu = Self { shell, popup, extended: extended() };
        let flags = if menu.extended { CMF_NORMAL | CMF_EXTENDEDVERBS } else { CMF_NORMAL };
        unsafe { menu.shell.QueryContextMenu(popup, 0, FIRST_COMMAND, LAST_COMMAND, flags) }
            .ok()
            .context("filling the context menu")?;
        Ok(menu)
    }

    fn open(&self, submenu: HMENU, position: u32) {
        let (wparam, lparam) = (WPARAM(submenu.0 as usize), LPARAM(position as isize));
        if forward(&self.shell, WM_INITMENUPOPUP, wparam, lparam).is_none() {
            log::debug!("The shell filled submenu {position} without being asked");
        }
    }

    fn invoke(&self, command: u32, point: POINT, owner: HWND) -> Result<()> {
        let Some(offset) = command.checked_sub(FIRST_COMMAND) else { return Ok(()) };
        let offset = offset as usize;
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: UNICODE | CMIC_MASK_PTINVOKE,
            hwnd: owner,
            lpVerb: PCSTR(offset as *const u8),
            lpVerbW: PCWSTR(offset as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: point,
            ..Default::default()
        };
        unsafe { self.shell.InvokeCommand((&info as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>()) }
            .context("running the menu command")
    }
}

fn extended() -> bool {
    unsafe { GetKeyState(i32::from(VK_SHIFT.0)) < 0 }
}

fn forward(menu: &IContextMenu, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if let Ok(third) = menu.cast::<IContextMenu3>() {
        let mut result = LRESULT(0);
        if unsafe { third.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }.is_ok() {
            return Some(result);
        }
    }
    let second = menu.cast::<IContextMenu2>().ok()?;
    unsafe { second.HandleMenuMsg(message, wparam, lparam) }.ok().map(|()| LRESULT(0))
}
