use super::{ensure_com, unix_seconds};
use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, FILETIME};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{GPFIDL_DEFAULT, SHGetPathFromIDListEx};
use windows::core::{HSTRING, PWSTR};

const DIALOG_PICKS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\ComDlg32\\OpenSavePidlMRU";
const ORDER: &str = "MRUListEx";
const PICKS_PER_LIST: usize = 10;
const PATH_LENGTH: usize = 1024;
const NAME_LENGTH: usize = 1024;
const DATA_LENGTH: usize = 256;

pub(crate) struct Picked {
    pub(crate) paths: Vec<String>,
    pub(crate) at: u64,
}

pub(crate) fn dialog_picks(lists: &[String]) -> Vec<Picked> {
    ensure_com();
    lists
        .iter()
        .filter_map(|list| {
            let key = Key::open(&format!("{DIALOG_PICKS}\\{list}"))?;
            let paths = order(&key.value(ORDER)?)
                .take(PICKS_PER_LIST)
                .filter_map(|entry| item_path(&key.value(&entry.to_string())?))
                .collect();
            Some(Picked { paths, at: key.written()? })
        })
        .collect()
}

pub(super) struct Key(HKEY);

impl Key {
    pub(super) fn open(path: &str) -> Option<Self> {
        let mut key = HKEY::default();
        let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut key) };
        (opened == ERROR_SUCCESS).then_some(Self(key))
    }

    fn value(&self, name: &str) -> Option<Vec<u8>> {
        let name = HSTRING::from(name);
        let mut size = 0u32;
        let sized = unsafe { RegQueryValueExW(self.0, &name, None, None, None, Some(&mut size)) };
        if sized != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        let read = unsafe { RegQueryValueExW(self.0, &name, None, None, Some(data.as_mut_ptr()), Some(&mut size)) };
        data.truncate(size as usize);
        (read == ERROR_SUCCESS).then_some(data)
    }

    fn written(&self) -> Option<u64> {
        let mut time = FILETIME::default();
        let read = unsafe {
            RegQueryInfoKeyW(self.0, None, None, None, None, None, None, None, None, None, None, Some(&mut time))
        };
        let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        (read == ERROR_SUCCESS).then(|| unix_seconds(ticks)).flatten()
    }

    pub(super) fn values(&self) -> Vec<(String, Vec<u8>)> {
        let mut values = Vec::new();
        let mut name = [0u16; NAME_LENGTH];
        let mut data = [0u8; DATA_LENGTH];
        for index in 0u32.. {
            let mut name_length = name.len() as u32;
            let mut data_length = data.len() as u32;
            let read = unsafe {
                RegEnumValueW(
                    self.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_length,
                    None,
                    None,
                    Some(data.as_mut_ptr()),
                    Some(&mut data_length),
                )
            };
            if read == ERROR_NO_MORE_ITEMS {
                break;
            }
            let (Some(name), Some(data)) = (name.get(..name_length as usize), data.get(..data_length as usize)) else {
                continue;
            };
            if read == ERROR_SUCCESS {
                values.push((String::from_utf16_lossy(name), data.to_vec()));
            }
        }
        values
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn order(list: &[u8]) -> impl Iterator<Item = u32> + '_ {
    list.as_chunks::<4>().0.iter().map(|bytes| u32::from_le_bytes(*bytes)).take_while(|&entry| entry != u32::MAX)
}

fn item_path(item: &[u8]) -> Option<String> {
    if !terminated(item) {
        return None;
    }
    let mut buffer = [0u16; PATH_LENGTH];
    let found = unsafe { SHGetPathFromIDListEx(item.as_ptr().cast::<ITEMIDLIST>(), &mut buffer, GPFIDL_DEFAULT) };
    let length = buffer.iter().position(|&unit| unit == 0)?;
    found.as_bool().then(|| String::from_utf16_lossy(buffer.get(..length).unwrap_or_default()))
}

fn terminated(item: &[u8]) -> bool {
    let mut at = 0usize;
    while let Some(&[low, high]) = item.get(at..at + 2) {
        match u16::from_le_bytes([low, high]) {
            0 => return true,
            1 => return false,
            size => at += usize::from(size),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_list_ends_at_its_marker() {
        let list = [2, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 255, 255, 255, 255, 1, 0, 0, 0];
        assert_eq!(order(&list).collect::<Vec<_>>(), [2, 0, 7]);
        assert_eq!(order(&[3, 0, 0]).count(), 0);
    }

    #[test]
    fn only_item_lists_that_end_inside_their_bytes_are_read() {
        assert!(terminated(&[4, 0, 9, 9, 3, 0, 1, 0, 0]));
        assert!(terminated(&[0, 0]));
        assert!(!terminated(&[4, 0, 9, 9, 3, 0, 1]));
        assert!(!terminated(&[4, 0, 9, 9, 40, 0, 1, 2, 0, 0]));
        assert!(!terminated(&[1, 0, 0, 0]));
        assert!(!terminated(&[]));
    }
}
