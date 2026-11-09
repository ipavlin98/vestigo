use super::{Host, HostKind, ensure_com};
use crate::settings::Program;
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL, POINT};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, IServiceProvider,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    CSIDL_DESKTOP, DROPFILES, FO_DELETE, FOF_ALLOWUNDO, IShellBrowser, IShellDispatch2, IShellFolderViewDual,
    IShellWindows, IWebBrowserApp, OAIF_ALLOW_REGISTRATION, OAIF_EXEC, OPENASINFO, SEE_MASK_NOASYNC,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHFILEOPSTRUCTW, SHFileOperationW, SHOpenWithDialog,
    SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellExecuteExW, ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetShellWindow, GetWindowThreadProcessId, SHOW_WINDOW_CMD, SW_HIDE, SW_SHOWNORMAL,
};
use windows::core::{BOOL, BSTR, HSTRING, Interface, PCWSTR};

pub(super) const DROP_EFFECT: &str = "Preferred DropEffect";
pub(super) const COPY_EFFECT: u32 = 1;
const MOVE_EFFECT: u32 = 2;
const DEFAULT_EXTENSIONS: &str = ".COM;.EXE;.BAT;.CMD";

pub(crate) fn on_shell_thread(job: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new().name("shell command".into()).spawn(move || {
        let apartment = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        job();
        if apartment.is_ok() {
            unsafe { CoUninitialize() };
        }
    });
    if let Err(error) = spawned {
        log::warn!("Couldn't start a shell command: {error}");
    }
}

pub(crate) fn run(program: &Program) -> Result<()> {
    let file = resolve(&program.path);
    let launch = Launch {
        file: &file.to_string_lossy(),
        arguments: &program.arguments,
        folder: &program.folder,
        verb: if program.admin { "runas" } else { "" },
        show: if program.hidden { SW_HIDE } else { SW_SHOWNORMAL },
    };
    launch.start().with_context(|| format!("starting {}", program.path))
}

pub(super) struct Launch<'a> {
    pub(super) file: &'a str,
    pub(super) arguments: &'a str,
    pub(super) folder: &'a str,
    pub(super) verb: &'a str,
    pub(super) show: SHOW_WINDOW_CMD,
}

impl Launch<'_> {
    pub(super) fn start(&self) -> Result<()> {
        if self.verb != "runas" && super::is_elevated() {
            match self.through_explorer() {
                Ok(()) => return Ok(()),
                Err(error) => log::warn!("Explorer couldn't start {} unelevated: {error:#}", self.file),
            }
        }
        self.directly()
    }

    pub(super) fn directly(&self) -> Result<()> {
        let file = HSTRING::from(self.file);
        let arguments = HSTRING::from(self.arguments);
        let folder = HSTRING::from(self.folder);
        let verb = HSTRING::from(self.verb);
        let text = |value: &HSTRING| if value.is_empty() { PCWSTR::null() } else { PCWSTR(value.as_ptr()) };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS,
            lpVerb: text(&verb),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: text(&arguments),
            lpDirectory: text(&folder),
            nShow: self.show.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }?;
        let process = info.hProcess;
        if !process.is_invalid() {
            let _ = unsafe { CloseHandle(process) };
        }
        Ok(())
    }

    fn through_explorer(&self) -> Result<()> {
        ensure_com();
        let windows: IShellWindows =
            unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_ALL) }.context("connecting to Explorer")?;
        let desktop = VARIANT::from(CSIDL_DESKTOP as i32);
        let mut hwnd = 0;
        let found =
            unsafe { windows.FindWindowSW(&desktop, &VARIANT::default(), SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH) }
                .context("finding the desktop")?;
        let browser: IShellBrowser = unsafe { found.cast::<IServiceProvider>()?.QueryService(&SID_STopLevelBrowser) }
            .context("reaching the desktop browser")?;
        let view: IShellFolderViewDual =
            unsafe { browser.QueryActiveShellView()?.GetItemObject::<IDispatch>(SVGIO_BACKGROUND)?.cast() }
                .context("reaching the desktop view")?;
        let shell: IShellDispatch2 = unsafe { view.Application()? }.cast().context("reaching the shell")?;
        let mut explorer = 0;
        if unsafe { GetWindowThreadProcessId(GetShellWindow(), Some(&mut explorer)) } != 0 {
            let _ = unsafe { AllowSetForegroundWindow(explorer) };
        }
        let text = |value: &str| VARIANT::from(BSTR::from(value));
        unsafe {
            shell.ShellExecute(
                &BSTR::from(self.file),
                &text(self.arguments),
                &text(self.folder),
                &text(self.verb),
                &VARIANT::from(self.show.0),
            )
        }
        .context("asking Explorer to start it")
    }
}

