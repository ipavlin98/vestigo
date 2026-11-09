use super::files::Launch;
use super::{Host, HostKind, ensure_com};
use crate::settings::Visit;
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use std::cmp::Reverse;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{
    CLSCTX_ALL, CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile, STGM_READ,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Recent,
    FOLDERID_Screenshots, FOLDERID_Videos, Folder2, IShellFolderViewDual, IShellLinkW, IShellWindows, IWebBrowserApp,
    KF_FLAG_DEFAULT, SHFILEINFOW, SHGFI_DISPLAYNAME, SHGetFileInfoW, SHGetKnownFolderPath, SHOP_FILEPATH,
    SHObjectProperties, ShellLink, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetDlgCtrlID, GetDlgItem, GetParent, IDOK, IsWindowVisible,
    PostMessageW, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW, SetForegroundWindow, WM_COMMAND, WM_GETTEXT,
    WM_GETTEXTLENGTH, WM_SETTEXT,
};
use windows::core::{BOOL, BSTR, GUID, HSTRING, Interface, PCWSTR};

const FILE_NAME_CONTROL: i32 = 1148;
const FOLDER_NAME_CONTROL: i32 = 1152;
const SAVE_NAME_CONTROL: i32 = 1001;
const FILE_TYPE_CONTROL: i32 = 1136;
const RECENT_LINKS: usize = 40;
const PATH_LENGTH: usize = 1024;
const MESSAGE_TIMEOUT_MS: u32 = 1000;
const RESTORE_DELAY: Duration = Duration::from_millis(300);
const CLASS_LENGTH: usize = 64;
static RESOLVED: Mutex<Vec<Resolved>> = Mutex::new(Vec::new());

const NAMED_FOLDERS: [&GUID; 6] = [
    &FOLDERID_Desktop,
    &FOLDERID_Documents,
    &FOLDERID_Downloads,
    &FOLDERID_Pictures,
    &FOLDERID_Music,
    &FOLDERID_Videos,
];

pub(crate) fn open(path: &str) -> Result<()> {
    execute("open", path)
}

pub(crate) fn run_as_admin(path: &str) -> Result<()> {
    execute("runas", path)
}

fn execute(verb: &str, path: &str) -> Result<()> {
    let folder = Path::new(path).parent().filter(|folder| folder.is_dir()).and_then(Path::to_str).unwrap_or_default();
    Launch { file: path, arguments: "", folder, verb, show: SW_SHOWNORMAL }
        .start()
        .with_context(|| format!("the shell couldn't open {path}"))
}

pub(crate) fn reveal(path: &str) -> Result<()> {
    let arguments = format!("/select,\"{path}\"");
    Launch { file: "explorer.exe", arguments: &arguments, folder: "", verb: "open", show: SW_SHOWNORMAL }
        .directly()
        .with_context(|| format!("Explorer couldn't show {path}"))
}

pub(crate) fn properties(path: &str) -> Result<()> {
    if !unsafe { SHObjectProperties(None, SHOP_FILEPATH, &HSTRING::from(path), PCWSTR::null()) }.as_bool() {
        bail!("the shell couldn't show the properties of {path}");
    }
    Ok(())
}

pub(crate) fn explorer_folders() -> Vec<String> {
    ensure_com();
    let windows: IShellWindows = match unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_ALL) } {
        Ok(windows) => windows,
        Err(error) => {
            log::debug!("Couldn't connect to Explorer windows: {error}");
            return Vec::new();
        }
    };
    let count = unsafe { windows.Count() }.unwrap_or(0);
    let mut folders: Vec<(isize, String)> = (0..count)
        .filter_map(|position| {
            let browser = unsafe { windows.Item(&VARIANT::from(position)) }.ok()?.cast::<IWebBrowserApp>().ok()?;
            let hwnd = unsafe { browser.HWND() }.ok()?.0;
            let view = unsafe { browser.Document() }.ok()?.cast::<IShellFolderViewDual>().ok()?;
            let path = unsafe {
                view.Folder()
                    .and_then(|folder| folder.cast::<Folder2>())
                    .and_then(|folder| folder.Self_())
                    .and_then(|item| item.Path())
            }
            .ok()?;
            Some((hwnd, path.to_string())).filter(|(_, path)| is_file_system(path))
        })
        .collect();
    let order = top_level_windows();
    folders.sort_by_key(|(hwnd, _)| order.iter().position(|window| window == hwnd).unwrap_or(usize::MAX));
    let mut paths: Vec<String> = Vec::with_capacity(folders.len());
    for (_, path) in folders {
        if !paths.iter().any(|known| known.eq_ignore_ascii_case(&path)) {
            paths.push(path);
        }
    }
    paths
}

