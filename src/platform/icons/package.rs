use super::super::apps::{APPS_FOLDER, PACKAGED};
use super::Icon;
use image::RgbaImage;
use std::path::PathBuf;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{IShellItem, IShellItem2, SHCreateItemFromParsingName};
use windows::core::{GUID, HSTRING, Interface};

const TILE: GUID = GUID::from_u128(0x86d40b4d_9069_443c_819a_2a54090dccec);
const APP_MODEL: GUID = GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3);
const SMALL_LOGO: PROPERTYKEY = PROPERTYKEY { fmtid: TILE, pid: 2 };
const BACKGROUND: PROPERTYKEY = PROPERTYKEY { fmtid: TILE, pid: 4 };
const APP_ID: PROPERTYKEY = PROPERTYKEY { fmtid: APP_MODEL, pid: 5 };
const INSTALL_PATH: PROPERTYKEY = PROPERTYKEY { fmtid: APP_MODEL, pid: 15 };
const TARGET_SIZE: &str = "targetsize-";
const SCALE: &str = "scale-";
const UNPLATED: &str = "altform-unplated";
const BASE_SCALE: u32 = 100;
const VISIBLE: u8 = 32;
const WHITE: u8 = 0xF0;
const OPAQUE: u8 = u8::MAX;
const GLYPH_SHARE: f32 = 2.0 / 3.0;

pub(super) struct Package {
    logo: PathBuf,
    plate: Option<[u8; 3]>,
}

impl Package {
    pub(super) fn of(item: &IShellItem) -> Option<Self> {
        let item: IShellItem2 = item.cast().ok()?;
        let app = match text(&item, &INSTALL_PATH) {
            Some(_) => item,
            None => {
                let id = text(&item, &APP_ID).filter(|id| id.contains(PACKAGED))?;
                let path = HSTRING::from(format!("{APPS_FOLDER}{id}"));
                unsafe { SHCreateItemFromParsingName::<_, _, IShellItem2>(&path, None) }.ok()?
            }
        };
        let folder = text(&app, &INSTALL_PATH)?;
        let logo = PathBuf::from(expanded(&folder)).join(expanded(&text(&app, &SMALL_LOGO)?));
        let plate = unsafe { app.GetUInt32(&BACKGROUND) }.ok().and_then(plate);
        Some(Self { logo, plate })
    }

    pub(super) fn logo(&self, pixels: u32) -> Option<Icon> {
        let folder = self.logo.parent()?;
        let stem = self.logo.file_stem()?.to_str()?;
        let extension = self.logo.extension()?.to_str()?;
        let names = std::fs::read_dir(folder).ok()?.flatten().filter_map(|entry| entry.file_name().into_string().ok());
        ranked(names, stem, extension, pixels).into_iter().find_map(|(name, asset)| {
            let image = image::ImageReader::open(folder.join(name)).ok()?.with_guessed_format().ok()?.decode().ok()?;
            let image = image.into_rgba8();
            if image.width() != image.height() || asset.unplated && white_glyph(&image) {
                return None;
            }
            let plate = self.plate.filter(|_| asset.plated());
            let image = if plate.is_some() { framed(&image) } else { image };
            let bgra = image.pixels().flat_map(|pixel| bgra(pixel.0, plate)).collect();
            Some(Icon { width: image.width(), height: image.height(), bgra, small: false })
        })
    }
}

pub(super) fn text(item: &IShellItem2, key: &PROPERTYKEY) -> Option<String> {
    let value = unsafe { item.GetString(key) }.ok()?;
    let text = unsafe { value.to_string() }.ok();
    unsafe { CoTaskMemFree(Some(value.0 as _)) };
    text.filter(|text| !text.is_empty())
}

fn plate(color: u32) -> Option<[u8; 3]> {
    let [red, green, blue, alpha] = color.to_le_bytes();
    (alpha != 0).then_some([red, green, blue])
}

