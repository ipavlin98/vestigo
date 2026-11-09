use super::super::tip::{Tip, with_tip};
use super::super::tokens::PATH_SLACK;
use crate::assets;
use crate::theme::{StatusBar, pixels, rem, snap};
use gpui::{
    Bounds, Div, ElementId, Hsla, Pixels, ShapedLine, SharedString, Stateful, TextAlign, TextRun, TransformationMatrix,
    Window, canvas, div, point, prelude::*, px, size,
};
use std::borrow::Cow;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

const ELLIPSIS: &str = "…";
const SEPARATOR: char = '\\';
const UNC_ROOT_SEPARATORS: usize = 3;

pub(crate) fn path_label(
    id: impl Into<ElementId>,
    path: SharedString,
    marks: Vec<Range<usize>>,
    tip: Tip,
    line: f32,
) -> Stateful<Div> {
    let clipped = Rc::new(Cell::new(None));
    let measured = clipped.clone();
    let text = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let color = window.text_style().color;
            let room = bounds.size.width - pixels(PATH_SLACK, window);
            let shown = elide(&path, &marks, |candidate| shape(candidate, color, window).width <= room);
            let line = shape(&shown, color, window);
            let elided = matches!(shown, Cow::Owned(_));
            measured.set(elided.then(|| Bounds::new(bounds.origin, size(line.width, bounds.size.height))));
            if let Err(error) = line.paint(bounds.origin, bounds.size.height, TextAlign::Left, None, window, cx) {
                log::warn!("path paint failed: {error}");
            }
        },
    )
    .size_full();
    with_tip(div().h(rem(line)).line_height(rem(line)).child(text).id(id), tip, clipped)
}

pub(crate) fn path_crumbs(
    id: impl Into<ElementId>,
    path: SharedString,
    tip: Tip,
    line: f32,
    colors: StatusBar,
) -> Stateful<Div> {
    let clipped = Rc::new(Cell::new(None));
    let measured = clipped.clone();
    let crumbs = canvas(
        |_, _, _| {},
        move |bounds, _, window, cx| {
            let separator = pixels(line, window);
            let room = bounds.size.width - pixels(PATH_SLACK, window);
            let shown = elide(&path, &[], |candidate| crumbs_width(candidate, separator, colors.text, window) <= room);
            let names = names(&shown);
            let last = names.len().saturating_sub(1);
            let lines: Vec<ShapedLine> = names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let color = match (index == last, *name == ELLIPSIS) {
                        (true, _) => colors.place,
                        (false, true) => colors.separator,
                        (false, false) => colors.text,
                    };
                    shape(name, color, window)
                })
                .collect();
            let scale = window.scale_factor();
            let mut x = bounds.origin.x;
            for (index, shaped) in lines.iter().enumerate() {
                if let Err(error) =
                    shaped.paint(point(x, bounds.origin.y), bounds.size.height, TextAlign::Left, None, window, cx)
                {
                    log::warn!("path paint failed: {error}");
                }
                x += shaped.width;
                if index == last {
                    break;
                }
                let top = bounds.origin.y + (bounds.size.height - separator) / 2.;
                let glyph = Bounds::new(point(snap(x, scale), snap(top, scale)), size(separator, separator));
                let unit = TransformationMatrix::unit();
                if let Err(error) =
                    window.paint_svg(glyph, assets::CHEVRON_RIGHT.into(), None, unit, colors.separator, cx)
                {
                    log::warn!("path separator paint failed: {error}");
                }
                x += separator;
            }
            let elided = matches!(shown, Cow::Owned(_));
            let width = x - bounds.origin.x;
            measured.set(elided.then(|| Bounds::new(bounds.origin, size(width, bounds.size.height))));
        },
    )
    .size_full();
    with_tip(div().h(rem(line)).line_height(rem(line)).child(crumbs).id(id), tip, clipped)
}

fn shape(text: &str, color: Hsla, window: &Window) -> ShapedLine {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let text = SharedString::from(text.to_owned());
    let run = TextRun { color, ..style.to_run(text.len()) };
    window.text_system().shape_line(text, font_size, &[run], None)
}

fn crumbs_width(shown: &str, separator: Pixels, color: Hsla, window: &Window) -> Pixels {
    let names = names(shown);
    let text: Pixels = names.iter().map(|name| shape(name, color, window).width).fold(px(0.), |sum, width| sum + width);
    text + separator * names.len().saturating_sub(1) as f32
}

fn names(shown: &str) -> Vec<&str> {
    parts(shown).into_iter().filter_map(|part| shown.get(part)).filter(|name| !name.is_empty()).collect()
}

fn elide<'a>(path: &'a str, marks: &[Range<usize>], mut fits: impl FnMut(&str) -> bool) -> Cow<'a, str> {
    if fits(path) {
        return Cow::Borrowed(path);
    }
    let parts = parts(path);
    let last = parts.len().saturating_sub(1);
    if last > 1 {
        let mut kept = vec![0, last];
        let mut shown = join(path, &parts, &kept);
        if fits(&shown) {
            let mut order: Vec<usize> = (1..last).rev().collect();
            order.sort_by_key(|&index| {
                !parts.get(index).is_some_and(|part| marks.iter().any(|mark| overlaps(part, mark)))
            });
            for index in order {
                kept.push(index);
                let wider = join(path, &parts, &kept);
                if !fits(&wider) {
                    break;
                }
                shown = wider;
            }
            return Cow::Owned(shown);
        }
    }
    let name = parts.get(last).and_then(|part| path.get(part.clone())).unwrap_or(path);
    if last > 0 {
        let shown = format!("{ELLIPSIS}{SEPARATOR}{name}");
        if fits(&shown) {
            return Cow::Owned(shown);
        }
    }
    Cow::Owned(middle(name, fits))
}

