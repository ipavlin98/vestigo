use super::super::hidden::hidden_files_shown;
use super::super::shell::{class_name, host_folder};
use super::super::{Clicked, Event, Host, HostKind, send};
use super::mouse::Limits;
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAllocEx, VirtualFreeEx,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_VM_OPERATION, PROCESS_VM_WRITE};
use windows::Win32::UI::Accessibility::{
    AutomationElementMode_None, CUIAutomation, IUIAutomation, IUIAutomationCacheRequest, UIA_ControlTypePropertyId,
    UIA_ListControlTypeId,
};
use windows::Win32::UI::Controls::{LVHITTESTINFO, LVM_HITTEST};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetParent, GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WindowFromPoint,
};

const DIRECT_UI: &str = "DirectUIHWND";
const LIST_VIEW: &str = "SysListView32";
const VIEW_PARENT: &str = "SHELLDLL_DefView";
const NO_ITEM: isize = -1;
const HIT_TEST_TIMEOUT_MS: u32 = 200;

enum Job {
    Probe(POINT),
    Open(POINT, Limits),
}

static JOBS: OnceLock<Sender<Job>> = OnceLock::new();

pub(super) fn probe(at: POINT) {
    request(Job::Probe(at));
}

pub(super) fn open(at: POINT, limits: Limits) {
    request(Job::Open(at, limits));
}

fn request(job: Job) {
    let jobs = JOBS.get_or_init(|| {
        let (sender, receiver) = channel();
        if let Err(error) = std::thread::Builder::new().name("menu-click".into()).spawn(move || serve(receiver)) {
            log::warn!("Couldn't start the click checks: {error}");
        }
        sender
    });
    if jobs.send(job).is_err() {
        log::warn!("The click checks stopped");
    }
}

fn serve(jobs: Receiver<Job>) {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let mut prober = Prober::default();
    let mut ready: Option<(POINT, Option<Clicked>)> = None;
    while let Ok(first) = jobs.recv() {
        let mut wanted = None;
        for job in std::iter::once(first).chain(jobs.try_iter()) {
            match job {
                Job::Probe(at) => wanted = Some(at),
                Job::Open(at, limits) => {
                    let seen = wanted.take().map(|at| (at, prober.spot(at))).or_else(|| ready.take());
                    let spot = match seen {
                        Some((seen, spot)) if near(seen, at, limits) => spot,
                        Some(_) | None => prober.spot(at),
                    };
                    if let Some(clicked) = spot {
                        send(Event::FolderMenu(clicked));
                    }
                }
            }
        }
        ready = wanted.map(|at| (at, prober.spot(at)));
    }
}

fn near(seen: POINT, at: POINT, limits: Limits) -> bool {
    (seen.x - at.x).abs() <= limits.width / 2 && (seen.y - at.y).abs() <= limits.height / 2
}

#[derive(Default)]
struct Prober {
    automation: Option<Automation>,
    remote: Option<Remote>,
}

struct Automation {
    client: IUIAutomation,
    request: IUIAutomationCacheRequest,
}

impl Prober {
    fn spot(&mut self, at: POINT) -> Option<Clicked> {
        let view = unsafe { WindowFromPoint(at) };
        let class = class_name(view);
        if !(class == DIRECT_UI || class == LIST_VIEW)
            || !unsafe { GetParent(view) }.is_ok_and(|parent| class_name(parent) == VIEW_PARENT)
        {
            return None;
        }
        let root = unsafe { GetAncestor(view, GA_ROOT) };
        let kind = match class_name(root).as_str() {
            "CabinetWClass" | "ExploreWClass" => HostKind::Explorer,
            "Progman" | "WorkerW" => HostKind::Desktop,
            _ => return None,
        };
        let empty = if class == LIST_VIEW { self.list_empty(view, at) } else { self.view_empty(at) };
        if !empty {
            return None;
        }
        let host = Host::new(root, kind);
        Some(Clicked { host, folder: host_folder(host), hidden_shown: hidden_files_shown() })
    }

    fn view_empty(&mut self, at: POINT) -> bool {
        if self.automation.is_none() {
            self.automation =
                Automation::create().inspect_err(|error| log::warn!("Couldn't start UI Automation: {error}")).ok();
        }
        let Some(automation) = self.automation.as_ref() else { return false };
        let kind = unsafe { automation.client.ElementFromPointBuildCache(at, &automation.request) }
            .and_then(|element| unsafe { element.CachedControlType() });
        match kind {
            Ok(kind) => kind == UIA_ListControlTypeId,
            Err(error) => {
                log::debug!("Couldn't read the clicked spot: {error}");
                false
            }
        }
    }

    fn list_empty(&mut self, view: HWND, at: POINT) -> bool {
        let mut process = 0;
        unsafe { GetWindowThreadProcessId(view, Some(&mut process)) };
        if self.remote.as_ref().is_none_or(|remote| remote.process != process) {
            self.remote = None;
            self.remote = Remote::open(process);
        }
        let Some(remote) = self.remote.as_ref() else { return false };
        let mut point = at;
        if !unsafe { ScreenToClient(view, &mut point) }.as_bool() {
            return false;
        }
        match remote.hit_test(view, point) {
            Some(item) => item == NO_ITEM,
            None => {
                self.remote = None;
                false
            }
        }
    }
}

impl Automation {
    fn create() -> windows::core::Result<Self> {
        let client: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }?;
        let request = unsafe { client.CreateCacheRequest() }?;
        unsafe {
            request.AddProperty(UIA_ControlTypePropertyId)?;
            request.SetAutomationElementMode(AutomationElementMode_None)?;
        }
        Ok(Self { client, request })
    }
}

struct Remote {
    process: u32,
    handle: HANDLE,
    buffer: *mut c_void,
}

impl Remote {
    fn open(process: u32) -> Option<Self> {
        let handle = unsafe { OpenProcess(PROCESS_VM_OPERATION | PROCESS_VM_WRITE, false, process) }
            .inspect_err(|error| log::debug!("Couldn't reach the desktop process: {error}"))
            .ok()?;
        let size = size_of::<LVHITTESTINFO>();
        let buffer = unsafe { VirtualAllocEx(handle, None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if buffer.is_null() {
            let _ = unsafe { CloseHandle(handle) };
            return None;
        }
        Some(Self { process, handle, buffer })
    }

    fn hit_test(&self, view: HWND, point: POINT) -> Option<isize> {
        let info = LVHITTESTINFO { pt: point, ..Default::default() };
        let size = size_of::<LVHITTESTINFO>();
        unsafe { WriteProcessMemory(self.handle, self.buffer, (&info as *const LVHITTESTINFO).cast(), size, None) }
            .ok()?;
        let mut item = 0usize;
        let sent = unsafe {
            SendMessageTimeoutW(
                view,
                LVM_HITTEST,
                WPARAM(0),
                LPARAM(self.buffer as _),
                SMTO_ABORTIFHUNG,
                HIT_TEST_TIMEOUT_MS,
                Some(&mut item),
            )
        };
        (sent.0 != 0).then_some(item as isize)
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFreeEx(self.handle, self.buffer, 0, MEM_RELEASE);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits { time: 500, width: 4, height: 4 };

    #[test]
    fn a_probe_counts_for_a_click_inside_the_double_click_box() {
        let seen = POINT { x: 100, y: 100 };
        assert!(near(seen, POINT { x: 102, y: 98 }, LIMITS));
        assert!(!near(seen, POINT { x: 103, y: 100 }, LIMITS));
        assert!(!near(seen, POINT { x: 100, y: 97 }, LIMITS));
    }
}
