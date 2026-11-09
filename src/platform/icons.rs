mod native;
mod package;

use super::{ensure_com, is_store_app};
use package::Package;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP,
};
use windows::Win32::UI::Shell::{
    ILFree, IShellItem, IShellItemImageFactory, SHCreateItemFromIDList, SHCreateItemFromParsingName,
    SHSimpleIDListFromPath, SIIGBF, SIIGBF_ICONONLY, SIIGBF_INCACHEONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
};
use windows::core::{HSTRING, Interface};

const GENERIC_FILE: &str = "C:\\file";
const FOLDER_SAMPLE: &str = "SystemRoot";
const OPAQUE: u8 = 255;
const SOURCE_PIXELS: u32 = 256;
const HINTED_PIXELS: u32 = 16;
const UNFRAMED_PIXELS: u32 = 48;

const START_MENUS: [&str; 2] = ["ProgramData", "APPDATA"];
const START_MENU_PROGRAMS: &str = "Microsoft\\Windows\\Start Menu\\Programs";
const SHORTCUT: &str = "lnk";
const SHORTCUT_DEPTH: usize = 3;

const OWN_ICONS: [&str; 7] = ["exe", "lnk", "ico", "url", "msc", "cpl", "appref-ms"];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum IconSource {
    Folder,
    Extension(String),
    File(String),
}

impl IconSource {
    pub(crate) fn for_path(path: &str, dir: bool) -> Self {
        if is_store_app(path) {
            return Self::File(path.to_owned());
        }
        if dir {
            return if path.ends_with('\\') { Self::File(path.to_owned()) } else { Self::Folder };
        }
        let extension =
            std::path::Path::new(path).extension().map(|extension| extension.to_string_lossy().to_lowercase());
        match extension {
            Some(extension) if !OWN_ICONS.contains(&extension.as_str()) => Self::Extension(extension),
            _ => Self::File(path.to_owned()),
        }
    }

    fn may_be_packaged(&self) -> bool {
        match self {
            Self::File(path) => {
                is_store_app(path)
                    || std::path::Path::new(path)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case(SHORTCUT))
            }
            Self::Folder | Self::Extension(_) => false,
        }
    }
}

pub(crate) struct Icon {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bgra: Vec<u8>,
    pub(crate) small: bool,
}

pub(crate) fn load(source: &IconSource, pixels: u32) -> Option<Icon> {
    ensure_com();
    let item = shell_item(source)?;
    if let Some(package) = source.may_be_packaged().then(|| Package::of(&item)).flatten() {
        return package.logo(pixels).or_else(|| image(&item, pixels));
    }
    if pixels <= HINTED_PIXELS {
        return image(&item, pixels);
    }
    let source = pixels.max(SOURCE_PIXELS);
    match native::largest(&item).filter(|&side| side < source).map(|side| side.min(UNFRAMED_PIXELS)) {
        Some(side) if side < pixels => image(&item, source).map(|icon| Icon { small: true, ..icon }),
        Some(side) => image(&item, side),
        None => image(&item, source),
    }
}

pub(crate) fn thumbnail(path: &str, side: u32) -> Option<Icon> {
    ensure_com();
    let item = shell_item(&IconSource::File(path.to_owned()))?;
    render(&item, side, SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT)
}

pub(crate) fn cached_thumbnail(path: &str, sides: &[u32]) -> Option<(u32, Icon)> {
    ensure_com();
    let item = shell_item(&IconSource::File(path.to_owned()))?;
    let flags = SIIGBF_THUMBNAILONLY | SIIGBF_RESIZETOFIT | SIIGBF_INCACHEONLY;
    sides.iter().find_map(|&side| Some((side, render(&item, side, flags)?)))
}

fn image(item: &IShellItem, pixels: u32) -> Option<Icon> {
    render(item, pixels, SIIGBF_ICONONLY)
}

fn render(item: &IShellItem, pixels: u32, flags: SIIGBF) -> Option<Icon> {
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let side = i32::try_from(pixels).ok()?;
    let bitmap = unsafe { factory.GetImage(SIZE { cx: side, cy: side }, flags) }.ok()?;
    let icon = read_image(bitmap);
    unsafe {
        let _ = DeleteObject(bitmap.into());
    }
    icon
}

pub(crate) fn app_shortcuts() -> Vec<String> {
    let mut shortcuts = Vec::new();
    for root in START_MENUS.iter().filter_map(std::env::var_os) {
        collect_shortcuts(&std::path::Path::new(&root).join(START_MENU_PROGRAMS), SHORTCUT_DEPTH, &mut shortcuts);
    }
    shortcuts
}

fn collect_shortcuts(folder: &std::path::Path, depth: usize, shortcuts: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() && depth > 0 => collect_shortcuts(&path, depth - 1, shortcuts),
            Ok(kind) if kind.is_file() && path.extension().is_some_and(|extension| extension == SHORTCUT) => {
                shortcuts.push(path.to_string_lossy().into_owned());
            }
            Ok(_) | Err(_) => {}
        }
    }
}

fn shell_item(source: &IconSource) -> Option<IShellItem> {
    let parsed = |path: HSTRING| unsafe { SHCreateItemFromParsingName(&path, None) }.ok();
    match source {
        IconSource::File(path) => parsed(HSTRING::from(path.as_str())),
        IconSource::Folder => parsed(HSTRING::from(std::env::var_os(FOLDER_SAMPLE)?.as_os_str())),
        IconSource::Extension(extension) => unsafe {
            let list = SHSimpleIDListFromPath(&HSTRING::from(format!("{GENERIC_FILE}.{extension}")));
            if list.is_null() {
                return None;
            }
            let item = SHCreateItemFromIDList(list).ok();
            ILFree(Some(list));
            item
        },
    }
}

pub(super) fn read_image(bitmap: HBITMAP) -> Option<Icon> {
    let mut info = BITMAP::default();
    let read =
        unsafe { GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some((&mut info as *mut BITMAP).cast())) };
    if read == 0 || info.bmWidth <= 0 || info.bmHeight <= 0 {
        return None;
    }
    let (width, height) = (info.bmWidth, info.bmHeight);
    let mut bgra = read_bits(bitmap, width, height)?;
    let pixels = bgra.as_chunks_mut::<4>().0;
    if pixels.iter().all(|pixel| pixel[3] == 0) {
        pixels.iter_mut().for_each(|pixel| pixel[3] = OPAQUE);
    }
    Some(Icon { width: width as u32, height: height as u32, bgra, small: false })
}

fn read_bits(bitmap: HBITMAP, width: i32, height: i32) -> Option<Vec<u8>> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    unsafe {
        let dc = CreateCompatibleDC(None);
        let lines =
            GetDIBits(dc, bitmap, 0, height as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        (lines == height).then_some(pixels)
    }
}
