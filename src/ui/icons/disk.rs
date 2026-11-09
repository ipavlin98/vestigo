use super::{IconImage, IconKey};
use crate::platform::IconSource;
use anyhow::{Context, Result};
use gpui::RenderImage;
use image::{Frame, RgbaImage};
use std::path::Path;
use std::sync::Arc;

pub(super) type Saved = (IconKey, u64, IconImage);

const MAGIC: &[u8; 8] = b"VICONS02";
const FOLDER: u8 = 0;
const EXTENSION: u8 = 1;
const FILE: u8 = 2;
const MAX_SIDE: u32 = 1024;
const BYTES_PER_PIXEL: usize = 4;

pub(super) fn write(path: &Path, saved: &[Saved]) -> Result<()> {
    let mut out = MAGIC.to_vec();
    for (key, used, icon) in saved {
        let Some(bytes) = icon.image.as_bytes(0) else { continue };
        let size = icon.image.size(0);
        let (kind, text) = match &key.source {
            IconSource::Folder => (FOLDER, ""),
            IconSource::Extension(extension) => (EXTENSION, extension.as_str()),
            IconSource::File(path) => (FILE, path.as_str()),
        };
        out.push(kind);
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(&key.pixels.to_le_bytes());
        out.extend_from_slice(&used.to_le_bytes());
        out.push(u8::from(icon.small));
        out.extend_from_slice(&(size.width.0 as u32).to_le_bytes());
        out.extend_from_slice(&(size.height.0 as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    let folder = path.parent().context("the icon cache has no folder")?;
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, out).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

pub(super) fn read(path: &Path) -> Vec<Saved> {
    let Ok(data) = std::fs::read(path) else { return Vec::new() };
    let mut reader = Reader { data: &data };
    if reader.take(MAGIC.len()) != Some(MAGIC) {
        log::warn!("{} has an unknown format, starting over", path.display());
        return Vec::new();
    }
    let mut saved = Vec::new();
    while !reader.data.is_empty() {
        let Some(entry) = reader.entry() else {
            log::warn!("{} is damaged after {} icons", path.display(), saved.len());
            break;
        };
        saved.push(entry);
    }
    saved
}

struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.data.split_at_checked(count)?;
        self.data = rest;
        Some(taken)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn entry(&mut self) -> Option<Saved> {
        let [kind] = self.array()?;
        let length = usize::try_from(self.u32()?).ok()?;
        let text = std::str::from_utf8(self.take(length)?).ok()?.to_owned();
        let source = match kind {
            FOLDER => IconSource::Folder,
            EXTENSION => IconSource::Extension(text),
            FILE => IconSource::File(text),
            _ => return None,
        };
        let pixels = self.u32()?;
        let used = u64::from_le_bytes(self.array()?);
        let [small] = self.array()?;
        let (width, height) = (self.u32()?, self.u32()?);
        if width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let bytes = self.take(width as usize * height as usize * BYTES_PER_PIXEL)?.to_vec();
        let buffer = RgbaImage::from_raw(width, height, bytes)?;
        let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]));
        Some((IconKey { source, pixels }, used, IconImage { image, small: small != 0 }))
    }
}

#[cfg(test)]
mod tests {
    use super::{Saved, read, write};
    use crate::platform::IconSource;
    use crate::ui::icons::{IconImage, IconKey};
    use gpui::RenderImage;
    use image::{Frame, Rgba, RgbaImage};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn icon(source: IconSource, used: u64, shade: u8) -> Saved {
        let buffer = RgbaImage::from_pixel(2, 2, Rgba([shade, 2, 3, 255]));
        let image = Arc::new(RenderImage::new(vec![Frame::new(buffer)]));
        (IconKey { source, pixels: 2 }, used, IconImage { image, small: false })
    }

    fn small((key, used, icon): Saved) -> Saved {
        (key, used, IconImage { small: true, ..icon })
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("vestigo-icons-{name}-{}.bin", std::process::id()))
    }

    fn summary(saved: &[Saved]) -> Vec<(IconKey, u64, bool, Vec<u8>)> {
        saved
            .iter()
            .map(|(key, used, icon)| (key.clone(), *used, icon.small, icon.image.as_bytes(0).unwrap().to_vec()))
            .collect()
    }

    #[test]
    fn saved_icons_come_back_in_order() {
        let path = temp_file("round-trip");
        let saved = vec![
            icon(IconSource::File("C:\\Программы\\Редактор.lnk".into()), 30, 10),
            small(icon(IconSource::Extension("py".into()), 20, 20)),
            icon(IconSource::Folder, 10, 30),
        ];
        write(&path, &saved).unwrap();
        let restored = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(summary(&restored), summary(&saved));
    }

    #[test]
    fn a_truncated_cache_keeps_the_whole_icons_before_the_cut() {
        let path = temp_file("truncated");
        write(&path, &[icon(IconSource::Folder, 2, 1), icon(IconSource::Extension("txt".into()), 1, 2)]).unwrap();
        let data = std::fs::read(&path).unwrap();
        std::fs::write(&path, &data[..data.len() - 3]).unwrap();
        let restored = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(summary(&restored), summary(&[icon(IconSource::Folder, 2, 1)]));
    }

    #[test]
    fn a_foreign_or_missing_file_restores_nothing() {
        let path = temp_file("foreign");
        std::fs::write(&path, b"VESTIGO3 not icons at all").unwrap();
        let restored = read(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(restored.is_empty());
        assert!(read(&temp_file("missing")).is_empty());
    }
}