fn is_file_system(path: &str) -> bool {
    is_drive_path(path) || path.starts_with("\\\\")
}

fn is_drive_path(path: &str) -> bool {
    path.starts_with(|first: char| first.is_ascii_alphabetic()) && path.get(1..3) == Some(":\\")
}

pub(crate) fn host_folder(host: Host) -> Option<String> {
    ensure_com();
    if host.kind == HostKind::Desktop {
        return known_folder(&FOLDERID_Desktop).and_then(|path| path.to_str().map(str::to_owned));
    }
    let crumbs = find_descendant(host.hwnd(), &mut |child| {
        class_name(child) == "Breadcrumb Parent" && unsafe { IsWindowVisible(child) }.as_bool()
    })?;
    let bar = find_descendant(crumbs, &mut |child| class_name(child) == "ToolbarWindow32")?;
    let text = window_text(bar);
    address_path(&text).map(str::to_owned).or_else(|| named_address(&text, &named_folders()))
}

fn address_path(text: &str) -> Option<&str> {
    text.char_indices().filter_map(|(at, _)| text.get(at..)).find(|rest| is_file_system(rest)).map(str::trim_end)
}

fn named_address(text: &str, folders: &[(String, String)]) -> Option<String> {
    let label = text.split_once(':')?.1.trim();
    let (head, rest) = label.split_once('\\').unwrap_or((label, ""));
    let (_, path) = folders.iter().find(|(name, _)| name == head)?;
    Some(if rest.is_empty() { path.clone() } else { format!("{path}\\{rest}") })
}

fn named_folders() -> Vec<(String, String)> {
    NAMED_FOLDERS
        .iter()
        .filter_map(|id| {
            let path = known_folder(id)?.to_str()?.to_owned();
            Some((display_name(&path)?, path))
        })
        .collect()
}

fn display_name(path: &str) -> Option<String> {
    let mut info = SHFILEINFOW::default();
    let size = size_of::<SHFILEINFOW>() as u32;
    let found = unsafe {
        SHGetFileInfoW(&HSTRING::from(path), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut info), size, SHGFI_DISPLAYNAME)
    };
    if found == 0 {
        return None;
    }
    let name = info.szDisplayName;
    let length = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
    String::from_utf16(name.get(..length)?).ok().filter(|name| !name.is_empty())
}

pub(crate) fn recent_items() -> Vec<Visit> {
    ensure_com();
    let Some(folder) = known_folder(&FOLDERID_Recent) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&folder) else { return Vec::new() };
    let mut links: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let shortcut = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"));
            let modified = entry.metadata().ok()?.modified().ok()?;
            shortcut.then(|| (modified.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs()), path))
        })
        .collect();
    links.sort_by_key(|(at, _)| Reverse(*at));
    links.truncate(RECENT_LINKS);
    let mut known = std::mem::take(&mut *RESOLVED.lock());
    let mut reader = None;
    let mut resolved = Vec::with_capacity(links.len());
    for (at, shortcut) in links {
        let target = match known.iter().position(|known| known.shortcut == shortcut && known.at == at) {
            Some(position) => known.swap_remove(position).target,
            None => reader.get_or_insert_with(LinkReader::new).as_ref().and_then(|reader| reader.target(&shortcut)),
        };
        resolved.push(Resolved { shortcut, at, target });
    }
    let visits = resolved
        .iter()
        .filter_map(|known| {
            let (path, dir) = known.target.clone()?;
            Some(Visit { path, dir, at: known.at })
        })
        .collect();
    *RESOLVED.lock() = resolved;
    visits
}

pub(crate) fn shortcut_folder(shortcut: &str) -> Option<String> {
    ensure_com();
    let (path, dir) = LinkReader::new()?.target(Path::new(shortcut))?;
    dir.then_some(path)
}

struct Resolved {
    shortcut: PathBuf,
    at: u64,
    target: Option<(String, bool)>,
}

