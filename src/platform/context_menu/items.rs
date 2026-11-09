use super::{Menu, extended};
use crate::platform::icons::{Icon, read_image};
use anyhow::{Context, Result};
use futures::channel::oneshot;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMenuItemCount, GetMenuItemInfoW, GetMessageW,
    HBMMENU_CALLBACK, HMENU, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR, MIIM_BITMAP, MIIM_CHECKMARKS, MIIM_FTYPE,
    MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU, MSG, PostMessageW, RegisterClassW, TranslateMessage, WM_APP,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, PWSTR, w};

const CLASS: PCWSTR = w!("Vestigo.shell-menu");
const WAKE: u32 = WM_APP + 1;
const TITLE_LENGTH: usize = 256;
const LAST_PREDEFINED_BITMAP: isize = 11;
const OPAQUE: u8 = u8::MAX;

pub(crate) struct ShellItem {
    pub(crate) title: String,
    pub(crate) icon: Option<Icon>,
    pub(crate) submenu: bool,
    pub(crate) trail: Vec<u32>,
    pub(crate) separated: bool,
}

enum Request {
    Prepare { path: String },
    Load { path: String, trail: Vec<u32>, reply: oneshot::Sender<Vec<ShellItem>> },
    Run { path: String, trail: Vec<u32> },
}

struct Loaded {
    path: String,
    menu: Menu,
    opened: Vec<HMENU>,
}

struct Prepared {
    path: String,
    menu: Menu,
}

#[derive(Default)]
struct Menus {
    loaded: Option<Loaded>,
    prepared: Option<Prepared>,
}

impl Menus {
    fn take(&mut self, owner: HWND, path: &str) -> Result<Menu> {
        let ready = self.prepared.take_if(|prepared| prepared.path == path && prepared.menu.extended == extended());
        match ready {
            Some(prepared) => Ok(prepared.menu),
            None => Menu::of(owner, path),
        }
    }
}

static REQUESTS: OnceLock<Sender<Request>> = OnceLock::new();
static WINDOW: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn load(path: String, trail: Vec<u32>) -> oneshot::Receiver<Vec<ShellItem>> {
    let (reply, receiver) = oneshot::channel();
    request(Request::Load { path, trail, reply });
    receiver
}

pub(crate) fn prepare(path: String) {
    request(Request::Prepare { path });
}

pub(crate) fn run(path: String, trail: Vec<u32>) {
    request(Request::Run { path, trail });
}

fn request(request: Request) {
    let sender = REQUESTS.get_or_init(|| {
        let (sender, receiver) = channel();
        if let Err(error) = std::thread::Builder::new().name("shell menu".into()).spawn(move || serve(receiver)) {
            log::warn!("Couldn't start the shell menu thread: {error}");
        }
        sender
    });
    if sender.send(request).is_err() {
        log::warn!("The shell menu thread is gone");
        return;
    }
    let hwnd = HWND(WINDOW.load(Ordering::Acquire) as _);
    if !hwnd.is_invalid() {
        let _ = unsafe { PostMessageW(Some(hwnd), WAKE, WPARAM(0), LPARAM(0)) };
    }
}

fn serve(receiver: Receiver<Request>) {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            log::warn!("{error:#}");
            return;
        }
    };
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    let mut menus = Menus::default();
    drain(&receiver, hwnd, &mut menus);
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        if message.message == WAKE {
            drain(&receiver, hwnd, &mut menus);
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn create_window() -> Result<HWND> {
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.context("reading the module handle")?.into();
    let class =
        WNDCLASSW { lpfnWndProc: Some(menu_proc), hInstance: instance, lpszClassName: CLASS, ..Default::default() };
    if unsafe { RegisterClassW(&class) } == 0 {
        anyhow::bail!("registering the shell menu window class failed");
    }
    unsafe { CreateWindowExW(WS_EX_TOOLWINDOW, CLASS, None, WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None) }
        .context("creating the shell menu window")
}

unsafe extern "system" fn menu_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn drain(receiver: &Receiver<Request>, owner: HWND, menus: &mut Menus) {
    loop {
        let requests: Vec<Request> = receiver.try_iter().collect();
        if requests.is_empty() {
            return;
        }
        serve_batch(requests, owner, menus);
    }
}

fn serve_batch(requests: Vec<Request>, owner: HWND, menus: &mut Menus) {
    let latest = requests.iter().rposition(|request| !matches!(request, Request::Run { .. }));
    for (at, request) in requests.into_iter().enumerate() {
        match request {
            Request::Prepare { path } if Some(at) == latest => prepare_menu(owner, menus, path),
            Request::Prepare { .. } => {}
            Request::Load { path, trail, reply } => {
                let items = load_items(owner, menus, path, &trail).unwrap_or_else(|error| {
                    log::warn!("{error:#}");
                    Vec::new()
                });
                let _ = reply.send(items);
            }
            Request::Run { path, trail } => {
                if let Err(error) = run_item(menus.loaded.as_ref(), &path, &trail, owner) {
                    log::warn!("{error:#}");
                }
            }
        }
    }
}

fn prepare_menu(owner: HWND, menus: &mut Menus, path: String) {
    if menus.prepared.as_ref().is_some_and(|prepared| prepared.path == path) {
        return;
    }
    menus.prepared = None;
    match Menu::of(owner, &path) {
        Ok(menu) => menus.prepared = Some(Prepared { path, menu }),
        Err(error) => log::debug!("{error:#}"),
    }
}

fn load_items(owner: HWND, menus: &mut Menus, path: String, trail: &[u32]) -> Result<Vec<ShellItem>> {
    if trail.is_empty() || menus.loaded.as_ref().is_none_or(|known| known.path != path) {
        menus.loaded = None;
        let menu = menus.take(owner, &path)?;
        menu.open(menu.popup, 0);
        menus.loaded = Some(Loaded { opened: vec![menu.popup], menu, path });
    }
    let Some(known) = menus.loaded.as_mut() else { return Ok(Vec::new()) };
    let mut popup = known.menu.popup;
    for &position in trail {
        let submenu = entry(popup, position)?.hSubMenu;
        if submenu.is_invalid() {
            anyhow::bail!("menu item {position} of {} has no submenu", known.path);
        }
        if !known.opened.contains(&submenu) {
            known.menu.open(submenu, position);
            known.opened.push(submenu);
        }
        popup = submenu;
    }
    Ok(items(popup, trail))
}

fn run_item(loaded: Option<&Loaded>, path: &str, trail: &[u32], owner: HWND) -> Result<()> {
    let known = loaded.filter(|known| known.path == path).context("the context menu is no longer loaded")?;
    let (&last, parents) = trail.split_last().context("no menu item to run")?;
    let mut popup = known.menu.popup;
    for &position in parents {
        popup = entry(popup, position)?.hSubMenu;
    }
    let command = entry(popup, last)?.wID;
    let mut point = POINT::default();
    let _ = unsafe { GetCursorPos(&mut point) };
    known.menu.invoke(command, point, owner)
}

fn entry(popup: HMENU, position: u32) -> Result<MENUITEMINFOW> {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_ID | MIIM_SUBMENU,
        ..Default::default()
    };
    unsafe { GetMenuItemInfoW(popup, position, true, &mut info) }.context("reading the menu item")?;
    Ok(info)
}