fn resolve(path: &str) -> PathBuf {
    let given = Path::new(path);
    if path.contains(['\\', '/']) || given.extension().is_some() {
        return given.to_path_buf();
    }
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| DEFAULT_EXTENSIONS.to_owned());
    let folders: Vec<PathBuf> =
        std::env::var_os("PATH").map(|paths| std::env::split_paths(&paths).collect()).unwrap_or_default();
    folders
        .iter()
        .flat_map(|folder| extensions.split(';').map(move |extension| folder.join(format!("{path}{extension}"))))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| given.to_path_buf())
}

pub(crate) fn open_with(path: &str) -> Result<()> {
    let file = HSTRING::from(path);
    let info = OPENASINFO {
        pcszFile: PCWSTR(file.as_ptr()),
        pcszClass: PCWSTR::null(),
        oaifInFlags: OAIF_EXEC | OAIF_ALLOW_REGISTRATION,
    };
    unsafe { SHOpenWithDialog(None, &info) }.with_context(|| format!("choosing a program for {path}"))
}

pub(crate) fn put_files_on_clipboard(paths: &[String], cut: bool) -> Result<()> {
    let header = size_of::<DROPFILES>();
    let names: Vec<u16> = paths.iter().flat_map(|path| path.encode_utf16().chain([0])).chain([0]).collect();
    let drop = global(header + names.len() * 2, |memory| unsafe {
        let files = DROPFILES { pFiles: header as u32, pt: POINT::default(), fNC: BOOL(0), fWide: BOOL(1) };
        memory.cast::<DROPFILES>().write_unaligned(files);
        std::ptr::copy_nonoverlapping(names.as_ptr(), memory.add(header).cast::<u16>(), names.len());
    })?;
    let effect = if cut { MOVE_EFFECT } else { COPY_EFFECT };
    let effect = global(size_of::<u32>(), |memory| unsafe { memory.cast::<u32>().write_unaligned(effect) })?;
    unsafe {
        if let Err(error) = OpenClipboard(None) {
            let _ = GlobalFree(Some(drop));
            let _ = GlobalFree(Some(effect));
            return Err(error).context("opening the clipboard");
        }
        let _ = EmptyClipboard();
        let placed = SetClipboardData(u32::from(CF_HDROP.0), Some(HANDLE(drop.0)));
        let format = RegisterClipboardFormatW(&HSTRING::from(DROP_EFFECT));
        let _ = SetClipboardData(format, Some(HANDLE(effect.0)));
        let _ = CloseClipboard();
        placed.context("placing files on the clipboard")?;
    }
    Ok(())
}

pub(super) fn global(size: usize, fill: impl FnOnce(*mut u8)) -> Result<HGLOBAL> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, size) }.context("allocating clipboard memory")?;
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        let _ = unsafe { GlobalFree(Some(memory)) };
        bail!("locking clipboard memory failed");
    }
    fill(pointer.cast());
    let _ = unsafe { GlobalUnlock(memory) };
    Ok(memory)
}

pub(crate) fn recycle(paths: &[String]) -> Result<()> {
    let names: Vec<u16> = paths.iter().flat_map(|path| path.encode_utf16().chain([0])).chain([0]).collect();
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(names.as_ptr()),
        fFlags: FOF_ALLOWUNDO.0 as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut operation) };
    if code != 0 {
        bail!("moving to the Recycle Bin failed (code {code})");
    }
    Ok(())
}

pub(crate) fn explorer_selection(host: Host) -> Vec<String> {
    if host.kind != HostKind::Explorer {
        return Vec::new();
    }
    ensure_com();
    let Ok(windows) = (unsafe { CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_ALL) }) else {
        return Vec::new();
    };
    let count = unsafe { windows.Count() }.unwrap_or(0);
    for position in 0..count {
        let Ok(item) = (unsafe { windows.Item(&VARIANT::from(position)) }) else { continue };
        let Ok(browser) = item.cast::<IWebBrowserApp>() else { continue };
        if !unsafe { browser.HWND() }.is_ok_and(|hwnd| hwnd.0 == host.window) {
            continue;
        }
        let Some(view) = (unsafe { browser.Document() }).ok().and_then(|view| view.cast::<IShellFolderViewDual>().ok())
        else {
            return Vec::new();
        };
        let Ok(items) = (unsafe { view.SelectedItems() }) else { return Vec::new() };
        let total = unsafe { items.Count() }.unwrap_or(0);
        return (0..total)
            .filter_map(|index| unsafe { items.Item(&VARIANT::from(index)).and_then(|item| item.Path()) }.ok())
            .map(|path| path.to_string())
            .collect();
    }
    Vec::new()
}
