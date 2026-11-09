use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

pub(crate) const FOLDER: &str = "icons/folder.svg";
pub(crate) const FILE: &str = "icons/file.svg";
pub(crate) const ENTER: &str = "icons/enter.svg";
pub(crate) const CLOSE: &str = "icons/close.svg";
pub(crate) const MINIMIZE: &str = "icons/minimize.svg";
pub(crate) const MAXIMIZE: &str = "icons/maximize.svg";
pub(crate) const RESTORE: &str = "icons/restore.svg";
pub(crate) const ARROW_UP: &str = "icons/arrow-up.svg";
pub(crate) const ARROW_DOWN: &str = "icons/arrow-down.svg";
pub(crate) const CHEVRON_DOWN: &str = "icons/chevron-down.svg";
pub(crate) const CHEVRON_UP: &str = "icons/chevron-up.svg";
pub(crate) const CHEVRON_RIGHT: &str = "icons/chevron-right.svg";
pub(crate) const CHECK: &str = "icons/check.svg";
pub(crate) const GLOBE: &str = "icons/globe.svg";
pub(crate) const COMMAND: &str = "icons/command.svg";
pub(crate) const ACTION: &str = "icons/action.svg";
pub(crate) const SETTINGS: &str = "icons/settings.svg";
pub(crate) const PREVIEW: &str = "icons/preview.svg";
pub(crate) const OPEN: &str = "icons/open.svg";
pub(crate) const OPEN_WITH: &str = "icons/open-with.svg";
pub(crate) const REVEAL: &str = "icons/reveal.svg";
pub(crate) const ADMIN: &str = "icons/admin.svg";
pub(crate) const COPY: &str = "icons/copy.svg";
pub(crate) const CUT: &str = "icons/cut.svg";
pub(crate) const COPY_PATH: &str = "icons/copy-path.svg";
pub(crate) const COPY_NAME: &str = "icons/copy-name.svg";
pub(crate) const DELETE: &str = "icons/delete.svg";
pub(crate) const PROPERTIES: &str = "icons/properties.svg";
pub(crate) const CONTEXT_MENU: &str = "icons/context-menu.svg";
pub(crate) const MENU_ITEM: &str = "icons/menu-item.svg";
pub(crate) const RESET: &str = "icons/reset.svg";
pub(crate) const PIPETTE: &str = "icons/pipette.svg";
pub(crate) const SEARCH: &str = "icons/search.svg";
pub(crate) const GRIP: &str = "icons/grip.svg";
pub(crate) const STAR: &str = "icons/star.svg";
pub(crate) const STAR_FILLED: &str = "icons/star-filled.svg";
pub(crate) const HISTORY: &str = "icons/history.svg";
pub(crate) const EYE: &str = "icons/eye.svg";

const HEADER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" shape-rendering="crispEdges">"#;

