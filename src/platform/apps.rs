use super::ensure_com;
use crate::index::StoreApp;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT, SHCreateItemFromParsingName,
    SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::core::HSTRING;

pub(super) const APPS_FOLDER: &str = "shell:AppsFolder\\";
pub(super) const PACKAGED: char = '!';

pub(crate) fn is_store_app(path: &str) -> bool {
    path.get(..APPS_FOLDER.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(APPS_FOLDER))
}

pub(crate) fn store_apps() -> Vec<StoreApp> {
    ensure_com();
    let items = unsafe {
        SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
            .and_then(|folder| folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems))
    };
    let items = match items {
        Ok(items) => items,
        Err(error) => {
            log::warn!("Couldn't list the Store apps: {error}");
            return Vec::new();
        }
    };
    let mut apps = Vec::new();
    let mut slot = [None];
    while unsafe { items.Next(&mut slot, None) }.is_ok()
        && let Some(item) = slot[0].take()
    {
        let Some(id) = display_name(&item, SIGDN_PARENTRELATIVEPARSING).filter(|id| id.contains(PACKAGED)) else {
            continue;
        };
        if let Some(name) = display_name(&item, SIGDN_NORMALDISPLAY) {
            apps.push(StoreApp { name, path: format!("{APPS_FOLDER}{id}") });
        }
    }
    apps
}

pub(crate) fn store_app_name(path: &str) -> Option<String> {
    ensure_com();
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None) }.ok()?;
    display_name(&item, SIGDN_NORMALDISPLAY)
}

fn display_name(item: &IShellItem, kind: SIGDN) -> Option<String> {
    let name = unsafe { item.GetDisplayName(kind) }.ok()?;
    let text = unsafe { name.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(name.0 as _)) };
    text.filter(|text| !text.is_empty())
}
