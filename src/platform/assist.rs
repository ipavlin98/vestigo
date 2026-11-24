use super::apps::APPS_FOLDER;
use super::mru::Key;
use super::shell::known_folder;
use super::{ensure_com, unix_seconds};
use crate::settings::Launch;
use rustc_hash::FxHashMap;
use std::ops::Range;
use std::path::Path;
use windows::core::GUID;

const COUNTS: [&str; 2] = [
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist\\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\\Count",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\UserAssist\\{F4E57C4B-2036-45F0-A9AB-443BCFE33D9F}\\Count",
];
const RUNS: Range<usize> = 4..8;
const LAST_RUN: Range<usize> = 60..68;
const GUID_LENGTH: usize = 36;
const LINK: &str = "://";
const SESSION: &str = "UEME_";
const PROGRAM_ROOTS: [&str; 4] = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "windir"];
const USER_PROGRAMS: [&str; 2] = ["Programs", "Microsoft\\WindowsApps"];
const USER_ROOT: &str = "LOCALAPPDATA";

/// Programs Windows saw started from the Start menu, the taskbar and Explorer (UserAssist).
pub(crate) fn assisted_launches() -> Vec<Launch> {
    ensure_com();
    let mut folders: FxHashMap<String, Option<String>> = FxHashMap::default();
    let mut folder = |id: &str| folders.entry(id.to_owned()).or_insert_with(|| folder_path(id)).clone();
    COUNTS
        .iter()
        .filter_map(|count| Key::open(count))
        .flat_map(|key| key.values())
        .filter_map(|(name, data)| launch(&rot13(&name), &data, &mut folder))
        .collect()
}

/// Folders programs are installed in: starting a program there says nothing about the folder.
pub(crate) fn program_folders() -> Vec<String> {
    let roots = PROGRAM_ROOTS.iter().filter_map(std::env::var_os).map(std::path::PathBuf::from);
    let user = std::env::var_os(USER_ROOT).map(std::path::PathBuf::from);
    let user = USER_PROGRAMS.iter().filter_map(|folder| Some(user.as_ref()?.join(folder)));
    roots
        .chain(user)
        .filter_map(|path| Some(format!("{}\\", path.to_str()?.trim_end_matches('\\')).to_lowercase()))
        .collect()
}

fn launch(name: &str, data: &[u8], folder: &mut impl FnMut(&str) -> Option<String>) -> Option<Launch> {
    let count = u32::from_le_bytes(data.get(RUNS)?.try_into().ok()?);
    let at = unix_seconds(u64::from_le_bytes(data.get(LAST_RUN)?.try_into().ok()?))?;
    let path = path(name, folder)?;
    Some(Launch { path: path.to_lowercase(), at, count: count.max(1) })
}

fn path(name: &str, folder: &mut impl FnMut(&str) -> Option<String>) -> Option<String> {
    if let Some(rest) = name.strip_prefix('{') {
        let id = rest.get(..GUID_LENGTH)?;
        let rest = rest.get(GUID_LENGTH..)?.strip_prefix('}')?;
        return Some(format!("{}{rest}", folder(id)?));
    }
    if Path::new(name).is_absolute() {
        return Some(name.to_owned());
    }
    let app_id = !name.contains('\\') && !name.contains(LINK) && !name.starts_with(SESSION);
    app_id.then(|| format!("{APPS_FOLDER}{name}"))
}

fn folder_path(id: &str) -> Option<String> {
    let id = GUID::try_from(id).ok()?;
    known_folder(&id)?.to_str().map(|path| path.trim_end_matches('\\').to_owned())
}

fn rot13(text: &str) -> String {
    let turn = |ch: char, base: u8| char::from((ch as u8 - base + 13) % 26 + base);
    text.chars()
        .map(|ch| match ch {
            'a'..='z' => turn(ch, b'a'),
            'A'..='Z' => turn(ch, b'A'),
            _ => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILETIME_2023: u64 = 133_400_000_000_000_000;
    const UNIX_2023: u64 = 1_695_526_400;

    fn record(runs: u32, ticks: u64) -> Vec<u8> {
        let mut data = vec![0u8; 72];
        data.splice(RUNS, runs.to_le_bytes());
        data.splice(LAST_RUN, ticks.to_le_bytes());
        data
    }

    fn programs(id: &str) -> Option<String> {
        (id == "A77F5D77-2E2B-44C3-A6A2-ABA601054A51").then(|| "C:\\Users\\Алекс\\Start Menu\\Programs".to_owned())
    }

    #[test]
    fn names_are_read_back_from_rot13() {
        assert_eq!(rot13("P:\\Gbbyf\\Pbqr.rkr"), "C:\\Tools\\Code.exe");
        assert_eq!(rot13(&rot13("Отчёт {1}.lnk")), "Отчёт {1}.lnk");
    }

    #[test]
    fn a_known_folder_prefix_becomes_its_path_and_the_last_run_its_time() {
        let name = "{A77F5D77-2E2B-44C3-A6A2-ABA601054A51}\\Accessories\\Paint.lnk";
        let found = launch(name, &record(3, FILETIME_2023), &mut programs);
        let expected = "c:\\users\\алекс\\start menu\\programs\\accessories\\paint.lnk";
        assert_eq!(found, Some(Launch { path: expected.into(), at: UNIX_2023, count: 3 }));
    }

    #[test]
    fn app_ids_paths_and_unknown_names_are_told_apart() {
        let calculator =
            launch("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", &record(1, FILETIME_2023), &mut programs);
        assert_eq!(
            calculator.map(|launch| launch.path),
            Some("shell:appsfolder\\microsoft.windowscalculator_8wekyb3d8bbwe!app".into())
        );
        assert_eq!(
            launch("D:\\Tools\\tool.exe", &record(0, FILETIME_2023), &mut programs).map(|launch| launch.count),
            Some(1)
        );
        assert_eq!(
            launch("com.squirrel.Discord.Discord", &record(2, FILETIME_2023), &mut programs).map(|launch| launch.path),
            Some("shell:appsfolder\\com.squirrel.discord.discord".into())
        );
        assert_eq!(launch("steam://rungameid/2694490", &record(5, FILETIME_2023), &mut programs), None);
        assert_eq!(launch("UEME_CTLSESSION", &record(5, FILETIME_2023), &mut programs), None);
        assert_eq!(
            launch("{00000000-0000-0000-0000-000000000000}\\x.exe", &record(5, FILETIME_2023), &mut programs),
            None
        );
    }

    #[test]
    fn entries_without_a_run_time_or_with_short_data_are_skipped() {
        assert_eq!(launch("D:\\Tools\\tool.exe", &record(4, 0), &mut programs), None);
        assert_eq!(launch("D:\\Tools\\tool.exe", &[0u8; 16], &mut programs), None);
    }
}
