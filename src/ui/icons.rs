mod disk;

use crate::platform::{self, IconSource};
use crate::settings;
use disk::Saved;
use futures::future::join_all;
use gpui::{App, Global, RenderImage, prelude::*};
use image::imageops::{self, FilterType};
use image::{Rgba, Rgba32FImage, RgbaImage};
use rustc_hash::FxHashMap;
use std::cmp::Reverse;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CACHE_FILE: &str = "icons.bin";
const SAVED_ICONS: usize = 1024;
const SAVE_DELAY: Duration = Duration::from_secs(2);
const WARM_BATCH: usize = 16;
const SHOWN_BATCH: usize = 32;
const LOAD_WORKERS: usize = 4;
const VISIBLE_ALPHA: u8 = 32;
const GRAY_SPREAD: u8 = 24;
const LEAST_CONTRAST: u32 = 90;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct IconKey {
    pub(crate) source: IconSource,
    pub(crate) pixels: u32,
}

#[derive(Clone)]
pub(crate) struct IconImage {
    image: Arc<RenderImage>,
    small: bool,
}

impl IconImage {
    pub(crate) fn shown(&self, replace_small: bool) -> Option<Arc<RenderImage>> {
        (!(self.small && replace_small)).then(|| self.image.clone())
    }
}

enum Slot {
    Loading,
    Ready { icon: IconImage, used: u64, checked: bool },
    Missing,
}

#[derive(Default)]
pub(crate) struct Icons {
    slots: FxHashMap<IconKey, Slot>,
    saving: bool,
}

impl Global for Icons {}

impl Icons {
    pub(crate) fn get(&self, key: &IconKey, replace_small: bool) -> Option<Arc<RenderImage>> {
        match self.slots.get(key) {
            Some(Slot::Ready { icon, .. }) => icon.shown(replace_small),
            Some(Slot::Loading | Slot::Missing) | None => None,
        }
    }

    fn claim(&mut self, keys: Vec<IconKey>, now: u64) -> Vec<IconKey> {
        let mut wanted = Vec::new();
        let mut rechecked = Vec::new();
        for key in keys {
            match self.slots.get_mut(&key) {
                None => {
                    self.slots.insert(key.clone(), Slot::Loading);
                    wanted.push(key);
                }
                Some(Slot::Ready { used, checked, .. }) => {
                    *used = now;
                    if !*checked {
                        *checked = true;
                        rechecked.push(key);
                    }
                }
                Some(Slot::Loading | Slot::Missing) => {}
            }
        }
        wanted.extend(rechecked);
        wanted
    }

    fn claim_unknown(&mut self, keys: Vec<IconKey>) -> Vec<IconKey> {
        let mut wanted = Vec::new();
        for key in keys {
            if !self.slots.contains_key(&key) {
                self.slots.insert(key.clone(), Slot::Loading);
                wanted.push(key);
            }
        }
        wanted
    }

    fn restore(&mut self, saved: Vec<Saved>) {
        for (key, used, icon) in saved {
            self.slots.entry(key).or_insert(Slot::Ready { icon, used, checked: false });
        }
    }

    fn apply(&mut self, loaded: Vec<(IconKey, Option<IconImage>)>, now: u64) -> bool {
        let mut changed = false;
        for (key, icon) in loaded {
            let shown = match self.slots.get(&key) {
                Some(Slot::Ready { icon, used, .. }) => Some((icon, *used)),
                Some(Slot::Loading | Slot::Missing) | None => None,
            };
            let slot = match (icon, shown) {
                (Some(icon), Some((old, _)))
                    if icon.small == old.small && icon.image.as_bytes(0) == old.image.as_bytes(0) =>
                {
                    continue;
                }
                (Some(icon), shown) => Slot::Ready { icon, used: shown.map_or(now, |(_, used)| used), checked: true },
                (None, Some(_)) => continue,
                (None, None) => Slot::Missing,
            };
            changed |= matches!(slot, Slot::Ready { .. });
            self.slots.insert(key, slot);
        }
        changed
    }