fn framed(image: &RgbaImage) -> RgbaImage {
    let visible = image.enumerate_pixels().filter(|(_, _, pixel)| pixel.0[3] >= VISIBLE);
    let Some((left, top, right, bottom)) = visible.fold(None, |bounds, (x, y, _)| match bounds {
        None => Some((x, y, x, y)),
        Some((left, top, right, bottom)) => Some((left.min(x), top.min(y), right.max(x), bottom.max(y))),
    }) else {
        return image.clone();
    };
    let glyph = (right - left).max(bottom - top) + 1;
    let side = (glyph as f32 / GLYPH_SHARE).round() as u32;
    let origin = |low: u32, high: u32| (i64::from(low) + i64::from(high) + 1 - i64::from(side)).div_euclid(2);
    let (x0, y0) = (origin(left, right), origin(top, bottom));
    RgbaImage::from_fn(side, side, |x, y| {
        let at = |offset: i64, position: u32| u32::try_from(offset + i64::from(position)).ok();
        at(x0, x)
            .zip(at(y0, y))
            .and_then(|(x, y)| image.get_pixel_checked(x, y))
            .copied()
            .unwrap_or(image::Rgba([0; 4]))
    })
}

fn bgra([red, green, blue, alpha]: [u8; 4], plate: Option<[u8; 3]>) -> [u8; 4] {
    let Some(plate) = plate else { return [blue, green, red, alpha] };
    let over = |channel: u8, under: u8| {
        let mixed = u32::from(channel) * u32::from(alpha) + u32::from(under) * u32::from(OPAQUE - alpha);
        (mixed / u32::from(OPAQUE)) as u8
    };
    [over(blue, plate[2]), over(green, plate[1]), over(red, plate[0]), OPAQUE]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Size {
    Target(u32),
    Scale(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Asset {
    size: Size,
    unplated: bool,
}

impl Asset {
    fn parse(qualifiers: &str) -> Option<Self> {
        let (mut size, mut unplated) = (None, false);
        for qualifier in qualifiers.split('_').filter(|qualifier| !qualifier.is_empty()) {
            let qualifier = qualifier.to_ascii_lowercase();
            let parsed = if let Some(pixels) = qualifier.strip_prefix(TARGET_SIZE) {
                Size::Target(pixels.parse().ok()?)
            } else if let Some(scale) = qualifier.strip_prefix(SCALE) {
                Size::Scale(scale.parse().ok()?)
            } else if qualifier == UNPLATED {
                unplated = true;
                continue;
            } else {
                return None;
            };
            if size.replace(parsed).is_some() {
                return None;
            }
        }
        Some(Self { size: size.unwrap_or(Size::Scale(BASE_SCALE)), unplated })
    }

    fn plated(self) -> bool {
        !self.unplated
    }

    fn rank(self, pixels: u32) -> (bool, bool, bool, u32) {
        match self.size {
            Size::Target(size) if size >= pixels => (false, self.plated(), false, size),
            Size::Target(size) => (true, self.plated(), false, u32::MAX - size),
            Size::Scale(scale) => (false, self.plated(), true, u32::MAX - scale),
        }
    }
}

pub(super) fn expanded(path: &str) -> String {
    let pieces: Vec<&str> = path.split('%').collect();
    let mut expanded = String::with_capacity(path.len());
    for (index, piece) in pieces.iter().enumerate() {
        let name = index % 2 == 1;
        let closed = name && index + 1 < pieces.len();
        match std::env::var(piece).ok().filter(|_| closed) {
            Some(value) => expanded.push_str(&value),
            None if closed => expanded.push_str(&format!("%{piece}%")),
            None if name => expanded.push_str(&format!("%{piece}")),
            None => expanded.push_str(piece),
        }
    }
    expanded
}

fn white_glyph(image: &RgbaImage) -> bool {
    image.pixels().all(|pixel| {
        let [red, green, blue, alpha] = pixel.0;
        alpha < VISIBLE || red.min(green).min(blue) >= WHITE
    })
}

fn ranked(names: impl Iterator<Item = String>, stem: &str, extension: &str, pixels: u32) -> Vec<(String, Asset)> {
    let mut assets: Vec<(String, Asset)> = names
        .filter_map(|name| {
            let named = name.get(..stem.len()).is_some_and(|start| start.eq_ignore_ascii_case(stem));
            let rest = name.get(stem.len()..).filter(|_| named)?.strip_prefix('.')?;
            let (qualifiers, end) = rest.rsplit_once('.').unwrap_or(("", rest));
            if !end.eq_ignore_ascii_case(extension) {
                return None;
            }
            let asset = Asset::parse(qualifiers)?;
            Some((name, asset))
        })
        .collect();
    assets.sort_by_key(|(_, asset)| asset.rank(pixels));
    assets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chosen(names: &[&str], pixels: u32) -> Option<String> {
        let ranked = ranked(names.iter().map(|name| (*name).to_owned()), "AppList", "png", pixels);
        ranked.into_iter().next().map(|(name, _)| name)
    }

    #[test]
    fn unplated_logos_of_the_nearest_larger_size_win() {
        let names = [
            "AppList.scale-200.png",
            "AppList.targetsize-32.png",
            "AppList.targetsize-48.png",
            "AppList.targetsize-32_altform-unplated.png",
            "AppList.targetsize-48_altform-unplated.png",
            "AppList.targetsize-256_altform-unplated.png",
            "AppList.targetsize-40_altform-unplated_contrast-black.png",
            "AppList.targetsize-40_altform-lightunplated.png",
            "Other.targetsize-40_altform-unplated.png",
        ];
        assert_eq!(chosen(&names, 40).as_deref(), Some("AppList.targetsize-48_altform-unplated.png"));
        assert_eq!(chosen(&names, 32).as_deref(), Some("AppList.targetsize-32_altform-unplated.png"));
    }

    #[test]
    fn plated_logos_come_before_too_small_unplated_ones() {
        let names = ["applist.TARGETSIZE-16_ALTFORM-UNPLATED.PNG", "AppList.targetsize-48.png", "AppList.png"];
        assert_eq!(chosen(&names, 40).as_deref(), Some("AppList.targetsize-48.png"));
        assert_eq!(chosen(&names[..1], 40).as_deref(), Some(names[0]));
        assert_eq!(chosen(&["AppList.png", "AppList.scale-150.png"], 40).as_deref(), Some("AppList.scale-150.png"));
        let unplated =
            ["AppList.scale-400.png", "AppList.altform-unplated.png", "AppList.targetsize-16_altform-unplated.png"];
        assert_eq!(chosen(&unplated, 40).as_deref(), Some("AppList.altform-unplated.png"));
        assert_eq!(chosen(&["AppList.jpg", "AppListLarge.png"], 40), None);
    }

    #[test]
    fn environment_variables_in_package_paths_expand() {
        let windows = std::env::var("windir").expect("windir is set");
        assert_eq!(expanded("%windir%\\Panel\\%nope_vestigo%\\50%"), format!("{windows}\\Panel\\%nope_vestigo%\\50%"));
    }

    #[test]
    fn white_glyphs_are_told_from_colored_logos() {
        let glyph =
            RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x < 2 { [255, 255, 255, 255] } else { [9, 9, 9, 0] }));
        assert!(white_glyph(&glyph));
        let logo =
            RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x < 2 { [255, 255, 255, 255] } else { [0, 120, 212, 255] }));
        assert!(!white_glyph(&logo));
    }

    #[test]
    fn plated_glyphs_take_two_thirds_of_the_tile_centered() {
        let glyph = |x: u32, y: u32| (4..14).contains(&x) && (2..12).contains(&y);
        let asset =
            RgbaImage::from_fn(16, 16, |x, y| image::Rgba(if glyph(x, y) { [255; 4] } else { [255, 255, 255, 0] }));
        let tile = framed(&asset);
        assert_eq!(tile.dimensions(), (15, 15));
        let visible: Vec<u32> = (0..15).filter(|&x| tile.get_pixel(x, 7).0[3] > 0).collect();
        assert_eq!(visible, (3..13).collect::<Vec<_>>());
        let rows: Vec<u32> = (0..15).filter(|&y| tile.get_pixel(7, y).0[3] > 0).collect();
        assert_eq!(rows, (3..13).collect::<Vec<_>>());
        let edge_to_edge = RgbaImage::from_pixel(9, 9, image::Rgba([255; 4]));
        assert_eq!(framed(&edge_to_edge).dimensions(), (14, 14));
    }

    #[test]
    fn plated_logos_are_laid_on_the_tile_color() {
        assert_eq!(plate(0xFFD4_7800), Some([0x00, 0x78, 0xD4]));
        assert_eq!(plate(0x00D4_7800), None);
        assert_eq!(bgra([255, 255, 255, 0], Some([0, 120, 212])), [212, 120, 0, 255]);
        assert_eq!(bgra([255, 255, 255, 255], Some([0, 120, 212])), [255, 255, 255, 255]);
        assert_eq!(bgra([10, 20, 30, 40], None), [30, 20, 10, 40]);
    }
}
