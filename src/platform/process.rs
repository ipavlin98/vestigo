use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_INC_WORKING_SET_NAME, SE_INCREASE_QUOTA_NAME,
    SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    QUOTA_LIMITS_HARDWS_MAX_DISABLE, QUOTA_LIMITS_HARDWS_MIN_DISABLE, QUOTA_LIMITS_HARDWS_MIN_ENABLE,
    SetProcessWorkingSetSizeEx, VirtualLock,
};
use windows::Win32::System::ProcessStatus::{
    K32GetModuleInformation, K32GetProcessMemoryInfo, MODULEINFO, PROCESS_MEMORY_COUNTERS,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetCurrentThread, OpenProcessToken,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling,
    SetPriorityClass, SetProcessInformation, SetThreadInformation, SetThreadPriority,
    THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED, THREAD_POWER_THROTTLING_STATE,
    THREAD_PRIORITY, THREAD_PRIORITY_ABOVE_NORMAL, THREAD_PRIORITY_LOWEST, ThreadPowerThrottling,
};

const LOCK_OVERHEAD: usize = 1 << 20;
const WS_HEADROOM: usize = 16 << 20;
const WS_STEP: usize = 4 << 20;
const WS_MAX_EXTRA: usize = 256 << 20;
const MIN_RESIDENT_CAP: usize = 64 << 20;
const MAX_RESIDENT_CAP: usize = if usize::BITS >= 64 { 2 << 30 } else { 256 << 20 };
const RESIDENT_SHARE: u64 = 8;

static LOCKED: AtomicUsize = AtomicUsize::new(0);
static RESIDENT_FLOOR: AtomicUsize = AtomicUsize::new(0);
static HARD_WS_WARNED: AtomicBool = AtomicBool::new(false);
static MEMORY_CAP: OnceLock<usize> = OnceLock::new();

pub(crate) fn prepare_process() {
    enable_quota_privilege();
    unsafe {
        if let Err(error) = SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) {
            log::warn!("Couldn't raise the process priority: {error}");
        }
    }
    disable_process_throttling();
    lock_code();
}

pub(crate) fn keep_resident() {
    let Some((resident, committed)) = memory_usage() else { return };
    let locked = LOCKED.load(Ordering::Relaxed);
    let floor = RESIDENT_FLOOR.load(Ordering::Relaxed);
    let Some(minimum) = next_floor(resident, committed, locked, floor, memory_cap()) else { return };
    if reserve_working_set(minimum) {
        RESIDENT_FLOOR.store(minimum, Ordering::Relaxed);
    }
}

fn next_floor(resident: usize, committed: usize, locked: usize, floor: usize, cap: usize) -> Option<usize> {
    let desired = resident.saturating_add(WS_HEADROOM);
    let freed = floor > 0 && committed.saturating_add(locked).saturating_add(WS_HEADROOM) < floor;
    let target = if freed { desired } else { desired.max(floor) };
    let minimum = target.max(locked.saturating_add(LOCK_OVERHEAD)).min(cap);
    (floor == 0 || minimum.abs_diff(floor) >= WS_STEP).then_some(minimum)
}

fn lock_code() {
    let mut module = MODULEINFO::default();
    let size = size_of::<MODULEINFO>() as u32;
    unsafe {
        let Ok(exe) = GetModuleHandleW(None) else { return };
        if !K32GetModuleInformation(GetCurrentProcess(), exe, &mut module, size).as_bool() {
            return;
        }
        let image = module.SizeOfImage as usize;
        let resident = memory_usage().map_or(0, |(resident, _)| resident);
        let minimum = resident.saturating_add(image).saturating_add(LOCK_OVERHEAD).min(memory_cap());
        if reserve_working_set(minimum) {
            RESIDENT_FLOOR.store(minimum, Ordering::Relaxed);
        }
        match VirtualLock(module.lpBaseOfDll, image) {
            Ok(()) => LOCKED.store(image, Ordering::Relaxed),
            Err(error) => log::warn!("Couldn't keep the program code in memory: {error}"),
        }
    }
}