const ICONS: [(&str, &str); 38] = [
    (SEARCH, r#"<circle cx="10" cy="10" r="8.2"/><path d="m16 16 6.2 6.2"/>"#),
    (
        GRIP,
        r#"<g fill="black" stroke="none"><circle cx="9" cy="6" r="1.6"/><circle cx="15" cy="6" r="1.6"/><circle cx="9" cy="12" r="1.6"/><circle cx="15" cy="12" r="1.6"/><circle cx="9" cy="18" r="1.6"/><circle cx="15" cy="18" r="1.6"/></g>"#,
    ),
    (
        FOLDER,
        r#"<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4.6l2 2.2h8.4A1.5 1.5 0 0 1 21 8.7v9.8a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5z"/>"#,
    ),
    (
        FILE,
        r#"<path d="M6.5 3h7.6L19 7.9v11.6a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 19.5v-15A1.5 1.5 0 0 1 6.5 3z"/><path d="M14 3v5h5"/>"#,
    ),
    (ENTER, r#"<path d="M19 5v6a3 3 0 0 1-3 3H6"/><path d="m10 9-4 5 4 5"/>"#),
    (CLOSE, r#"<path d="M6 6l12 12M18 6 6 18"/>"#),
    (MINIMIZE, r#"<path d="M5 12h14"/>"#),
    (MAXIMIZE, r#"<rect x="5.5" y="5.5" width="13" height="13" rx="1"/>"#),
    (
        RESTORE,
        r#"<rect x="5" y="8.5" width="10.5" height="10.5" rx="1"/><path d="M8.5 8.5V6a1 1 0 0 1 1-1H18a1 1 0 0 1 1 1v8.5a1 1 0 0 1-1 1h-2.5"/>"#,
    ),
    (ARROW_UP, r#"<path d="M12 19V5M6 11l6-6 6 6"/>"#),
    (ARROW_DOWN, r#"<path d="M12 5v14M6 13l6 6 6-6"/>"#),
    (CHEVRON_DOWN, r#"<path d="m6 9 6 6 6-6"/>"#),
    (CHEVRON_UP, r#"<path d="m6 15 6-6 6 6"/>"#),
    (CHEVRON_RIGHT, r#"<path d="m9 6 6 6-6 6"/>"#),
    (CHECK, r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"#),
    (
        GLOBE,
        r#"<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17"/><path d="M12 3.5c2.4 2.3 3.6 5.1 3.6 8.5s-1.2 6.2-3.6 8.5c-2.4-2.3-3.6-5.1-3.6-8.5s1.2-6.2 3.6-8.5z"/>"#,
    ),
    (COMMAND, r#"<rect x="3" y="4.5" width="18" height="15" rx="1.5"/><path d="m7 9.5 3 2.5-3 2.5M12.5 15H17"/>"#),
    (ACTION, r#"<path d="M13 3 5 13.5h6L10 21l8-10.5h-6z"/>"#),
    (
        SETTINGS,
        r#"<path d="M12.2244 1.8h-.4488a2.04 2.04 0 0 0-2.04 2.2236a2.04 2.04 0 0 1-1.02 1.7646l-.4386 .255a2.04 2.04 0 0 1-2.04 0l-.153-.0816a2.04 2.04 0 0 0-2.7846 .7446l-.2244 .3876a2.04 2.04 0 0 0 .7446 2.7846l.153 .102a2.04 2.04 0 0 1 1.02 1.7544v.5202a2.04 2.04 0 0 1-1.02 1.7748l-.153 .0918a2.04 2.04 0 0 0-.7446 2.7846l.2244 .3876a2.04 2.04 0 0 0 2.7846 .7446l.153-.0816a2.04 2.04 0 0 1 2.04 0l.4386 .255a2.04 2.04 0 0 1 1.02 1.7646a2.04 2.04 0 0 0 2.04 2.2236h.4488a2.04 2.04 0 0 0 2.04-2.2236a2.04 2.04 0 0 1 1.02-1.7646l.4386-.255a2.04 2.04 0 0 1 2.04 0l.153 .0816a2.04 2.04 0 0 0 2.7846-.7446l.2244-.3978a2.04 2.04 0 0 0-.7446-2.7846l-.153-.0816a2.04 2.04 0 0 1-1.02-1.7748v-.51a2.04 2.04 0 0 1 1.02-1.7748l.153-.0918a2.04 2.04 0 0 0 .7446-2.7846l-.2244-.3876a2.04 2.04 0 0 0-2.7846-.7446l-.153 .0816a2.04 2.04 0 0 1-2.04 0l-.4386-.255a2.04 2.04 0 0 1-1.02-1.7646a2.04 2.04 0 0 0-2.04-2.2236z"/><circle cx="12" cy="12" r="3.06"/>"#,
    ),
    (PREVIEW, r#"<rect x="3.5" y="4.5" width="17" height="15" rx="1.5"/><path d="M14 4.5v15"/>"#),
    (
        OPEN,
        r#"<path d="M13.5 4.5h6v6"/><path d="m19.5 4.5-8 8"/><path d="M18 14v4.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 4 18.5v-11A1.5 1.5 0 0 1 5.5 6H10"/>"#,
    ),
    (
        OPEN_WITH,
        r#"<rect x="4" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.5"/>"#,
    ),
    (
        REVEAL,
        r#"<path d="M3.5 18V6.5A1.5 1.5 0 0 1 5 5h4.1l2 2.2H17.5A1.5 1.5 0 0 1 19 8.7V10"/><path d="M3.5 18l2.3-6.4A1.5 1.5 0 0 1 7.2 10.6h13.1a1 1 0 0 1 .95 1.3l-2 6.1a1.5 1.5 0 0 1-1.42 1H5a1.5 1.5 0 0 1-1.5-1.5"/>"#,
    ),
    (ADMIN, r#"<path d="M12 3.5 5 6.2v5.3c0 4.2 2.9 7.6 7 9 4.1-1.4 7-4.8 7-9V6.2z"/><path d="m9.2 12 2 2 3.8-4"/>"#),
    (
        COPY,
        r#"<rect x="8.5" y="8.5" width="11.5" height="11.5" rx="1.5"/><path d="M15.5 8.5v-3A1.5 1.5 0 0 0 14 4H5.5A1.5 1.5 0 0 0 4 5.5V14a1.5 1.5 0 0 0 1.5 1.5h3"/>"#,
    ),
    (
        CUT,
        r#"<circle cx="6.5" cy="6.5" r="2.5"/><circle cx="6.5" cy="17.5" r="2.5"/><path d="M8.6 8 20 19"/><path d="M8.6 16 20 5"/>"#,
    ),
    (
        COPY_PATH,
        r#"<path d="M10.5 13.5a4 4 0 0 0 6 .4l2.6-2.6a4 4 0 0 0-5.7-5.7l-1.4 1.4"/><path d="M13.5 10.5a4 4 0 0 0-6-.4l-2.6 2.6a4 4 0 0 0 5.7 5.7l1.4-1.4"/>"#,
    ),
    (COPY_NAME, r#"<path d="M5 7.5V5h14v2.5"/><path d="M12 5v14"/><path d="M9 19h6"/>"#),
    (
        DELETE,
        r#"<path d="M4 6.5h16"/><path d="M9 6.5V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v1.5"/><path d="M6 6.5l.9 12.1A1.5 1.5 0 0 0 8.4 20h7.2a1.5 1.5 0 0 0 1.5-1.4L18 6.5"/><path d="M10 10.5v6M14 10.5v6"/>"#,
    ),
    (PROPERTIES, r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5"/><path d="M12 7.6v.1"/>"#),
    (CONTEXT_MENU, r#"<rect x="4" y="4" width="16" height="16" rx="1.5"/><path d="M8 9h8M8 12h8M8 15h5"/>"#),
    (
        MENU_ITEM,
        r#"<path d="M9.2 9.2 19.5 13.4l-4.4 1.7-1.7 4.4z"/><path d="M15.1 15.1 19 19"/><path d="M5.5 4v3M4 5.5h3"/><path d="M10.5 4.5 9.5 6.5M4.5 10.5l2-1"/>"#,
    ),
    (
        PIPETTE,
        r#"<path d="m12.5 7.5 4 4"/><path d="M14.6 5.4a2.1 2.1 0 0 1 3 0l1 1a2.1 2.1 0 0 1 0 3L16.5 11.5l-4-4z"/><path d="M12.5 7.5 5.6 14.4a2 2 0 0 0-.6 1.4V19h3.2a2 2 0 0 0 1.4-.6l6.9-6.9"/>"#,
    ),
    (RESET, r#"<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3L4.5 9"/><path d="M4.5 4.5V9H9"/>"#),
    (STAR, r#"<path d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z"/>"#),
    (
        STAR_FILLED,
        r#"<path fill="black" d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z"/>"#,
    ),
    (HISTORY, r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>"#),
    (
        EYE,
        r#"<path d="M2.5 12C4.6 8 8 5.5 12 5.5s7.4 2.5 9.5 6.5c-2.1 4-5.5 6.5-9.5 6.5S4.6 16 2.5 12z"/><circle cx="12" cy="12" r="3"/>"#,
    ),
];

pub(crate) struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, shape)| Cow::Owned(format!("{HEADER}{shape}</svg>").into_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(name, _)| name.starts_with(path)).map(|(name, _)| SharedString::from(*name)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{DevicePixels, size, svg_icon_mask};

    #[test]
    fn icon_lines_land_on_whole_device_pixels_at_any_scale() {
        let preview = Assets.load(PREVIEW).expect("load").expect("the preview icon");
        for device in [16, 20, 24, 28, 32] {
            let side = device * 2;
            let texels = DevicePixels(side as i32);
            let (_, mask) = svg_icon_mask(&preview, size(texels, texels)).expect("rasterized");
            let pixel = |x: usize, y: usize| -> u32 {
                [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|(dx, dy)| u32::from(mask[(y * 2 + dy) * side + x * 2 + dx]))
                    .sum::<u32>()
                    / 4
            };
            let row: Vec<u32> = (0..device).map(|x| pixel(x, device / 2)).collect();
            let solid = row.iter().filter(|alpha| **alpha >= 250).count();
            assert!(solid >= 3, "at {device} px the three vertical lines of the preview icon blur: {row:?}");
        }
    }

    #[test]
    fn closed_curves_stay_centered_after_pixel_fitting() {
        let settings = Assets.load(SETTINGS).expect("load").expect("the settings icon");
        for device in [16, 20, 24, 28, 32] {
            let side = device * 2;
            let texels = DevicePixels(side as i32);
            let (_, mask) = svg_icon_mask(&settings, size(texels, texels)).expect("rasterized");
            let row = &mask[side * side / 2..side * (side / 2 + 1)];
            let column: Vec<u8> = mask.iter().skip(side / 2).step_by(side).copied().collect();
            for (line, direction) in [(row, "sideways"), (column.as_slice(), "up or down")] {
                let lean = line.iter().zip(line.iter().rev()).map(|(one, other)| one.abs_diff(*other)).max();
                assert!(lean <= Some(64), "at {device} px the gear leans {direction}: {line:?}");
            }
        }
    }

    fn stroke_centers(line: &[u8]) -> Vec<f32> {
        let mut strokes: Vec<(f32, f32)> = Vec::new();
        for (x, alpha) in line.iter().enumerate().filter(|(_, alpha)| **alpha > 0) {
            if x == 0 || line[x - 1] == 0 {
                strokes.push((0., 0.));
            }
            if let Some((sum, ink)) = strokes.last_mut() {
                *sum += x as f32 * f32::from(*alpha);
                *ink += f32::from(*alpha);
            }
        }
        strokes.iter().map(|(sum, ink)| sum / ink).collect()
    }

    #[test]
    fn stem_stays_centered_between_mirrored_lines_after_pixel_fitting() {
        let icon = Assets.load(COPY_NAME).expect("load").expect("the copy name icon");
        for device in [14, 15, 16, 17, 18, 20, 21, 22, 24, 25, 28, 32] {
            let side = device * 2;
            let texels = DevicePixels(side as i32);
            let (_, mask) = svg_icon_mask(&icon, size(texels, texels)).expect("rasterized");
            let y = side * 7 / 24;
            let row = &mask[y * side..(y + 1) * side];
            let [left, stem, right] = stroke_centers(row)[..] else {
                panic!("at {device} px expected two serifs and the stem: {row:?}")
            };
            assert!(
                ((stem - left) - (right - stem)).abs() <= 0.25,
                "at {device} px the stem is off the middle of its serifs: {left} {stem} {right}"
            );
        }
    }
}
