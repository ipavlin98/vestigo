use crate::i18n::Language;
use crate::platform::{self, Transfer};
use crate::settings::Program;
use gpui::Global;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use std::cmp::Ordering;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use zip::ZipArchive;

pub(crate) const WAIT_FLAG: &str = "--after-update";
const RELEASES: &str = "https://api.github.com/repos/ipavlin98/vestigo/releases?per_page=30";
const ACCEPT: &str = "Accept: application/vnd.github+json";
const SUMS: &str = "SHA256SUMS.txt";
const FOLDER: &str = "Vestigo-update";
const UNINSTALLER: &str = "unins000.exe";
const OLD: &str = ".update-old";
const NEW: &str = ".update-new";
const SETUP_FLAGS: &str = "/SILENT /SUPPRESSMSGBOXES /NORESTART /relaunch=1";

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Updates {
    #[default]
    Idle,
    Checking,
    Latest,
    CheckFailed(Trouble),
    Available(Release),
    Downloading(Release),
    InstallFailed(Release, Trouble),
}

impl Global for Updates {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Trouble {
    Offline,
    TimedOut,
    Damaged,
    Failed(String),
}

impl From<Transfer> for Trouble {
    fn from(transfer: Transfer) -> Self {
        match transfer {
            Transfer::Offline => Self::Offline,
            Transfer::TimedOut => Self::TimedOut,
            Transfer::Failed(message) => Self::Failed(message),
        }
    }
}

impl From<io::Error> for Trouble {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<zip::result::ZipError> for Trouble {
    fn from(_: zip::result::ZipError) -> Self {
        Self::Damaged
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Part {
    Number(u64),
    Text(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Version {
    core: [u64; 3],
    pre: Vec<Part>,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split_once('+').map_or(text, |(version, _)| version);
        let (core, pre) = text.split_once('-').map_or((text, None), |(core, pre)| (core, Some(pre)));
        let mut numbers = core.split('.').map(|number| number.parse::<u64>().ok());
        let core = [numbers.next()??, numbers.next()??, numbers.next()??];
        if numbers.next().is_some() {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre.split('.').map(part).collect::<Option<_>>()?,
        };
        Some(Self { core, pre })
    }

    fn current() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }

    fn preview(&self) -> bool {
        !self.pre.is_empty()
    }
}

fn part(text: &str) -> Option<Part> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(text.parse().map_or_else(|_| Part::Text(text.into()), Part::Number))
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.preview(), other.preview()) {
            (false, false) => Ordering::Equal,
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (true, true) => self.pre.cmp(&other.pre),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let [major, minor, patch] = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        for (index, part) in self.pre.iter().enumerate() {
            f.write_str(if index == 0 { "-" } else { "." })?;
            match part {
                Part::Number(number) => write!(f, "{number}")?,
                Part::Text(text) => f.write_str(text)?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arch {
    X64,
    X86,
    Arm64,
}

impl Arch {
    const CURRENT: Arch = if cfg!(target_arch = "aarch64") {
        Arch::Arm64
    } else if cfg!(target_arch = "x86") {
        Arch::X86
    } else {
        Arch::X64
    };

    fn suffix(self) -> &'static str {
        match self {
            Arch::X64 | Arch::X86 => "",
            Arch::Arm64 => "-arm64",
        }
    }

    fn exe(self) -> &'static str {
        match self {
            Arch::X64 | Arch::Arm64 => "Vestigo.exe",
            Arch::X86 => "Vestigo-x86.exe",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    Setup,
    Portable,
}

impl Form {
    fn of(exe: &Path) -> Self {
        let installed = exe.parent().is_some_and(|folder| folder.join(UNINSTALLER).is_file());
        if installed { Self::Setup } else { Self::Portable }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Channel {
    Stable,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Install {
    arch: Arch,
    form: Form,
    channel: Channel,
}

impl Install {
    fn asset(self, version: &Version) -> String {
        let arch = self.arch.suffix();
        match self.form {
            Form::Setup => format!("Vestigo-{version}{arch}-setup.exe"),
            Form::Portable => format!("Vestigo-{version}{arch}-portable.zip"),
        }
    }

    fn accepts(self, version: &Version) -> bool {
        self.channel == Channel::Preview || !version.preview()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Release {
    pub(crate) version: Version,
    form: Form,
    file: Asset,
    sums: String,
}

fn newest(json: &[u8], install: Install, current: &Version) -> Result<Option<Release>, Trouble> {
    let listed: Vec<Listed> = serde_json::from_slice(json)
        .map_err(|error| Trouble::Failed(format!("GitHub returned an unexpected release list: {error}")))?;
    let usable = listed.into_iter().filter(|listed| !listed.draft).filter_map(|listed| {
        let version = Version::parse(&listed.tag_name)?;
        if version <= *current || !install.accepts(&version) {
            return None;
        }
        let url = |name: &str| {
            listed.assets.iter().find(|asset| asset.name == name).map(|asset| asset.browser_download_url.clone())
        };
        let name = install.asset(&version);
        let file = Asset { browser_download_url: url(&name)?, name };
        Some(Release { sums: url(SUMS)?, version, form: install.form, file })
    });
    Ok(usable.max_by(|a, b| a.version.cmp(&b.version)))
}

pub(crate) fn check() -> Result<Option<Release>, Trouble> {
    let current = Version::current().ok_or_else(|| Trouble::Failed("the app version isn't valid".into()))?;
    let exe = std::env::current_exe()?;
    let channel = if current.preview() { Channel::Preview } else { Channel::Stable };
    let install = Install { arch: Arch::CURRENT, form: Form::of(&exe), channel };
    log::info!("Checking for updates to {current}, {install:?}");
    newest(&platform::fetch(RELEASES, ACCEPT)?, install, &current)
}

pub(crate) enum Handover {
    Setup(PathBuf),
    Restart(PathBuf),
}

impl Handover {
    pub(crate) fn start(self, language: Language) -> Result<(), Trouble> {
        match self {
            Self::Setup(file) => {
                let language = match language {
                    Language::English => "en",
                    Language::Russian => "ru",
                };
                let program = Program {
                    path: file.to_string_lossy().into_owned(),
                    arguments: format!("{SETUP_FLAGS} /LANG={language}"),
                    admin: true,
                    ..Program::default()
                };
                platform::run(&program).map_err(|error| Trouble::Failed(format!("{error:#}")))
            }
            Self::Restart(exe) => {
                Command::new(exe).args([WAIT_FLAG, crate::ELEVATED_FLAG]).spawn()?;
                Ok(())
            }
        }
    }
}

pub(crate) fn prepare(release: &Release) -> Result<Handover, Trouble> {
    let folder = std::env::temp_dir().join(FOLDER);
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder)?;
    let sums = folder.join(SUMS);
    platform::download(&release.sums, &sums)?;
    let file = folder.join(&release.file.name);
    platform::download(&release.file.browser_download_url, &file)?;
    verify(&fs::read_to_string(&sums)?, &release.file.name, &file)?;
    match release.form {
        Form::Setup => Ok(Handover::Setup(file)),
        Form::Portable => {
            let exe = std::env::current_exe()?;
            replace(&file, &exe, Arch::CURRENT)?;
            Ok(Handover::Restart(exe))
        }
    }
}

fn verify(sums: &str, name: &str, file: &Path) -> Result<(), Trouble> {
    let expected = sums.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    });
    if expected != Some(sha256(&fs::read(file)?)) {
        return Err(Trouble::Damaged);
    }
    Ok(())
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(suffix);
    name.into()
}

fn replace(archive: &Path, exe: &Path, arch: Arch) -> Result<(), Trouble> {
    let folder = exe.parent().ok_or_else(|| Trouble::Failed("the app has no folder".into()))?;
    let mut fresh = Vec::new();
    let unpacked = unpack(archive, exe, folder, arch, &mut fresh);
    if unpacked.is_err() || !fresh.iter().any(|(target, _)| target == exe) {
        discard(&fresh);
        unpacked?;
        return Err(Trouble::Damaged);
    }
    fresh.sort_by_key(|(target, _)| target != exe);
    let Some(((main, path), rest)) = fresh.split_first() else { return Err(Trouble::Damaged) };
    if let Err(error) = swap(main, path) {
        discard(&fresh);
        return Err(error.into());
    }
    for (target, path) in rest {
        if let Err(error) = swap(target, path) {
            log::warn!("Couldn't update {}: {error}", target.display());
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn discard(fresh: &[(PathBuf, PathBuf)]) {
    for (_, path) in fresh {
        let _ = fs::remove_file(path);
    }
}

fn unpack(
    archive: &Path,
    exe: &Path,
    folder: &Path,
    arch: Arch,
    fresh: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), Trouble> {
    let mut zip = ZipArchive::new(File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(name) = entry.enclosed_name().and_then(|path| path.file_name().map(ToOwned::to_owned)) else {
            continue;
        };
        if !entry.is_file() {
            continue;
        }
        let own = name.eq_ignore_ascii_case(arch.exe());
        let target = if own { exe.to_path_buf() } else { folder.join(&name) };
        if !own && target.as_os_str().eq_ignore_ascii_case(exe.as_os_str()) {
            continue;
        }
        let path = suffixed(&target, NEW);
        let mut output = File::create(&path)?;
        fresh.push((target, path));
        io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}

fn swap(target: &Path, fresh: &Path) -> io::Result<()> {
    let old = suffixed(target, OLD);
    let existed = target.exists();
    if existed {
        fs::rename(target, &old)?;
    }
    fs::rename(fresh, target).inspect_err(|_| {
        if existed {
            let _ = fs::rename(&old, target);
        }
    })
}

pub(crate) fn sweep() {
    let _ = fs::remove_dir_all(std::env::temp_dir().join(FOLDER));
    if let Ok(exe) = std::env::current_exe() {
        sweep_beside(&exe);
    }
}

fn sweep_beside(exe: &Path) {
    let Some(Ok(entries)) = exe.parent().map(fs::read_dir) else {
        return;
    };
    for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if (name.ends_with(OLD) || name.ends_with(NEW)) && fs::remove_file(&path).is_ok() {
            log::info!("Removed a file left by an update: {}", path.display());
        }
    }
}

#[cfg(test)]
mod tests;