pub(super) struct LinkReader {
    link: IShellLinkW,
    file: IPersistFile,
}

impl LinkReader {
    pub(super) fn new() -> Option<Self> {
        let link: IShellLinkW = match unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) } {
            Ok(link) => link,
            Err(error) => {
                log::debug!("Couldn't read shortcuts: {error}");
                return None;
            }
        };
        let file = link.cast::<IPersistFile>().ok()?;
        Some(Self { link, file })
    }

    pub(super) fn target(&self, shortcut: &Path) -> Option<(String, bool)> {
        unsafe { self.file.Load(&HSTRING::from(shortcut.as_os_str()), STGM_READ) }.ok()?;
        let mut buffer = [0u16; PATH_LENGTH];
        unsafe { self.link.GetPath(&mut buffer, std::ptr::null_mut(), 0) }.ok()?;
        let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        let path = String::from_utf16_lossy(buffer.get(..length)?);
        if !is_drive_path(&path) {
            return None;
        }
        let dir = std::fs::metadata(&path).ok()?.is_dir();
        Some((path, dir))
    }
}

pub(crate) fn desktop_folder() -> Option<String> {
    ensure_com();
    known_folder(&FOLDERID_Desktop).and_then(|path| path.into_os_string().into_string().ok())
}

pub(crate) fn landing_folders() -> Vec<String> {
    ensure_com();
    [&FOLDERID_Downloads, &FOLDERID_Desktop, &FOLDERID_Screenshots]
        .into_iter()
        .filter_map(|id| known_folder(id)?.into_os_string().into_string().ok())
        .collect()
}

pub(crate) fn downloads_folder() -> Option<String> {
    ensure_com();
    known_folder(&FOLDERID_Downloads).and_then(|path| path.into_os_string().into_string().ok())
}

pub(crate) fn current_folder(host: Option<Host>) -> Option<String> {
    host.and_then(host_folder).or_else(|| explorer_folders().into_iter().next()).or_else(desktop_folder)
}

fn known_folder(id: &GUID) -> Option<PathBuf> {
    let path = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok()?;
    let folder = unsafe { path.to_string() }.ok().map(PathBuf::from);
    unsafe { CoTaskMemFree(Some(path.0 as _)) };
    folder
}

pub(super) fn top_level_windows() -> Vec<isize> {
    unsafe extern "system" fn visit(hwnd: HWND, state: LPARAM) -> BOOL {
        let windows = unsafe { &mut *(state.0 as *mut Vec<isize>) };
        windows.push(hwnd.0 as isize);
        BOOL(1)
    }
    let mut windows = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut windows as *mut Vec<isize> as isize));
    }
    windows
}

pub(crate) fn navigate_explorer(host: Host, path: &str) -> Result<()> {
    if host.kind == HostKind::Desktop {
        return open(path);
    }
    let windows: IShellWindows =
        unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_ALL) }.context("connecting to Explorer windows")?;
    let count = unsafe { windows.Count() }.context("counting Explorer windows")?;
    for position in 0..count {
        let Ok(item) = (unsafe { windows.Item(&VARIANT::from(position)) }) else { continue };
        let Ok(browser) = item.cast::<IWebBrowserApp>() else { continue };
        if unsafe { browser.HWND() }.is_ok_and(|hwnd| hwnd.0 == host.window) {
            let empty = VARIANT::default();
            let empty = Some(&empty as *const VARIANT);
            unsafe { browser.Navigate(&BSTR::from(path), empty, empty, empty, empty) }
                .with_context(|| format!("navigating Explorer to {path}"))?;
            focus_host(host);
            return Ok(());
        }
    }
    bail!("the Explorer window is gone")
}

pub(crate) fn choose_in_dialog(host: Host, path: &str, keep_name: bool) -> Result<()> {
    let dialog = host.hwnd();
    let (edit, picks) = name_field(dialog).context("the dialog has no file name field")?;
    let previous = window_text(edit);
    set_text(edit, chosen(path, picks))?;
    focus_host(host);
    let ok = unsafe { GetDlgItem(Some(dialog), IDOK.0) }.unwrap_or_default();
    unsafe { PostMessageW(Some(dialog), WM_COMMAND, WPARAM(IDOK.0 as usize), LPARAM(ok.0 as isize)) }
        .context("pressing the dialog's button")?;
    if keep_name {
        let edit = edit.0 as isize;
        std::thread::spawn(move || {
            std::thread::sleep(RESTORE_DELAY);
            if let Err(error) = set_text(HWND(edit as _), &previous) {
                log::debug!("Couldn't restore the file name: {error:#}");
            }
        });
    }
    Ok(())
}