    fn saved(&self) -> Vec<Saved> {
        let mut saved: Vec<Saved> = self
            .slots
            .iter()
            .filter_map(|(key, slot)| match slot {
                Slot::Ready { icon, used, .. } => Some((key.clone(), *used, icon.clone())),
                Slot::Loading | Slot::Missing => None,
            })
            .collect();
        saved.sort_by_key(|(_, used, _)| Reverse(*used));
        saved.truncate(SAVED_ICONS);
        saved
    }

    fn store(&mut self, loaded: Vec<(IconKey, Option<IconImage>)>, cx: &mut App) {
        if !self.apply(loaded, now()) || self.saving {
            return;
        }
        self.saving = true;
        cx.spawn(async move |cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let saved = cx.update_global::<Self, _>(|icons, _| {
                icons.saving = false;
                icons.saved()
            });
            cx.background_spawn(async move {
                if let Err(error) = disk::write(&cache_file(), &saved) {
                    log::warn!("Couldn't save the icon cache: {error:#}");
                }
            })
            .await;
        })
        .detach();
    }
}

pub(crate) fn start(mut paths: Vec<(String, bool)>, pixels: u32, cx: &mut App) {
    cx.set_global(Icons::default());
    cx.spawn(async move |cx| {
        let (saved, warm) = cx
            .background_spawn(async move {
                let started = Instant::now();
                let saved = disk::read(&cache_file());
                log::info!("{} icons restored in {:?}", saved.len(), started.elapsed());
                paths.extend(platform::app_shortcuts().into_iter().map(|path| (path, false)));
                paths.extend(platform::store_apps().into_iter().map(|app| (app.path, false)));
                let warm: Vec<IconKey> = paths
                    .iter()
                    .map(|(path, dir)| IconKey { source: IconSource::for_path(path, *dir), pixels })
                    .collect();
                (saved, warm)
            })
            .await;
        let wanted = cx.update_global::<Icons, _>(|icons, _| {
            icons.restore(saved);
            icons.claim_unknown(warm)
        });
        for batch in wanted.chunks(WARM_BATCH) {
            let batch = batch.to_vec();
            let loaded = cx.background_spawn(async move { load_all(batch) }).await;
            cx.update_global::<Icons, _>(|icons, cx| icons.store(loaded, cx));
        }
    })
    .detach();
}

pub(crate) fn request(keys: Vec<IconKey>, cx: &mut App) {
    let wanted = cx.update_global::<Icons, _>(|icons, _| icons.claim(keys, now()));
    if wanted.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        for batch in wanted.chunks(SHOWN_BATCH) {
            let parts = batch.chunks(batch.len().div_ceil(LOAD_WORKERS)).map(|part| {
                let part = part.to_vec();
                cx.background_spawn(async move { load_all(part) })
            });
            let loaded = join_all(parts.collect::<Vec<_>>()).await.into_iter().flatten().collect();
            cx.update_global::<Icons, _>(|icons, cx| icons.store(loaded, cx));
        }
    })
    .detach();
}

