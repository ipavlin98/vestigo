use super::watch::TRAY_MESSAGE;
use super::{Event, SearchEvent, send};
use crate::i18n::Language;
use parking_lot::Mutex;
use std::sync::OnceLock;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_RESPECT_QUIET_TIME, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIN_BALLOONUSERCLICK, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON, HMENU, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MENU_ITEM_FLAGS, MF_SEPARATOR, MF_STRING, PostMessageW, SM_CXSMICON, SM_CYSMICON,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_CONTEXTMENU, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP,
};
use windows::core::{HSTRING, PCWSTR};

const ICON_ID: u32 = 1;
const APP_ICON: usize = 1;

#[derive(Clone, Copy)]
enum Item {
    Search,
    Window,
    Settings,
    Quit,
}

impl Item {
    const ALL: [Self; 4] = [Self::Search, Self::Window, Self::Settings, Self::Quit];

    fn id(self) -> usize {
        self as usize + 1
    }

    fn event(self) -> Event {
        match self {
            Self::Search => SearchEvent::Toggle.into(),
            Self::Window => Event::OpenWindow,
            Self::Settings => Event::OpenSettings,
            Self::Quit => Event::Quit,
        }
    }

    fn label(self, language: Language) -> &'static str {
        let strings = language.strings();
        match self {
            Self::Search => strings.tray_open,
            Self::Window => strings.tray_window,
            Self::Settings => strings.tray_settings,
            Self::Quit => strings.tray_quit,
        }
    }
}

static LANGUAGE: Mutex<Language> = Mutex::new(Language::English);
static ICON: OnceLock<isize> = OnceLock::new();
static WINDOW: Mutex<Option<isize>> = Mutex::new(None);

pub(super) fn sync(language: Language) {
    *LANGUAGE.lock() = language;
    if let Some(hwnd) = *WINDOW.lock() {
        notify(HWND(hwnd as _), NIM_MODIFY);
    }
}

pub(super) fn add(hwnd: HWND) {
    *WINDOW.lock() = Some(hwnd.0 as isize);
    notify(hwnd, NIM_ADD);
}

pub(super) fn remove(hwnd: HWND) {
    *WINDOW.lock() = None;
    let data =
        NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: ICON_ID, ..Default::default() };
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

pub(super) fn on_message(hwnd: HWND, lparam: LPARAM) {
    match (lparam.0 as u32) & 0xffff {
        WM_LBUTTONUP => send(Event::OpenWindow),
        WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
        NIN_BALLOONUSERCLICK => send(Event::ShowUpdate),
        _ => {}
    }
}

pub(super) fn announce(title: &str, text: &str) -> bool {
    let Some(hwnd) = *WINDOW.lock() else { return false };
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(hwnd as _),
        uID: ICON_ID,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_RESPECT_QUIET_TIME,
        ..Default::default()
    };
    fill(&mut data.szInfoTitle, title);
    fill(&mut data.szInfo, text);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool()
}

fn notify(hwnd: HWND, action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY_MESSAGE,
        hIcon: icon(),
        ..Default::default()
    };
    fill(&mut data.szTip, LANGUAGE.lock().strings().tooltip);
    if !unsafe { Shell_NotifyIconW(action, &data) }.as_bool() {
        log::debug!("The tray icon update was refused");
    }
}

fn fill(field: &mut [u16], text: &str) {
    let text: Vec<u16> = text.encode_utf16().collect();
    let length = text.len().min(field.len().saturating_sub(1));
    if let (Some(target), Some(source)) = (field.get_mut(..length), text.get(..length)) {
        target.copy_from_slice(source);
    }
}

fn show_menu(hwnd: HWND) {
    let language = *LANGUAGE.lock();
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };
    for item in Item::ALL {
        if matches!(item, Item::Quit) {
            append(menu, MF_SEPARATOR, 0, "");
        }
        append(menu, MF_STRING, item.id(), item.label(language));
    }
    let mut cursor = Default::default();
    let command = unsafe {
        let _ = GetCursorPos(&mut cursor);
        let _ = SetForegroundWindow(hwnd);
        let command =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, cursor.x, cursor.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        command.0 as usize
    };
    if let Some(item) = Item::ALL.into_iter().find(|item| item.id() == command) {
        send(item.event());
    }
}

fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, label: &str) {
    unsafe {
        let _ = AppendMenuW(menu, flags, id, &HSTRING::from(label));
    }
}

fn icon() -> HICON {
    let handle = *ICON.get_or_init(|| load_icon().map_or(0, |icon| icon.0 as isize));
    HICON(handle as _)
}

fn load_icon() -> Option<HICON> {
    let module = unsafe { GetModuleHandleW(None) }.ok()?;
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
    let image =
        unsafe { LoadImageW(Some(module.into()), PCWSTR(APP_ICON as _), IMAGE_ICON, width, height, LR_DEFAULTCOLOR) };
    match image {
        Ok(image) => Some(HICON(image.0)),
        Err(error) => {
            log::warn!("Couldn't load the tray icon: {error}");
            None
        }
    }
}
