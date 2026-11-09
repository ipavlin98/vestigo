use crate::index::Pipe;
use anyhow::{Context, Result};
use std::ffi::c_void;
use std::fs::File;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::Path;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation};
use windows::Win32::Globalization::GetACP;
use windows::Win32::Security::{
    CreateRestrictedToken, CreateWellKnownSid, DISABLE_MAX_PRIVILEGE, PSID, SECURITY_ATTRIBUTES, SID_AND_ATTRIBUTES,
    SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL,
    TOKEN_QUERY, TokenIntegrityLevel, WELL_KNOWN_SID_TYPE, WinBuiltinAdministratorsSid, WinLowLabelSid,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, PROCESS_INFORMATION,
    PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW,
    SetPriorityClass, SetProcessInformation,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

const SID_BYTES: usize = 68;
const PIPE_BYTES: u32 = 64 * 1024;
const MEMORY_LIMIT: usize = 1 << 30;
const FLAG: &str = "--extract";

struct Handle(HANDLE);

impl Handle {
    fn into_file(self) -> File {
        let handle = self.0.0 as RawHandle;
        std::mem::forget(self);
        unsafe { File::from_raw_handle(handle) }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Job(isize);

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let job = HANDLE(self.0 as _);
            let _ = TerminateJobObject(job, 1);
            let _ = CloseHandle(job);
        }
    }
}

fn well_known(kind: WELL_KNOWN_SID_TYPE, sid: &mut [u8; SID_BYTES]) -> Result<PSID> {
    let id = PSID(sid.as_mut_ptr().cast());
    let mut size = SID_BYTES as u32;
    unsafe { CreateWellKnownSid(kind, None, Some(id), &mut size) }.context("building a security id")?;
    Ok(id)
}

fn limited_token() -> Result<Handle> {
    let mut own = HANDLE::default();
    let access = TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_DEFAULT;
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut own) }.context("opening the process token")?;
    let own = Handle(own);
    let mut administrators = [0; SID_BYTES];
    let denied =
        [SID_AND_ATTRIBUTES { Sid: well_known(WinBuiltinAdministratorsSid, &mut administrators)?, Attributes: 0 }];
    let mut limited = HANDLE::default();
    unsafe { CreateRestrictedToken(own.0, DISABLE_MAX_PRIVILEGE, Some(&denied), None, None, &mut limited) }
        .context("restricting the token")?;
    let limited = Handle(limited);
    let mut low = [0; SID_BYTES];
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES { Sid: well_known(WinLowLabelSid, &mut low)?, Attributes: SE_GROUP_INTEGRITY as u32 },
    };
    let size = (size_of::<TOKEN_MANDATORY_LABEL>() + SID_BYTES) as u32;
    let label: *const c_void = (&raw const label).cast();
    unsafe { SetTokenInformation(limited.0, TokenIntegrityLevel, label, size) }.context("lowering the token")?;
    Ok(limited)
}

fn pipe() -> Result<(Handle, Handle)> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: true.into(),
        ..Default::default()
    };
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut read, &mut write, Some(&attributes), PIPE_BYTES) }.context("creating a pipe")?;
    Ok((Handle(read), Handle(write)))
}

fn keep_here(handle: &Handle) -> Result<()> {
    unsafe { SetHandleInformation(handle.0, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }.context("guarding a pipe")
}

fn limited_job() -> Result<Job> {
    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.context("creating a job")?;
    let job = Job(job.0 as isize);
    let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
            LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            ..Default::default()
        },
        ProcessMemoryLimit: MEMORY_LIMIT,
        ..Default::default()
    };
    let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    let limits: *const c_void = (&raw const limits).cast();
    unsafe { SetInformationJobObject(HANDLE(job.0 as _), JobObjectExtendedLimitInformation, limits, size) }
        .context("limiting the job")?;
    Ok(job)
}

pub(crate) fn spawn_program(program: &Path) -> Result<Pipe> {
    let token = limited_token()?;
    let (child_input, input) = pipe()?;
    let (output, child_output) = pipe()?;
    keep_here(&input)?;
    keep_here(&output)?;
    let job = limited_job()?;
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: child_input.0,
        hStdOutput: child_output.0,
        ..Default::default()
    };
    let mut line: Vec<u16> =
        format!("\"{}\" {FLAG} {}", program.display(), code_page()).encode_utf16().chain([0]).collect();
    let mut process = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessAsUserW(
            Some(token.0),
            &HSTRING::from(program.as_os_str()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            None,
            None,
            &startup,
            &mut process,
        )
    }
    .context("starting the text extractor")?;
    let (child, thread) = (Handle(process.hProcess), Handle(process.hThread));
    unsafe { AssignProcessToJobObject(HANDLE(job.0 as _), child.0) }.context("adding the extractor to its job")?;
    unsafe { ResumeThread(thread.0) };
    let stop = Box::new(move || drop(job));
    Ok(Pipe { input: Box::new(input.into_file()), output: Box::new(output.into_file()), stop })
}

pub(crate) fn spawn_extractor() -> Result<Pipe> {
    spawn_program(&std::env::current_exe().context("finding the program file")?)
}

pub(crate) fn code_page() -> u32 {
    unsafe { GetACP() }
}

pub(crate) fn enter_background() {
    let throttling = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    };
    unsafe {
        let process = GetCurrentProcess();
        let _ = SetPriorityClass(process, PROCESS_MODE_BACKGROUND_BEGIN);
        let state: *const c_void = (&raw const throttling).cast();
        let size = size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32;
        let _ = SetProcessInformation(process, ProcessPowerThrottling, state, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::read_documents;
    use std::process::{Command, Stdio};
    use std::sync::Arc;

    const SAMPLE_CHARS: usize = 160;

    fn program() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("debug").join("vestigo.exe")
    }

    fn plain() -> Result<Pipe> {
        let mut child = Command::new(program())
            .args([FLAG, &code_page().to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let input = child.stdin.take().context("no input")?;
        let output = child.stdout.take().context("no output")?;
        let stop = Box::new(move || {
            let _ = child.kill();
        });
        Ok(Pipe { input: Box::new(input), output: Box::new(output), stop })
    }

    #[test]
    #[ignore = "starts target\\debug\\vestigo.exe on the documents listed in VESTIGO_DOCUMENTS"]
    fn the_limited_extractor_reads_what_a_plain_process_reads() {
        let list = std::env::var("VESTIGO_DOCUMENTS").expect("a file that lists documents");
        let list = std::fs::read_to_string(list).expect("the list of documents");
        let paths: Vec<String> = list.lines().map(str::to_owned).collect();
        let limited = read_documents(&(Arc::new(|| spawn_program(&program())) as _), &paths);
        let open = read_documents(&(Arc::new(plain) as _), &paths);
        let count = |read: &[(String, String, String)], outcome: &str| {
            read.iter().filter(|(_, found, _)| found == outcome).count()
        };
        for outcome in ["Text", "Empty", "Unsupported", "Encrypted", "Denied", "Failed"] {
            println!("{outcome}: limited {}, plain {}", count(&limited, outcome), count(&open, outcome));
        }
        for (left, right) in limited.iter().zip(&open) {
            if left.1 != right.1 || left.2.len() != right.2.len() {
                println!("DIFFERS {}: {} {} / {} {}", left.0, left.1, left.2.len(), right.1, right.2.len());
            }
        }
        for (path, outcome, text) in &limited {
            let sample: String = text.chars().filter(|ch| !ch.is_ascii()).take(SAMPLE_CHARS).collect();
            println!("{outcome} {} chars {path}: {sample}", text.chars().count());
        }
    }
}