fn cache_file() -> PathBuf {
    settings::cache_dir().join(CACHE_FILE)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

pub(crate) fn load_all(keys: Vec<IconKey>) -> Vec<(IconKey, Option<IconImage>)> {
    keys.into_iter()
        .map(|key| {
            let image = load(&key);
            (key, image)
        })
        .collect()
}

fn load(key: &IconKey) -> Option<IconImage> {
    let icon = platform::load_icon(&key.source, key.pixels)?;
    let small = icon.small;
    let buffer = RgbaImage::from_raw(icon.width, icon.height, icon.bgra)?;
    Some(IconImage { image: rendered(shrink(buffer, key.pixels)), small })
}

pub(crate) fn picture(width: u32, height: u32, bgra: Vec<u8>) -> Option<Arc<RenderImage>> {
    RgbaImage::from_raw(width, height, bgra).map(rendered)
}

pub(crate) fn readable(bgra: &mut [u8], background: f32) {
    let pixels = bgra.as_chunks_mut::<4>().0;
    let (mut total, mut count) = (0, 0);
    for &[b, g, r, _] in pixels.iter().filter(|pixel| pixel[3] >= VISIBLE_ALPHA) {
        let (most, least) = (b.max(g).max(r), b.min(g).min(r));
        if most - least > GRAY_SPREAD {
            return;
        }
        (total, count) = (total + u32::from(most), count + 1);
    }
    let behind = (background.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u32;
    if count > 0 && (total / count).abs_diff(behind) < LEAST_CONTRAST {
        for pixel in pixels {
            for channel in &mut pixel[..3] {
                *channel = u8::MAX - *channel;
            }
        }
    }
}

fn rendered(buffer: RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn shrink(buffer: RgbaImage, pixels: u32) -> RgbaImage {
    if buffer.width() <= pixels && buffer.height() <= pixels {
        return buffer;
    }
    let premultiplied =
        Rgba32FImage::from_fn(buffer.width(), buffer.height(), |x, y| premultiply(buffer.get_pixel(x, y)));
    let small = imageops::resize(&premultiplied, pixels, pixels, FilterType::Lanczos3);
    RgbaImage::from_fn(pixels, pixels, |x, y| unpremultiply(small.get_pixel(x, y)))
}

fn premultiply(pixel: &Rgba<u8>) -> Rgba<f32> {
    let [b, g, r, a] = pixel.0.map(|channel| f32::from(channel) / f32::from(u8::MAX));
    Rgba([b * a, g * a, r * a, a])
}

fn unpremultiply(pixel: &Rgba<f32>) -> Rgba<u8> {
    let [b, g, r, a] = pixel.0;
    let alpha = a.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Rgba([0; 4]);
    }
    let byte = |value: f32| (value.clamp(0.0, 1.0) * f32::from(u8::MAX)).round() as u8;
    Rgba([byte(b / alpha), byte(g / alpha), byte(r / alpha), byte(alpha)])
}

#[cfg(test)]
mod tests {
    use super::{IconImage, IconKey, Icons, SAVED_ICONS, readable, shrink};
    use crate::platform::IconSource;
    use gpui::RenderImage;
    use image::{Frame, Rgba, RgbaImage};

    #[test]
    fn a_gray_glyph_flips_only_where_it_would_vanish() {
        let glyph = || vec![40, 40, 40, 255, 0, 0, 0, 0];
        let mut on_dark = glyph();
        readable(&mut on_dark, 0.0);
        assert_eq!(on_dark, [215, 215, 215, 255, 255, 255, 255, 0]);
        let mut on_light = glyph();
        readable(&mut on_light, 1.0);
        assert_eq!(on_light, glyph());
    }

    #[test]
    fn colored_icons_keep_their_colors() {
        let colored = vec![200, 60, 20, 255, 30, 30, 30, 255];
        let mut shown = colored.clone();
        readable(&mut shown, 0.0);
        assert_eq!(shown, colored);
    }
    use std::sync::Arc;

    fn key(extension: &str) -> IconKey {
        IconKey { source: IconSource::Extension(extension.into()), pixels: 2 }
    }

    fn icon(shade: u8, small: bool) -> IconImage {
        let image = RgbaImage::from_pixel(2, 2, Rgba([shade, 0, 0, 255]));
        IconImage { image: Arc::new(RenderImage::new(vec![Frame::new(image)])), small }
    }

    fn image(shade: u8) -> IconImage {
        icon(shade, false)
    }

    fn shown(icons: &Icons, extension: &str, replace_small: bool) -> Option<u8> {
        let image = icons.get(&key(extension), replace_small)?;
        image.as_bytes(0).and_then(|bytes| bytes.first().copied())
    }

    fn shade(icons: &Icons, extension: &str) -> Option<u8> {
        shown(icons, extension, true)
    }

    fn restored(extensions: &[&str]) -> Icons {
        let mut icons = Icons::default();
        icons.restore(extensions.iter().map(|extension| (key(extension), 1, image(1))).collect());
        icons
    }

    #[test]
    fn a_restored_icon_shows_at_once_and_is_checked_once_after_missing_ones() {
        let mut icons = restored(&["py"]);
        assert_eq!(shade(&icons, "py"), Some(1));
        assert_eq!(icons.claim(vec![key("py"), key("txt")], 5), vec![key("txt"), key("py")]);
        assert!(icons.claim(vec![key("py"), key("txt")], 6).is_empty());
    }

    #[test]
    fn warming_skips_restored_icons() {
        let mut icons = restored(&["py"]);
        assert_eq!(icons.claim_unknown(vec![key("py"), key("txt"), key("txt")]), vec![key("txt")]);
        assert_eq!(icons.claim(vec![key("py")], 5), vec![key("py")]);
    }

    #[test]
    fn a_check_replaces_only_changed_pixels_and_keeps_the_icon_on_failure() {
        let mut icons = restored(&["py", "txt", "md"]);
        icons.claim(vec![key("py"), key("txt"), key("md")], 5);
        assert!(!icons.apply(vec![(key("py"), Some(image(1))), (key("txt"), None)], 6));
        assert_eq!((shade(&icons, "py"), shade(&icons, "txt")), (Some(1), Some(1)));
        assert!(icons.apply(vec![(key("md"), Some(image(9)))], 6));
        assert_eq!(shade(&icons, "md"), Some(9));
    }

    #[test]
    fn a_small_icon_gives_way_to_the_placeholder_only_while_replacing() {
        let mut icons = Icons::default();
        icons.claim(vec![key("exe"), key("lnk")], 5);
        icons.apply(vec![(key("exe"), Some(icon(7, true))), (key("lnk"), Some(icon(8, false)))], 6);
        assert_eq!((shown(&icons, "exe", true), shown(&icons, "lnk", true)), (None, Some(8)));
        assert_eq!(shown(&icons, "exe", false), Some(7));
    }

    #[test]
    fn a_missing_icon_is_not_retried() {
        let mut icons = Icons::default();
        icons.claim(vec![key("xyz")], 5);
        assert!(!icons.apply(vec![(key("xyz"), None)], 6));
        assert_eq!(shade(&icons, "xyz"), None);
        assert!(icons.claim(vec![key("xyz")], 7).is_empty());
    }

    #[test]
    fn the_saved_cache_keeps_the_most_recently_used_icons() {
        let mut icons = Icons::default();
        let extensions: Vec<String> = (0..=SAVED_ICONS).map(|number| format!("e{number}")).collect();
        for (used, extension) in extensions.iter().enumerate() {
            icons.claim(vec![key(extension)], used as u64);
            icons.apply(vec![(key(extension), Some(image(1)))], used as u64);
        }
        icons.claim(vec![key("e0")], 10_000);
        let saved = icons.saved();
        assert_eq!(saved.len(), SAVED_ICONS);
        assert_eq!(saved.first().map(|(key, used, _)| (key.clone(), *used)), Some((key("e0"), 10_000)));
        assert_eq!(saved.last().map(|(key, _, _)| key.clone()), Some(key("e2")));
    }

    #[test]
    fn shrinking_keeps_soft_edges_free_of_transparent_color() {
        let side = 256;
        let icon = RgbaImage::from_fn(side, side, |x, _| {
            if x < side / 2 { Rgba([128, 128, 128, 255]) } else { Rgba([255, 255, 255, 0]) }
        });
        let small = shrink(icon, 40);
        let edge = small.get_pixel(20, 20).0;
        assert!(edge[3] > 0 && edge[3] < 255, "edge {edge:?} is not soft");
        assert!(edge[..3].iter().all(|channel| (126..=130).contains(channel)), "edge {edge:?} took a fringe");
    }

    #[test]
    fn small_icons_keep_their_pixels() {
        let icon = RgbaImage::from_pixel(16, 16, Rgba([1, 2, 3, 4]));
        let kept = shrink(icon.clone(), 20);
        assert_eq!(kept, icon);
    }
}
