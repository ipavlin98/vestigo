use super::*;
use serde_json::{Value, json};
use std::io::Write as _;
use zip::write::{SimpleFileOptions, ZipWriter};

const SETUP_HASH: &str = "8fb6d5f37e8055ce720bd0b1d56587f88c0071f285966ba17e72b2b12672aa73";
const OTHER_HASH: &str = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2";
const X64_SETUP: Install = Install { arch: Arch::X64, form: Form::Setup, channel: Channel::Preview };

fn version(text: &str) -> Version {
    Version::parse(text).expect("valid version")
}

fn url(tag: &str, name: &str) -> String {
    format!("https://github.com/ipavlin98/vestigo/releases/download/{tag}/{name}")
}

fn listed(tag: &str, draft: bool, names: &[String]) -> Value {
    let assets: Vec<Value> =
        names.iter().map(|name| json!({"name": name, "browser_download_url": url(tag, name)})).collect();
    json!({"tag_name": tag, "draft": draft, "prerelease": tag.contains('-'), "assets": assets})
}

fn full(tag: &str) -> Value {
    let version = tag.trim_start_matches('v');
    let names = [
        SUMS.to_owned(),
        format!("Vestigo-{version}-setup.exe"),
        format!("Vestigo-{version}-portable.zip"),
        format!("Vestigo-{version}-arm64-setup.exe"),
        format!("Vestigo-{version}-arm64-portable.zip"),
    ];
    listed(tag, false, &names)
}

