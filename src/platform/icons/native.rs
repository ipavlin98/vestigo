use super::package::{expanded, text};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, PROPERTYKEY};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{
    EnumResourceNamesW, FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW,
    LoadResource, LockResource, SizeofResource,
};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_DEFAULTICON, AssocQueryStringW, BHID_SFUIObject, GIL_NOTFILENAME, IExtractIconW, IShellItem,
    IShellItem2, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::RT_GROUP_ICON;
use windows::core::{BOOL, GUID, HSTRING, Interface, PCWSTR, PWSTR};

const LOCATION_LENGTH: usize = 1024;
const ICON_FILE: &str = "ico";
const HEADER: usize = 6;
const COUNT_AT: usize = 4;
const FILE_ENTRY: usize = 16;
const RESOURCE_ENTRY: usize = 14;
const DEPTH_AT: usize = 6;
const FULL_SIDE: u32 = 256;
const LINK_TARGET: PROPERTYKEY = PROPERTYKEY { fmtid: GUID::from_u128(0xb9b4b3fc_2b51_4a42_b5d8_324146afcf25), pid: 2 };

pub(super) fn largest(item: &IShellItem) -> Option<u32> {
    let (path, index) = location(item)?;
    let is_icon_file =
        std::path::Path::new(&path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(ICON_FILE));
    if is_icon_file {
        return largest_in(&std::fs::read(&path).ok()?, FILE_ENTRY);
    }
    let module = unsafe {
        LoadLibraryExW(&HSTRING::from(path), None, LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE)
    }
    .ok()?;
    let side = largest_group(module, index);
    unsafe {
        let _ = FreeLibrary(module);
    }
    side
}

fn location(item: &IShellItem) -> Option<(String, i32)> {
    let extract: IExtractIconW = unsafe { item.BindToHandler(None, &BHID_SFUIObject) }.ok()?;
    let mut file = [0u16; LOCATION_LENGTH];
    let (mut index, mut flags) = (0, 0);
    unsafe { extract.GetIconLocation(Default::default(), &mut file, &mut index, &mut flags) }.ok()?;
    let length = file.iter().position(|&unit| unit == 0)?;
    let path = expanded(&String::from_utf16(file.get(..length)?).ok()?);
    if !std::path::Path::new(&path).is_file() {
        return default_icon(item);
    }
    let own = flags & GIL_NOTFILENAME != 0;
    Some((path, if own { 0 } else { index }))
}

fn default_icon(item: &IShellItem) -> Option<(String, i32)> {
    let item: IShellItem2 = item.cast().ok()?;
    let path = text(&item, &LINK_TARGET).or_else(|| file_path(&item))?;
    let extension = std::path::Path::new(&path).extension()?.to_str()?;
    let mut value = [0u16; LOCATION_LENGTH];
    let mut length = LOCATION_LENGTH as u32;
    let class = HSTRING::from(format!(".{extension}"));
    unsafe {
        AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_DEFAULTICON, &class, None, Some(PWSTR(value.as_mut_ptr())), &mut length)
    }
    .ok()
    .ok()?;
    let length = value.iter().position(|&unit| unit == 0)?;
    let (path, index) = parsed_location(&String::from_utf16(value.get(..length)?).ok()?);
    std::path::Path::new(&path).is_file().then_some((path, index))
}

fn file_path(item: &IShellItem2) -> Option<String> {
    let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?;
    let path = unsafe { name.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(name.0 as _)) };
    path
}

fn parsed_location(value: &str) -> (String, i32) {
    let (path, index) = match value.rsplit_once(',').map(|(path, index)| (path, index.trim().parse())) {
        Some((path, Ok(index))) => (path, index),
        Some((_, Err(_))) | None => (value, 0),
    };
    (expanded(path.trim().trim_matches('"')), index)
}

fn largest_group(module: HMODULE, index: i32) -> Option<u32> {
    let name = match usize::try_from(index) {
        Ok(position) => *names(module).get(position)?,
        Err(_) => index.unsigned_abs() as usize,
    };
    unsafe {
        let resource = FindResourceW(Some(module), PCWSTR(name as *const u16), RT_GROUP_ICON);
        if resource.is_invalid() {
            return None;
        }
        let size = SizeofResource(Some(module), resource) as usize;
        let data = LockResource(LoadResource(Some(module), resource).ok()?);
        if data.is_null() {
            return None;
        }
        largest_in(std::slice::from_raw_parts(data.cast::<u8>(), size), RESOURCE_ENTRY)
    }
}

