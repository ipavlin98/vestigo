use super::super::icons;
use super::super::results::icon_key;
use crate::i18n::Stamp;
use crate::index::Hit;
use crate::platform;
use gpui::{RenderImage, SharedString};
use std::io::Read;
use std::sync::Arc;

pub(super) const ICON: f32 = 64.0;
const PICTURE: f32 = 640.0;
pub(super) const TEXT_BYTES: u64 = 64 * 1024;
const TEXT_LINES: usize = 400;
const TAB: &str = "    ";
const CACHED_SIDES: [u32; 2] = [256, 96];
const IMAGE_TYPES: [&str; 6] = ["png", "jpg", "jpeg", "gif", "bmp", "webp"];
const THUMBNAIL_TYPES: [&str; 12] =
    ["heic", "heif", "avif", "tif", "tiff", "mp4", "mov", "mkv", "avi", "wmv", "webm", "pdf"];
const TEXT_TYPES: [&str; 33] = [
    "txt", "md", "log", "json", "toml", "yaml", "yml", "xml", "csv", "tsv", "ini", "cfg", "conf", "rs", "py", "js",
    "ts", "tsx", "jsx", "c", "h", "cpp", "hpp", "cs", "java", "go", "html", "css", "ps1", "bat", "cmd", "sh", "sql",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Image,
    Text,
    Other,
}

pub(super) fn kind(hit: &Hit) -> Kind {
    let extension = hit.name().rsplit_once('.').map(|(_, extension)| extension.to_lowercase()).unwrap_or_default();
    match (hit.dir, extension.as_str()) {
        (true, _) => Kind::Other,
        (false, extension) if IMAGE_TYPES.contains(&extension) || THUMBNAIL_TYPES.contains(&extension) => Kind::Image,
        (false, extension) if TEXT_TYPES.contains(&extension) => Kind::Text,
        (false, _) => Kind::Other,
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Facts {
    pub(super) size: Option<u64>,
    pub(super) created: Option<Stamp>,
    pub(super) modified: Option<Stamp>,
}

#[derive(Clone)]
pub(super) enum Visual {
    Picture(Arc<RenderImage>),
    Text(SharedString),
    Icon(Option<Arc<RenderImage>>),
}

#[derive(Clone)]
pub(super) struct Loaded {
    pub(super) visual: Visual,
    pub(super) facts: Facts,
}

pub(super) enum Early {
    Done(Loaded),
    Glimpse(Loaded),
    Nothing,
}

pub(super) fn early(hit: &Hit, ratio: f32) -> Early {
    if kind(hit) != Kind::Image {
        return Early::Nothing;
    }
    let side = picture_side(ratio);
    let sides = [side].into_iter().chain(CACHED_SIDES.into_iter().filter(|&cached| cached < side)).collect::<Vec<_>>();
    let Some((found, thumbnail)) = platform::cached_thumbnail(&hit.path, &sides) else { return Early::Nothing };
    let Some(picture) = icons::picture(thumbnail.width, thumbnail.height, thumbnail.bgra) else {
        return Early::Nothing;
    };
    let loaded = Loaded { visual: Visual::Picture(picture), facts: facts(&hit.path) };
    if found == side { Early::Done(loaded) } else { Early::Glimpse(loaded) }
}

pub(super) fn load(hit: &Hit, ratio: f32, replace_small: bool) -> Loaded {
    let facts = facts(&hit.path);
    let side = picture_side(ratio);
    let visual = match kind(hit) {
        Kind::Image => thumbnail(&hit.path, side).or_else(|| picture(&hit.path, side)).map(Visual::Picture),
        Kind::Text => read_text(&hit.path).map(|text| Visual::Text(text.into())),
        Kind::Other => None,
    };
    let visual = visual.unwrap_or_else(|| {
        let key = icon_key(hit, (ICON * ratio).round() as u32);
        Visual::Icon(icons::load_all(vec![key]).into_iter().next().and_then(|(_, icon)| icon?.shown(replace_small)))
    });
    Loaded { visual, facts }
}

fn picture_side(ratio: f32) -> u32 {
    (PICTURE * ratio).round() as u32
}

fn facts(path: &str) -> Facts {
    std::fs::metadata(path).map_or(Facts::default(), |metadata| Facts {
        size: metadata.is_file().then_some(metadata.len()),
        created: metadata.created().ok().and_then(platform::local_time),
        modified: metadata.modified().ok().and_then(platform::local_time),
    })
}

fn thumbnail(path: &str, side: u32) -> Option<Arc<RenderImage>> {
    let thumbnail = platform::thumbnail(path, side)?;
    icons::picture(thumbnail.width, thumbnail.height, thumbnail.bgra)
}

pub(super) fn picture(path: &str, side: u32) -> Option<Arc<RenderImage>> {
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = if image.width() > side || image.height() > side { image.thumbnail(side, side) } else { image };
    let mut pixels = image.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

pub(super) fn read_text(path: &str) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(TEXT_BYTES).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}').trim_end_matches(char::REPLACEMENT_CHARACTER);
    Some(text.lines().take(TEXT_LINES).collect::<Vec<_>>().join("\n").replace('\t', TAB))
}