fn found(releases: &[Value], install: Install, current: &str) -> Option<Release> {
    let json = Value::Array(releases.to_vec()).to_string();
    newest(json.as_bytes(), install, &version(current)).expect("release list")
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vestigo-update-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("scratch folder");
        Self(root)
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        fs::write(&path, text).expect("file");
        path
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).expect(name)
    }

    fn names(&self, folder: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.join(folder))
            .expect("folder")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn archive(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let path = self.0.join(name);
        let mut zip = ZipWriter::new(File::create(&path).expect("archive"));
        for (name, text) in files {
            zip.start_file(*name, SimpleFileOptions::default()).expect("entry");
            zip.write_all(text.as_bytes()).expect("entry data");
        }
        zip.finish().expect("archive written");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn versions_follow_semver_precedence() {
    let ascending = [
        "0.1.0-alpha",
        "0.1.0-alpha.2",
        "0.1.0-alpha.10",
        "0.1.0-alpha.beta",
        "0.1.0-beta",
        "0.1.0-rc.1",
        "0.1.0",
        "0.1.1",
        "0.2.0-alpha",
        "0.10.0",
        "1.0.0",
    ];
    for pair in ascending.windows(2) {
        assert!(version(pair[0]) < version(pair[1]), "{} < {}", pair[0], pair[1]);
    }
    assert_eq!(version("v0.2.0"), version("0.2.0"));
    assert_eq!(version("0.2.0+build.5"), version("0.2.0"));
    assert_eq!(version("0.2.0-alpha.2").to_string(), "0.2.0-alpha.2");
    for broken in ["", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "1.2.3-a..b", "1.2.3-a/b", "-1.2.3"] {
        assert_eq!(Version::parse(broken), None, "{broken:?}");
    }
}

#[test]
fn the_app_version_is_a_valid_version() {
    assert!(Version::current().is_some());
}

#[test]
fn newest_complete_release_is_offered() {
    let releases = [
        full("v0.1.0"),
        full("v0.2.0"),
        listed("v0.3.0", true, &[SUMS.into(), "Vestigo-0.3.0-setup.exe".into()]),
        listed("v0.4.0", false, &[SUMS.into(), "Vestigo-0.4.0-arm64-setup.exe".into()]),
        listed("v0.5.0", false, &["Vestigo-0.5.0-setup.exe".into()]),
    ];
    let release = found(&releases, X64_SETUP, "0.1.0").expect("an update");
    assert_eq!(release.version, version("0.2.0"));
    assert_eq!(release.file.name, "Vestigo-0.2.0-setup.exe");
    assert_eq!(release.file.browser_download_url, url("v0.2.0", "Vestigo-0.2.0-setup.exe"));
    assert_eq!(release.sums, url("v0.2.0", SUMS));
}

#[test]
fn each_install_gets_the_file_of_its_build() {
    let cases = [
        (Arch::X64, Form::Setup, "Vestigo-0.2.0-setup.exe"),
        (Arch::X86, Form::Setup, "Vestigo-0.2.0-setup.exe"),
        (Arch::Arm64, Form::Setup, "Vestigo-0.2.0-arm64-setup.exe"),
        (Arch::X64, Form::Portable, "Vestigo-0.2.0-portable.zip"),
        (Arch::X86, Form::Portable, "Vestigo-0.2.0-portable.zip"),
        (Arch::Arm64, Form::Portable, "Vestigo-0.2.0-arm64-portable.zip"),
    ];
    for (arch, form, name) in cases {
        let install = Install { arch, form, channel: Channel::Stable };
        let release = found(&[full("v0.2.0")], install, "0.1.0").expect("an update");
        assert_eq!(release.form, form);
        assert_eq!(release.file.name, name, "{arch:?} {form:?}");
    }
}

#[test]
fn stable_install_ignores_previews() {
    let stable = Install { channel: Channel::Stable, ..X64_SETUP };
    let releases = [full("v1.1.0-beta"), full("v1.0.1"), full("v1.0.0")];
    assert_eq!(found(&releases, stable, "1.0.0").map(|release| release.version), Some(version("1.0.1")));
    assert_eq!(found(&[full("v1.1.0-beta")], stable, "1.0.0"), None);
}

#[test]
fn preview_install_moves_to_a_newer_stable_release() {
    let releases = [full("v0.9.1-alpha"), full("v1.0.0"), full("v1.0.0-rc.2")];
    assert_eq!(found(&releases, X64_SETUP, "0.9.1-alpha").map(|release| release.version), Some(version("1.0.0")));
}

#[test]
fn nothing_is_offered_without_a_newer_release() {
    let releases = [full("v0.2.0"), full("v0.1.0")];
    assert_eq!(found(&releases, X64_SETUP, "0.2.0"), None);
    assert_eq!(found(&[], X64_SETUP, "0.2.0"), None);
    assert!(newest(br#"{"message": "API rate limit exceeded"}"#, X64_SETUP, &version("0.1.0")).is_err());
}

#[test]
fn an_installed_copy_is_told_by_its_uninstaller() {
    let scratch = Scratch::new("form");
    let installed = scratch.file("Program Files/Vestigo/Vestigo.exe", "app");
    scratch.file("Program Files/Vestigo/unins000.exe", "uninstaller");
    let portable = scratch.file("Мои программы/Vestigo/Vestigo.exe", "app");
    assert_eq!(Form::of(&installed), Form::Setup);
    assert_eq!(Form::of(&portable), Form::Portable);
}

#[test]
fn only_a_file_matching_its_checksum_passes() {
    let scratch = Scratch::new("verify");
    let name = "Vestigo-9.0.0-setup.exe";
    let file = scratch.file(name, "setup");
    let right = format!("{OTHER_HASH}  other.zip\n{}  {name}\n", SETUP_HASH.to_uppercase());
    assert_eq!(verify(&right, name, &file), Ok(()));
    let binary = format!("{SETUP_HASH} *{name}\n");
    assert_eq!(verify(&binary, name, &file), Ok(()));
    assert_eq!(verify(&format!("{OTHER_HASH}  {name}\n"), name, &file), Err(Trouble::Damaged));
    let elsewhere = format!("{SETUP_HASH}  another-file.exe\n");
    assert_eq!(verify(&elsewhere, name, &file), Err(Trouble::Damaged));
}

#[test]
fn portable_update_replaces_the_files_and_the_app_keeps_its_name() {
    let scratch = Scratch::new("portable");
    let exe = scratch.file("Мой Vestigo/Vestigo (2).exe", "old app");
    scratch.file("Мой Vestigo/Vestigo-x86.exe", "old x86");
    scratch.file("Мой Vestigo/LICENSE.txt", "old license");
    let zip = scratch.archive(
        "update.zip",
        &[
            ("Vestigo/Vestigo.exe", "new app"),
            ("Vestigo/Vestigo-x86.exe", "new x86"),
            ("Vestigo/LICENSE.txt", "new license"),
            ("Vestigo/THIRD-PARTY-NOTICES.txt", "notices"),
        ],
    );
    replace(&zip, &exe, Arch::X64).expect("replaced");
    assert_eq!(scratch.read("Мой Vestigo/Vestigo (2).exe"), "new app");
    assert_eq!(scratch.read("Мой Vestigo/Vestigo-x86.exe"), "new x86");
    assert_eq!(scratch.read("Мой Vestigo/LICENSE.txt"), "new license");
    assert_eq!(scratch.read("Мой Vestigo/THIRD-PARTY-NOTICES.txt"), "notices");

    sweep_beside(&exe);
    let left = ["LICENSE.txt", "THIRD-PARTY-NOTICES.txt", "Vestigo (2).exe", "Vestigo-x86.exe"];
    assert_eq!(scratch.names("Мой Vestigo"), left);
}

#[test]
fn a_32_bit_copy_takes_the_32_bit_build_whatever_its_name() {
    let scratch = Scratch::new("x86");
    let exe = scratch.file("app/Vestigo.exe", "old x86");
    let zip =
        scratch.archive("update.zip", &[("Vestigo/Vestigo.exe", "new x64"), ("Vestigo/Vestigo-x86.exe", "new x86")]);
    replace(&zip, &exe, Arch::X86).expect("replaced");
    assert_eq!(scratch.read("app/Vestigo.exe"), "new x86");
    assert!(!scratch.0.join("app/Vestigo-x86.exe").exists(), "the app keeps its own file name");
}

#[test]
fn portable_update_without_the_app_changes_nothing() {
    let scratch = Scratch::new("missing");
    let exe = scratch.file("app/Vestigo.exe", "old app");
    scratch.file("app/LICENSE.txt", "old license");
    let zip = scratch.archive("update.zip", &[("Vestigo/LICENSE.txt", "new license")]);
    assert_eq!(replace(&zip, &exe, Arch::X64), Err(Trouble::Damaged));
    assert_eq!(scratch.read("app/Vestigo.exe"), "old app");
    assert_eq!(scratch.read("app/LICENSE.txt"), "old license");
    assert_eq!(scratch.names("app"), ["LICENSE.txt", "Vestigo.exe"], "no leftovers");
}

#[test]
fn a_broken_archive_changes_nothing() {
    let scratch = Scratch::new("broken");
    let exe = scratch.file("app/Vestigo.exe", "old app");
    let zip = scratch.file("update.zip", "not a zip");
    assert_eq!(replace(&zip, &exe, Arch::X64), Err(Trouble::Damaged));
    assert_eq!(scratch.read("app/Vestigo.exe"), "old app");
    assert_eq!(scratch.names("app"), ["Vestigo.exe"]);
}