fn parts(path: &str) -> Vec<Range<usize>> {
    let root = if path.starts_with(r"\\") {
        path.match_indices(SEPARATOR).nth(UNC_ROOT_SEPARATORS).map_or(path.len(), |(at, _)| at)
    } else {
        path.find(SEPARATOR).unwrap_or(path.len())
    };
    let mut start = root;
    let rest = path.get(root..).unwrap_or_default().split(SEPARATOR).skip(1).map(|part| {
        start += SEPARATOR.len_utf8();
        let range = start..start + part.len();
        start = range.end;
        range
    });
    std::iter::once(0..root).chain(rest).collect()
}

fn overlaps(part: &Range<usize>, mark: &Range<usize>) -> bool {
    part.start < mark.end && mark.start < part.end
}

fn join(path: &str, parts: &[Range<usize>], kept: &[usize]) -> String {
    let mut shown = String::with_capacity(path.len());
    let mut previous = None;
    for (index, part) in parts.iter().enumerate().filter(|(index, _)| kept.contains(index)) {
        let gap = previous.map_or(index > 0, |previous| index > previous + 1);
        if previous.is_some() {
            shown.push(SEPARATOR);
        }
        if gap {
            shown.push_str(ELLIPSIS);
            shown.push(SEPARATOR);
        }
        shown.push_str(path.get(part.clone()).unwrap_or_default());
        previous = Some(index);
    }
    shown
}

fn middle(name: &str, mut fits: impl FnMut(&str) -> bool) -> String {
    let bounds: Vec<usize> = name.char_indices().map(|(at, _)| at).chain([name.len()]).collect();
    let count = bounds.len().saturating_sub(1);
    let cut = |keep: usize| {
        let head = bounds.get(keep.div_ceil(2)).copied().unwrap_or(0);
        let tail = bounds.get(count - keep / 2).copied().unwrap_or(name.len());
        format!("{}{ELLIPSIS}{}", name.get(..head).unwrap_or_default(), name.get(tail..).unwrap_or_default())
    };
    let (mut low, mut high) = (0, count.saturating_sub(1));
    while low < high {
        let keep = (low + high).div_ceil(2);
        if fits(&cut(keep)) {
            low = keep;
        } else {
            high = keep - 1;
        }
    }
    cut(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(path: &str, width: usize) -> String {
        shown_marked(path, &[], width)
    }

    fn shown_marked(path: &str, marks: &[Range<usize>], width: usize) -> String {
        elide(path, marks, |text| text.chars().count() <= width).into_owned()
    }

    #[test]
    fn a_path_that_fits_is_shown_whole() {
        let path = r"C:\Users\Alex\Desktop";
        assert!(matches!(elide(path, &[], |_| true), Cow::Borrowed(_)));
        assert_eq!(shown(path, 21), path);
    }

    #[test]
    fn the_middle_folders_give_way_to_the_drive_and_the_nearest_folders() {
        let path = r"C:\Users\Alex\Desktop\sfs\src\ui";
        assert_eq!(shown(path, 31), r"C:\…\Alex\Desktop\sfs\src\ui");
        assert_eq!(shown(path, 27), r"C:\…\Desktop\sfs\src\ui");
        assert_eq!(shown(path, 15), r"C:\…\sfs\src\ui");
        assert_eq!(shown(path, 14), r"C:\…\src\ui");
        assert_eq!(shown(path, 7), r"C:\…\ui");
    }

    #[test]
    fn a_folder_the_query_named_stays_while_others_go() {
        let path = r"C:\Users\Alex\Desktop\sfs\target\debug\build";
        let sfs = 22..25;
        assert_eq!(path.get(sfs.clone()), Some("sfs"));
        let sfs = std::slice::from_ref(&sfs);
        assert_eq!(shown_marked(path, sfs, 26), r"C:\…\sfs\…\debug\build");
        assert_eq!(shown_marked(path, sfs, 27), r"C:\…\sfs\target\debug\build");
        assert_eq!(shown_marked(path, sfs, 16), r"C:\…\sfs\…\build");
        assert_eq!(shown(path, 16), r"C:\…\debug\build");
    }

    #[test]
    fn a_network_share_is_kept_as_one_root() {
        assert_eq!(shown(r"\\server\share\projects\2026\reports", 25), r"\\server\share\…\reports");
    }

    #[test]
    fn crumbs_split_on_folders_and_keep_a_share_root_whole() {
        assert_eq!(names(r"C:\…\Alex\Отчёты"), ["C:", "…", "Alex", "Отчёты"]);
        assert_eq!(names(r"\\server\share\…\reports"), [r"\\server\share", "…", "reports"]);
        assert_eq!(names(r"C:\"), ["C:"]);
        assert_eq!(names("Отчёт…артал"), ["Отчёт…артал"]);
    }

    #[test]
    fn a_long_last_folder_drops_the_drive_then_loses_its_middle() {
        let path = r"C:\Program Files\Отчёты за квартал";
        assert_eq!(shown(path, 19), r"…\Отчёты за квартал");
        assert_eq!(shown(path, 11), "Отчёт…артал");
        assert_eq!(shown(path, 0), "…");
    }
}