fn items(popup: HMENU, trail: &[u32]) -> Vec<ShellItem> {
    let count = u32::try_from(unsafe { GetMenuItemCount(Some(popup)) }).unwrap_or(0);
    let mut items: Vec<ShellItem> = Vec::new();
    let mut separated = false;
    for position in 0..count {
        match item(popup, position, trail) {
            Some(Entry::Item(mut item)) => {
                item.separated = separated && !items.is_empty();
                separated = false;
                items.push(item);
            }
            Some(Entry::Separator) => separated = true,
            None => {}
        }
    }
    items
}

enum Entry {
    Item(ShellItem),
    Separator,
}

fn item(popup: HMENU, position: u32, trail: &[u32]) -> Option<Entry> {
    let mut text = [0u16; TITLE_LENGTH];
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE | MIIM_STATE | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP | MIIM_CHECKMARKS,
        dwTypeData: PWSTR(text.as_mut_ptr()),
        cch: (TITLE_LENGTH - 1) as u32,
        ..Default::default()
    };
    unsafe { GetMenuItemInfoW(popup, position, true, &mut info) }.ok()?;
    if info.fType.contains(MFT_SEPARATOR) {
        return Some(Entry::Separator);
    }
    if info.fState.contains(MFS_DISABLED) {
        return None;
    }
    let title = label(&String::from_utf16_lossy(text.get(..info.cch as usize)?));
    if title.is_empty() {
        return None;
    }
    let icon = [info.hbmpItem, info.hbmpUnchecked]
        .into_iter()
        .find(|&bitmap| drawn(bitmap))
        .and_then(read_image)
        .map(straight);
    let trail = trail.iter().copied().chain([position]).collect();
    Some(Entry::Item(ShellItem { title, icon, submenu: !info.hSubMenu.is_invalid(), trail, separated: false }))
}

fn drawn(bitmap: HBITMAP) -> bool {
    !(HBMMENU_CALLBACK.0 as isize..=LAST_PREDEFINED_BITMAP).contains(&(bitmap.0 as isize))
}

fn straight(mut icon: Icon) -> Icon {
    for pixel in icon.bgra.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        if alpha > 0 && alpha < u16::from(OPAQUE) {
            for channel in &mut pixel[..3] {
                *channel = (u16::from(*channel) * u16::from(OPAQUE) / alpha).min(u16::from(OPAQUE)) as u8;
            }
        }
    }
    icon
}

fn label(text: &str) -> String {
    let text = text.split('\t').next().unwrap_or_default();
    let mut label = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '&' => label.extend(chars.next()),
            _ => label.push(ch),
        }
    }
    label.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{HBITMAP, drawn, label};

    #[test]
    fn any_bitmap_handle_counts_except_the_predefined_ones() {
        let handle = |raw: isize| HBITMAP(raw as *mut _);
        assert!(drawn(handle(0x7705_10ff)));
        assert!(drawn(handle(0xffff_ffff_9a05_0d9a_u64 as isize)));
        assert!(!drawn(handle(0)));
        assert!(!drawn(handle(-1)));
        assert!(!drawn(handle(11)));
    }

    #[test]
    fn labels_drop_access_keys_and_shortcut_text() {
        assert_eq!(label("&Открыть"), "Открыть");
        assert_eq!(label("Save && &Exit\tCtrl+Q"), "Save & Exit");
        assert_eq!(label("Trailing&"), "Trailing");
        assert_eq!(label("  &  "), "");
    }
}
