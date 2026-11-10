use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const CONNECT_TIMEOUT: &str = "15";
const FETCH_TIMEOUT: &str = "60";
const CURL_RESOLVE: i32 = 6;
const CURL_CONNECT: i32 = 7;
const CURL_TIMEOUT: i32 = 28;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Transfer {
    Offline,
    TimedOut,
    Failed(String),
}

pub(crate) fn fetch(url: &str, accept: &str) -> Result<Vec<u8>, Transfer> {
    let output = curl(&["-sSfL", "--max-time", FETCH_TIMEOUT, "-H", accept, url])?;
    Ok(output.stdout)
}

pub(crate) fn download(url: &str, target: &Path) -> Result<(), Transfer> {
    let target = target.to_string_lossy();
    curl(&["-sSfL", "-o", &target, url]).map(drop)
}

fn curl(arguments: &[&str]) -> Result<Output, Transfer> {
    let output = Command::new(curl_exe())
        .args(["--connect-timeout", CONNECT_TIMEOUT])
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|error| Transfer::Failed(format!("curl.exe: {error}")))?;
    if output.status.success() {
        return Ok(output);
    }
    Err(match output.status.code() {
        Some(CURL_RESOLVE | CURL_CONNECT) => Transfer::Offline,
        Some(CURL_TIMEOUT) => Transfer::TimedOut,
        _ => Transfer::Failed(last_line(&output.stderr)),
    })
}

fn curl_exe() -> PathBuf {
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&system).join("System32").join("curl.exe")
}

fn last_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    text.lines().rev().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned()
}
