use super::floating::Floating;
use super::{REGION, Shape, bring_to_front, cursor, on_screen, ours, outer, physical, prompt, scale, work_area};
use crate::theme::{Chrome, zoom_at};
use parking_lot::Mutex;
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint, MonitorFromRect};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindowVisible, SetForegroundWindow};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MenuSpot {
    pub(crate) width: f32,
    pub(crate) heights: Vec<f32>,
    pub(crate) zoom: f32,
    pub(crate) chrome: Chrome,
    pub(crate) at: MenuAt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MenuAt {
    Cursor,
    Search { button: usize, strip: [f32; 2], top: f32, bottom: f32, upward: bool },
    Beside { panel: usize, top: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerMove {
    Still,
    Free,
    Aiming,
}

struct Spot {
    rect: RECT,
    ratio: f32,
    hanging: Option<Hanging>,
}

struct Hanging {
    shade: Shape,
    strip: RECT,
}

struct Panel {
    floating: Floating,
    rect: RECT,
    ratio: f32,
}

thread_local! {
    static PANELS: RefCell<Vec<Option<Panel>>> = const { RefCell::new(Vec::new()) };
    static ANCHOR: Cell<Option<POINT>> = const { Cell::new(None) };
    static STRIP: Cell<Option<RECT>> = const { Cell::new(None) };
    static PREVIOUS: Cell<Option<HWND>> = const { Cell::new(None) };
    static POINTER: Cell<Option<POINT>> = const { Cell::new(None) };
}

static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

pub(crate) fn attach_menu(window: &gpui::Window, panel: usize, activates: bool, cx: &mut gpui::App) {
    let Some(floating) = Floating::attach(window, activates, "menu", cx) else { return };
    WINDOWS.lock().push(floating.hwnd().0 as isize);
    PANELS.with_borrow_mut(|panels| {
        if panels.len() <= panel {
            panels.resize_with(panel + 1, || None);
        }
        if let Some(slot) = panels.get_mut(panel) {
            *slot = Some(Panel { floating, rect: RECT::default(), ratio: 1.0 });
        }
    });
}

pub(super) fn is_menu(hwnd: HWND) -> bool {
    WINDOWS.lock().contains(&(hwnd.0 as isize))
}

pub(crate) fn place_menu(panel: usize, spot: MenuSpot, replaced: Option<usize>) -> bool {
    prompt::placed();
    let Some(Spot { rect, ratio, hanging }) = spot_rect(&spot) else { return false };
    let root = match spot.at {
        MenuAt::Cursor | MenuAt::Search { .. } => true,
        MenuAt::Beside { .. } => false,
    };
    if root {
        STRIP.set(hanging.as_ref().map(|hanging| hanging.strip));
    }
    let mut placed = None;
    PANELS.with_borrow_mut(|panels| {
        let Some(Some(target)) = panels.get_mut(panel) else { return };
        let first = root && !target.floating.visible();
        if first {
            PREVIOUS.set(Some(unsafe { GetForegroundWindow() }));
        }
        target.floating.place(rect, ratio, &spot.chrome, hanging.map(|hanging| hanging.shade));
        target.rect = rect;
        target.ratio = ratio;
        placed = Some((target.floating.hwnd(), first));
        if let Some(Some(old)) = replaced.filter(|&old| old != panel).and_then(|old| panels.get(old)) {
            old.floating.hide();
        }
    });
    match placed {
        Some((hwnd, true)) => bring_to_front(hwnd),
        Some((_, false)) => true,
        None => false,
    }
}

pub(crate) fn hide_menus(panels: &[usize]) {
    PANELS.with_borrow(|all| {
        for panel in panels {
            if let Some(Some(panel)) = all.get(*panel) {
                panel.floating.hide();
            }
        }
    });
}

pub(crate) fn close_menu(restore: bool) {
    ANCHOR.set(None);
    STRIP.set(None);
    POINTER.set(None);
    let previous = PREVIOUS.take().filter(|previous| !previous.is_invalid());
    if let Some(previous) = previous.filter(|&previous| restore && unsafe { IsWindowVisible(previous) }.as_bool()) {
        let _ = unsafe { SetForegroundWindow(previous) };
    }
    PANELS.with_borrow(|panels| {
        for panel in panels.iter().rev().flatten() {
            panel.floating.hide();
        }
    });
}

pub(crate) fn pointer_move(submenu: Option<usize>) -> PointerMove {
    let now = cursor();
    let before = POINTER.replace(Some(now));
    let shown = |panel: usize| {
        PANELS.with_borrow(|panels| {
            panels.get(panel)?.as_ref().filter(|panel| panel.floating.visible()).map(|panel| panel.rect)
        })
    };
    classify(before, now, submenu.and_then(shown))
}

fn classify(before: Option<POINT>, now: POINT, submenu: Option<RECT>) -> PointerMove {
    let Some(before) = before.filter(|&before| before != now) else { return PointerMove::Still };
    match submenu {
        Some(rect) if heading_into(before, now, rect) => PointerMove::Aiming,
        Some(_) | None => PointerMove::Free,
    }
}

pub(crate) fn pointer_on_panel(panel: usize) -> bool {
    let at = cursor();
    PANELS.with_borrow(|panels| {
        panels
            .get(panel)
            .and_then(Option::as_ref)
            .is_some_and(|panel| panel.floating.visible() && holds(panel.rect, at))
    })
}

pub(super) fn press_stays_in_menu() -> bool {
    STRIP.get().is_some_and(|strip| holds(strip, cursor()))
}

fn holds(rect: RECT, point: POINT) -> bool {
    (rect.left..rect.right).contains(&point.x) && (rect.top..rect.bottom).contains(&point.y)
}

pub(super) fn shown() -> bool {
    PANELS.with_borrow(|panels| panels.iter().flatten().any(|panel| panel.floating.visible()))
}

fn heading_into(from: POINT, to: POINT, rect: RECT) -> bool {
    let edge = if rect.left >= from.x { rect.left } else { rect.right };
    let corners = [from, POINT { x: edge, y: rect.top }, POINT { x: edge, y: rect.bottom }];
    let side = |a: POINT, b: POINT| {
        i64::from(b.x - a.x) * i64::from(to.y - a.y) - i64::from(b.y - a.y) * i64::from(to.x - a.x)
    };
    let sides = [side(corners[0], corners[1]), side(corners[1], corners[2]), side(corners[2], corners[0])];
    sides.iter().all(|side| *side >= 0) || sides.iter().all(|side| *side <= 0)
}

fn spot_rect(spot: &MenuSpot) -> Option<Spot> {
    match spot.at {
        MenuAt::Cursor => {
            let point = match ANCHOR.get() {
                Some(point) => point,
                None => {
                    let point = cursor();
                    ANCHOR.set(Some(point));
                    point
                }
            };
            let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
            let ratio = ratio(scale(monitor), spot.zoom);
            let work = work_area(monitor);
            Some(Spot { rect: at_point(point, work, size(spot, ratio, work)), ratio, hanging: None })
        }
        MenuAt::Search { strip: [left, right], top, bottom, upward, .. } => {
            let search = ours()?;
            let window = outer(search);
            let region = REGION.get()?;
            let monitor = unsafe { MonitorFromRect(&window, MONITOR_DEFAULTTONEAREST) };
            let ratio = ratio(scale(monitor), spot.zoom);
            let span = [window.top + physical(top, ratio), window.top + physical(bottom, ratio)];
            let panel = on_screen(search, region);
            let work = work_area(monitor);
            let room = if upward { span[0] - work.top } else { work.bottom - span[1] };
            let rect = hanging(span, panel, upward, work, fitting(&spot.heights, ratio, room));
            let own = Shape::new(rect, spot.chrome.corners, ratio).radii;
            let shade = joined(panel, region.radii, rect, own, upward);
            let strip = RECT {
                left: window.left + physical(left, ratio),
                top: span[0],
                right: window.left + physical(right, ratio),
                bottom: span[1],
            };
            Some(Spot { rect, ratio, hanging: Some(Hanging { shade, strip }) })
        }
        MenuAt::Beside { panel, top } => {
            let (parent, parent_ratio) =
                PANELS.with_borrow(|panels| panels.get(panel)?.as_ref().map(|panel| (panel.rect, panel.ratio)))?;
            let monitor = unsafe { MonitorFromRect(&parent, MONITOR_DEFAULTTONEAREST) };
            let ratio = ratio(scale(monitor), spot.zoom);
            let row = parent.top + physical(top, parent_ratio);
            let work = work_area(monitor);
            Some(Spot { rect: beside(parent, row, work, size(spot, ratio, work)), ratio, hanging: None })
        }
    }
}

fn ratio(scale: f32, zoom: f32) -> f32 {
    scale * zoom_at(zoom, scale)
}

fn size(spot: &MenuSpot, ratio: f32, work: RECT) -> [i32; 2] {
    [physical(spot.width, ratio), fitting(&spot.heights, ratio, work.bottom - work.top)]
}

fn fitting(heights: &[f32], ratio: f32, room: i32) -> i32 {
    heights.iter().rev().map(|&height| physical(height, ratio)).find(|&height| height <= room).unwrap_or(room)
}

fn at_point(point: POINT, work: RECT, [width, height]: [i32; 2]) -> RECT {
    let x = if point.x + width <= work.right { point.x } else { point.x - width };
    let y = if point.y + height <= work.bottom { point.y } else { point.y - height };
    fitted(x, y, work, [width, height])
}

fn hanging([top, bottom]: [i32; 2], panel: RECT, upward: bool, work: RECT, height: i32) -> RECT {
    if upward {
        let height = height.min(top - work.top);
        RECT { left: panel.left, top: top - height, right: panel.right, bottom: top }
    } else {
        let height = height.min(work.bottom - bottom);
        RECT { left: panel.left, top: bottom, right: panel.right, bottom: bottom + height }
    }
}

fn joined(
    panel: RECT,
    [top_left, top_right, bottom_right, bottom_left]: [i32; 4],
    rect: RECT,
    own: [i32; 4],
    upward: bool,
) -> Shape {
    let [own_top_left, own_top_right, own_bottom_right, own_bottom_left] = own;
    if upward {
        Shape { rect: RECT { top: rect.top, ..panel }, radii: [own_top_left, own_top_right, bottom_right, bottom_left] }
    } else {
        Shape {
            rect: RECT { bottom: rect.bottom, ..panel },
            radii: [top_left, top_right, own_bottom_right, own_bottom_left],
        }
    }
}

fn beside(parent: RECT, top: i32, work: RECT, [width, height]: [i32; 2]) -> RECT {
    let right = parent.right;
    let left = parent.left - width;
    let x = if right + width <= work.right || left < work.left { right } else { left };
    fitted(x, top, work, [width, height])
}

fn fitted(x: i32, y: i32, work: RECT, [width, height]: [i32; 2]) -> RECT {
    let x = x.min(work.right - width).max(work.left);
    let y = y.min(work.bottom - height).max(work.top);
    RECT { left: x, top: y, right: x + width, bottom: y + height }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT { left: 0, top: 0, right: 1000, bottom: 800 };

    const SUBMENU: RECT = RECT { left: 400, top: 100, right: 600, bottom: 500 };

    #[test]
    fn moving_toward_a_submenu_is_aiming_at_it() {
        assert!(heading_into(POINT { x: 300, y: 300 }, POINT { x: 310, y: 290 }, SUBMENU));
        assert!(heading_into(POINT { x: 300, y: 300 }, POINT { x: 310, y: 312 }, SUBMENU));
    }

    #[test]
    fn moving_along_the_parent_or_away_is_not_aiming() {
        assert!(!heading_into(POINT { x: 300, y: 300 }, POINT { x: 300, y: 330 }, SUBMENU));
        assert!(!heading_into(POINT { x: 300, y: 300 }, POINT { x: 290, y: 300 }, SUBMENU));
    }

    #[test]
    fn a_message_without_a_new_position_is_not_a_pointer_move() {
        let at = POINT { x: 300, y: 300 };
        assert_eq!(classify(Some(at), at, Some(SUBMENU)), PointerMove::Still, "Windows repeats it when a window hides");
        assert_eq!(classify(None, at, None), PointerMove::Still, "the first sighting after opening");
        assert_eq!(classify(Some(at), POINT { x: 300, y: 330 }, None), PointerMove::Free);
        assert_eq!(classify(Some(at), POINT { x: 300, y: 330 }, Some(SUBMENU)), PointerMove::Free);
        assert_eq!(classify(Some(at), POINT { x: 310, y: 290 }, Some(SUBMENU)), PointerMove::Aiming);
    }

    #[test]
    fn a_submenu_on_the_left_is_aimed_at_leftward() {
        let left = RECT { left: 0, top: 100, right: 200, bottom: 500 };
        assert!(heading_into(POINT { x: 300, y: 300 }, POINT { x: 290, y: 305 }, left));
        assert!(!heading_into(POINT { x: 300, y: 300 }, POINT { x: 310, y: 300 }, left));
    }

    #[test]
    fn a_menu_opens_down_and_right_of_the_cursor() {
        let rect = at_point(POINT { x: 100, y: 200 }, WORK, [300, 400]);
        assert_eq!(rect, RECT { left: 100, top: 200, right: 400, bottom: 600 });
    }

    #[test]
    fn a_menu_near_the_corner_opens_up_and_left_of_the_cursor() {
        let rect = at_point(POINT { x: 900, y: 700 }, WORK, [300, 400]);
        assert_eq!(rect, RECT { left: 600, top: 300, right: 900, bottom: 700 });
    }

    #[test]
    fn a_menu_taller_than_the_screen_stays_inside_it() {
        let rect = at_point(POINT { x: 100, y: 500 }, WORK, [300, 900]);
        assert_eq!((rect.top, rect.bottom), (0, 900));
    }

    #[test]
    fn a_menu_takes_the_tallest_height_that_fits_on_the_screen() {
        let heights = [40.0, 76.0, 112.0];
        assert_eq!(fitting(&heights, 1.0, 800), 112);
        assert_eq!(fitting(&heights, 1.0, 100), 76);
        assert_eq!(fitting(&heights, 2.0, 200), 152);
    }

    const BAR: [i32; 2] = [100, 140];

    #[test]
    fn a_bar_menu_hangs_flush_from_the_bar_at_its_full_width() {
        let panel = RECT { left: 200, top: 100, right: 760, bottom: 140 };
        assert_eq!(hanging(BAR, panel, false, WORK, 300), RECT { left: 200, top: 140, right: 760, bottom: 440 });
        let panel = RECT { left: 200, top: 400, right: 760, bottom: 440 };
        assert_eq!(hanging([400, 440], panel, true, WORK, 300), RECT { left: 200, top: 100, right: 760, bottom: 400 });
    }

    #[test]
    fn a_bar_menu_is_as_tall_as_its_items_whatever_the_bar_showed_before() {
        let panel = RECT { left: 200, top: 100, right: 760, bottom: 500 };
        assert_eq!(hanging(BAR, panel, false, WORK, 120).bottom, 260);
        assert_eq!(hanging([460, 500], panel, true, WORK, 120).top, 340);
    }

    #[test]
    fn a_bar_menu_shrinks_to_the_screen_instead_of_leaving_the_bar() {
        let panel = RECT { left: 200, top: 600, right: 760, bottom: 640 };
        assert_eq!(hanging([600, 640], panel, false, WORK, 300), RECT { left: 200, top: 640, right: 760, bottom: 800 });
    }

    #[test]
    fn only_the_strip_itself_holds_a_press() {
        let strip = RECT { left: 700, top: 100, right: 760, bottom: 140 };
        assert!(holds(strip, POINT { x: 700, y: 100 }));
        assert!(holds(strip, POINT { x: 759, y: 139 }));
        assert!(!holds(strip, POINT { x: 699, y: 120 }), "the query field beside the buttons");
        assert!(!holds(strip, POINT { x: 760, y: 120 }));
        assert!(!holds(strip, POINT { x: 720, y: 140 }));
    }

    #[test]
    fn a_bar_and_its_menu_cast_one_shadow_rounded_at_their_far_ends() {
        let panel = RECT { left: 200, top: 100, right: 760, bottom: 140 };
        let menu = RECT { left: 200, top: 140, right: 760, bottom: 440 };
        let shade = joined(panel, [2, 3, 8, 8], menu, [0, 0, 6, 7], false);
        assert_eq!((shade.rect, shade.radii), (RECT { left: 200, top: 100, right: 760, bottom: 440 }, [2, 3, 6, 7]));
        let panel = RECT { left: 200, top: 400, right: 760, bottom: 440 };
        let menu = RECT { left: 200, top: 100, right: 760, bottom: 400 };
        let shade = joined(panel, [8, 8, 2, 3], menu, [6, 7, 0, 0], true);
        assert_eq!((shade.rect, shade.radii), (RECT { left: 200, top: 100, right: 760, bottom: 440 }, [6, 7, 2, 3]));
    }

    #[test]
    fn a_submenu_lines_up_with_its_row_right_of_the_parent() {
        let parent = RECT { left: 100, top: 100, right: 400, bottom: 500 };
        assert_eq!(beside(parent, 180, WORK, [200, 300]), RECT { left: 400, top: 180, right: 600, bottom: 480 });
    }

    #[test]
    fn a_submenu_flips_left_and_rises_when_the_screen_ends() {
        let parent = RECT { left: 600, top: 100, right: 900, bottom: 700 };
        assert_eq!(beside(parent, 600, WORK, [200, 300]), RECT { left: 400, top: 500, right: 600, bottom: 800 });
    }
}