pub(crate) fn focus_host(host: Host) {
    unsafe {
        let _ = SetForegroundWindow(host.hwnd());
    }
}

pub(super) fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; CLASS_LENGTH];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    buffer.get(..usize::try_from(length).unwrap_or(0)).map(String::from_utf16_lossy).unwrap_or_default()
}

pub(super) fn find_descendant(parent: HWND, predicate: &mut dyn FnMut(HWND) -> bool) -> Option<HWND> {
    struct Search<'a> {
        predicate: &'a mut dyn FnMut(HWND) -> bool,
        found: Option<HWND>,
    }
    unsafe extern "system" fn visit(child: HWND, state: LPARAM) -> BOOL {
        let search = unsafe { &mut *(state.0 as *mut Search<'_>) };
        if (search.predicate)(child) {
            search.found = Some(child);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut search = Search { predicate, found: None };
    unsafe {
        let _ = EnumChildWindows(Some(parent), Some(visit), LPARAM(&mut search as *mut Search<'_> as isize));
    }
    search.found
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Picks {
    Files,
    Saves,
    Folders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Open(Types),
    Save(Types),
    Browse,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Types {
    Any,
    Only(Vec<String>),
}

impl Types {
    fn parse(filter: &str) -> Self {
        let mut extensions = Vec::new();
        for pattern in filter.split(|ch: char| matches!(ch, ';' | ',' | '(' | ')') || ch.is_whitespace()) {
            if pattern == "*" || pattern == "*.*" {
                return Self::Any;
            }
            let extension = pattern.strip_prefix("*.").filter(|rest| !rest.is_empty() && !rest.contains(['*', '?']));
            extensions.extend(extension.map(str::to_lowercase));
        }
        if extensions.is_empty() { Self::Any } else { Self::Only(extensions) }
    }

    pub(crate) fn admit(&self, path: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Only(extensions) => {
                let path = path.to_lowercase();
                extensions
                    .iter()
                    .any(|extension| path.strip_suffix(extension.as_str()).is_some_and(|rest| rest.ends_with('.')))
            }
        }
    }
}

pub(crate) fn dialog_purpose(host: Host) -> Purpose {
    let dialog = host.hwnd();
    match name_field(dialog).map(|(_, picks)| picks) {
        Some(Picks::Saves) => Purpose::Save(saved_types(dialog)),
        Some(Picks::Files) => find_descendant(dialog, &mut |child| unsafe {
            GetDlgCtrlID(child) == FILE_TYPE_CONTROL && IsWindowVisible(child).as_bool()
        })
        .map_or(Purpose::Browse, |types| Purpose::Open(Types::parse(&window_text(types)))),
        Some(Picks::Folders) | None => Purpose::Browse,
    }
}

fn saved_types(dialog: HWND) -> Types {
    let mut types = Types::Any;
    find_descendant(dialog, &mut |child| {
        let chooser = class_name(child) == "ComboBox"
            && unsafe { GetParent(child) }.is_ok_and(|parent| class_name(parent) == "FloatNotifySink")
            && find_descendant(child, &mut |inner| class_name(inner) == "Edit").is_none();
        if chooser {
            types = Types::parse(&window_text(child));
        }
        matches!(types, Types::Only(_))
    });
    types
}

pub(super) fn name_field(dialog: HWND) -> Option<(HWND, Picks)> {
    if let Some(field) = find_descendant(dialog, &mut |child| unsafe { GetDlgCtrlID(child) } == FILE_NAME_CONTROL) {
        if class_name(field) == "Edit" {
            return Some((field, Picks::Files));
        }
        return find_descendant(field, &mut |child| class_name(child) == "Edit").map(|edit| (edit, Picks::Files));
    }
    if let Some(field) = find_descendant(dialog, &mut |child| is_save_name(child)) {
        return Some((field, Picks::Saves));
    }
    let folder = find_descendant(dialog, &mut |child| unsafe {
        GetDlgCtrlID(child) == FOLDER_NAME_CONTROL && IsWindowVisible(child).as_bool()
    })?;
    (class_name(folder) == "Edit").then_some((folder, Picks::Folders))
}

fn is_save_name(child: HWND) -> bool {
    let id = unsafe { GetDlgCtrlID(child) };
    id == SAVE_NAME_CONTROL
        && class_name(child) == "Edit"
        && unsafe { GetParent(child) }.is_ok_and(|parent| class_name(parent) == "ComboBox")
}

fn chosen(path: &str, picks: Picks) -> &str {
    match picks {
        Picks::Files | Picks::Saves => path,
        Picks::Folders if Path::new(path).is_dir() => path,
        Picks::Folders => Path::new(path).parent().and_then(Path::to_str).unwrap_or(path),
    }
}

fn window_text(hwnd: HWND) -> String {
    let mut length = 0usize;
    unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXTLENGTH,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            Some(&mut length),
        )
    };
    let mut buffer = vec![0u16; length + 1];
    let mut copied = 0usize;
    unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            Some(&mut copied),
        )
    };
    buffer.get(..copied).map(String::from_utf16_lossy).unwrap_or_default()
}

fn set_text(hwnd: HWND, text: &str) -> Result<()> {
    let text = HSTRING::from(text);
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            None,
        )
    };
    if sent.0 == 0 {
        bail!("the dialog didn't accept the path");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_dialog_gets_the_folder_of_a_chosen_file() {
        let folder = std::env::temp_dir().join(format!("vestigo-chosen-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("Папка")).unwrap();
        let root = folder.to_str().unwrap();
        let inner = format!("{root}\\Папка");
        let file = format!("{inner}\\отчёт.txt");
        assert_eq!(chosen(&file, Picks::Folders), inner);
        assert_eq!(chosen(&inner, Picks::Folders), inner);
        assert_eq!(chosen(&file, Picks::Files), file);
        assert_eq!(chosen(&file, Picks::Saves), file);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_dialog_filter_lists_the_extensions_it_opens() {
        let images = Types::parse("Изображения (*.PNG;*.jpg, *.tar.gz)");
        assert_eq!(images, Types::Only(vec!["png".into(), "jpg".into(), "tar.gz".into()]));
        assert!(images.admit("C:\\Фото\\Море.Png"));
        assert!(images.admit("D:\\backup.tar.gz"));
        assert!(!images.admit("C:\\Фото\\png"));
        assert!(!images.admit("C:\\notes.png.txt"));
        assert!(!images.admit("C:\\archive.gz"));
    }

    #[test]
    fn a_dialog_filter_without_patterns_or_with_a_wildcard_opens_anything() {
        assert_eq!(Types::parse("All files (*.*)"), Types::Any);
        assert_eq!(Types::parse("Images (*.png;*)"), Types::Any);
        assert_eq!(Types::parse("Документы Word"), Types::Any);
        assert_eq!(Types::parse(""), Types::Any);
    }

    #[test]
    fn the_address_bar_gives_the_folder_in_any_language() {
        assert_eq!(address_path("Адрес: C:\\Users\\Alex\\Загрузки"), Some("C:\\Users\\Alex\\Загрузки"));
        assert_eq!(address_path("Address: \\\\server\\share "), Some("\\\\server\\share"));
        assert_eq!(address_path("Адрес: Этот компьютер"), None);
        assert_eq!(address_path("Address: Libraries: Documents"), None);
    }

    #[test]
    fn known_folders_shown_by_name_resolve_to_their_paths() {
        let folders = [
            ("Рабочий стол".to_owned(), "C:\\Users\\Alex\\Desktop".to_owned()),
            ("Документы".to_owned(), "C:\\Users\\Alex\\Documents".to_owned()),
        ];
        assert_eq!(named_address("Адрес: Документы", &folders).as_deref(), Some("C:\\Users\\Alex\\Documents"));
        assert_eq!(
            named_address("Address: Рабочий стол\\проект", &folders).as_deref(),
            Some("C:\\Users\\Alex\\Desktop\\проект")
        );
        assert_eq!(named_address("Adresse : Документы ", &folders).as_deref(), Some("C:\\Users\\Alex\\Documents"));
        assert_eq!(named_address("Адрес: Этот компьютер", &folders), None);
        assert_eq!(named_address("Адрес: Документы общие", &folders), None);
    }
}