fn enable_quota_privilege() {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token).is_err() {
            return;
        }
        for name in [SE_INCREASE_QUOTA_NAME, SE_INC_WORKING_SET_NAME] {
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(None, name, &mut luid).is_ok() {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
                };
                let _ = AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None);
            }
        }
        let _ = CloseHandle(token);
    }
}

fn disable_process_throttling() {
    let set = |mask: u32| {
        let unthrottled = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: mask,
            StateMask: 0,
        };
        let state: *const std::ffi::c_void = (&raw const unthrottled).cast();
        let size = size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32;
        unsafe { SetProcessInformation(GetCurrentProcess(), ProcessPowerThrottling, state, size) }
    };
    let all = PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION;
    if set(all).is_err() {
        let _ = set(PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
    }
}

fn memory_usage() -> Option<(usize, usize)> {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) }
        .as_bool()
        .then_some((counters.WorkingSetSize, counters.PagefileUsage))
}

fn memory_cap() -> usize {
    *MEMORY_CAP.get_or_init(|| {
        let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_err() {
            return MIN_RESIDENT_CAP;
        }
        let share = (status.ullTotalPhys / RESIDENT_SHARE).clamp(MIN_RESIDENT_CAP as u64, MAX_RESIDENT_CAP as u64);
        usize::try_from(share).unwrap_or(MAX_RESIDENT_CAP)
    })
}

fn reserve_working_set(minimum: usize) -> bool {
    let maximum = minimum.saturating_add(minimum.min(WS_MAX_EXTRA)).max(minimum.saturating_add(WS_HEADROOM));
    let hard = QUOTA_LIMITS_HARDWS_MIN_ENABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE;
    match unsafe { SetProcessWorkingSetSizeEx(GetCurrentProcess(), minimum, maximum, hard) } {
        Ok(()) => true,
        Err(error) => {
            if !HARD_WS_WARNED.swap(true, Ordering::Relaxed) {
                log::debug!("Falling back to a soft working set floor: {error}");
            }
            let soft = QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE;
            unsafe { SetProcessWorkingSetSizeEx(GetCurrentProcess(), minimum, maximum, soft) }.is_ok()
        }
    }
}

pub(crate) fn prioritize_thread() {
    prioritize(THREAD_PRIORITY_ABOVE_NORMAL);
}

pub(crate) fn deprioritize_thread() {
    configure_thread(THREAD_PRIORITY_LOWEST, true);
}

pub(super) fn prioritize(priority: THREAD_PRIORITY) {
    configure_thread(priority, false);
}

fn configure_thread(priority: THREAD_PRIORITY, throttle: bool) {
    let throttling = THREAD_POWER_THROTTLING_STATE {
        Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: if throttle { THREAD_POWER_THROTTLING_EXECUTION_SPEED } else { 0 },
    };
    unsafe {
        let thread = GetCurrentThread();
        if let Err(error) = SetThreadPriority(thread, priority) {
            log::warn!("Couldn't change the thread priority: {error}");
        }
        let state: *const std::ffi::c_void = (&raw const throttling).cast();
        let size = size_of::<THREAD_POWER_THROTTLING_STATE>() as u32;
        let _ = SetThreadInformation(thread, ThreadPowerThrottling, state, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 1 << 20;

    #[test]
    fn working_set_floor_survives_trimming_and_lowers_when_memory_is_freed() {
        let cap = 1024 * MB;
        let locked = 16 * MB;
        assert_eq!(next_floor(200 * MB, 200 * MB, locked, 0, cap), Some(216 * MB));
        assert_eq!(next_floor(80 * MB, 200 * MB, locked, 216 * MB, cap), None);
        assert_eq!(next_floor(202 * MB, 202 * MB, locked, 216 * MB, cap), None);
        assert_eq!(next_floor(220 * MB, 220 * MB, locked, 216 * MB, cap), Some(236 * MB));
        assert_eq!(next_floor(100 * MB, 100 * MB, locked, 236 * MB, cap), Some(116 * MB));
        assert_eq!(next_floor(1500 * MB, 1500 * MB, locked, 0, cap), Some(cap));
    }
}