fn names(module: HMODULE) -> Vec<usize> {
    unsafe extern "system" fn collect(_: HMODULE, _: PCWSTR, name: PCWSTR, names: isize) -> BOOL {
        if let Some(names) = unsafe { (names as *mut Vec<usize>).as_mut() } {
            names.push(name.0 as usize);
        }
        true.into()
    }
    let mut names: Vec<usize> = Vec::new();
    unsafe {
        let _ = EnumResourceNamesW(Some(module), RT_GROUP_ICON, Some(collect), &mut names as *mut Vec<usize> as _);
    }
    names
}

fn largest_in(directory: &[u8], entry: usize) -> Option<u32> {
    let count = u16::from_le_bytes(directory.get(COUNT_AT..HEADER)?.try_into().ok()?) as usize;
    (0..count)
        .map(|position| {
            let record = directory.get(HEADER + position * entry..)?;
            let width = *record.first()?;
            let depth = u16::from_le_bytes(record.get(DEPTH_AT..DEPTH_AT + 2)?.try_into().ok()?);
            Some((depth, if width == 0 { FULL_SIDE } else { u32::from(width) }))
        })
        .collect::<Option<Vec<(u16, u32)>>>()?
        .into_iter()
        .max()
        .map(|(_, side)| side)
}

#[cfg(test)]
mod tests {
    use super::{DEPTH_AT, FILE_ENTRY, RESOURCE_ENTRY, largest_in, parsed_location};

    fn directory(widths: &[u8], entry: usize) -> Vec<u8> {
        let images: Vec<(u8, u8)> = widths.iter().map(|&width| (width, 32)).collect();
        directory_of(&images, entry)
    }

    fn directory_of(images: &[(u8, u8)], entry: usize) -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, images.len() as u8, 0];
        for &(width, depth) in images {
            let mut record = vec![0; entry];
            record[0] = width;
            record[1] = width;
            record[DEPTH_AT] = depth;
            bytes.extend(record);
        }
        bytes
    }

    #[test]
    fn the_largest_size_wins_and_zero_means_full_size() {
        assert_eq!(largest_in(&directory(&[16, 32], RESOURCE_ENTRY), RESOURCE_ENTRY), Some(32));
        assert_eq!(largest_in(&directory(&[48, 0, 16], RESOURCE_ENTRY), RESOURCE_ENTRY), Some(256));
        assert_eq!(largest_in(&directory(&[24, 64], FILE_ENTRY), FILE_ENTRY), Some(64));
    }

    #[test]
    fn a_larger_image_with_fewer_colors_does_not_count() {
        let images = [(16, 32), (32, 32), (48, 8), (16, 8)];
        assert_eq!(largest_in(&directory_of(&images, RESOURCE_ENTRY), RESOURCE_ENTRY), Some(32));
        let old = [(32, 4), (16, 4), (48, 8)];
        assert_eq!(largest_in(&directory_of(&old, RESOURCE_ENTRY), RESOURCE_ENTRY), Some(48));
    }

    #[test]
    fn default_icons_name_a_file_and_an_index() {
        let location = |path: &str, index| (path.to_owned(), index);
        assert_eq!(
            parsed_location("C:\\Program Files\\AutoHotkey\\AutoHotkey.exe,1"),
            location("C:\\Program Files\\AutoHotkey\\AutoHotkey.exe", 1)
        );
        assert_eq!(parsed_location("\"C:\\a,b\\x.dll\", -102"), location("C:\\a,b\\x.dll", -102));
        assert_eq!(parsed_location("C:\\a,b\\x.ico"), location("C:\\a,b\\x.ico", 0));
    }

    #[test]
    fn a_truncated_directory_tells_nothing() {
        let mut bytes = directory(&[16, 32], RESOURCE_ENTRY);
        bytes.truncate(bytes.len() - RESOURCE_ENTRY);
        assert_eq!(largest_in(&bytes, RESOURCE_ENTRY), None);
        assert_eq!(largest_in(&[0, 0, 1], RESOURCE_ENTRY), None);
        assert_eq!(largest_in(&directory(&[], FILE_ENTRY), FILE_ENTRY), None);
    }
}
