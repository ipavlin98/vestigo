use super::{glass, outer};
use std::iter::successors;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    GW_HWNDPREV, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

const ENOUGH_IN_SIGHT: f32 = 0.5;
const GRID: i32 = 32;
const MOST_WINDOWS: usize = 4096;

/// Whether most of the window shows past the other programs' windows above it.
pub(super) fn in_sight(hwnd: HWND) -> bool {
    let above = |window: &HWND| unsafe { GetWindow(*window, GW_HWNDPREV) }.ok();
    let covering: Vec<RECT> =
        successors(above(&hwnd), above).take(MOST_WINDOWS).filter(|window| covers(*window)).map(outer).collect();
    shown_share(outer(hwnd), &covering) >= ENOUGH_IN_SIGHT
}

fn covers(window: HWND) -> bool {
    let mut process = 0;
    let mut cloaked = 0u32;
    let size = size_of::<u32>() as u32;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process));
        let read = DwmGetWindowAttribute(window, DWMWA_CLOAKED, (&raw mut cloaked).cast(), size);
        process != std::process::id()
            && IsWindowVisible(window).as_bool()
            && !IsIconic(window).as_bool()
            && (glass::styles(window) & (WS_EX_TRANSPARENT | WS_EX_LAYERED)).0 == 0
            && (read.is_err() || cloaked == 0)
    }
}

fn shown_share(window: RECT, above: &[RECT]) -> f32 {
    let (width, height) = (window.right - window.left, window.bottom - window.top);
    if width <= 0 || height <= 0 {
        return 0.0;
    }
    let at = |start: i32, length: i32, cell: i32| start + (2 * cell + 1) * length / (2 * GRID);
    let shown = (0..GRID)
        .flat_map(|row| (0..GRID).map(move |column| (column, row)))
        .filter(|(column, row)| {
            let (x, y) = (at(window.left, width, *column), at(window.top, height, *row));
            !above.iter().any(|cover| x >= cover.left && x < cover.right && y >= cover.top && y < cover.bottom)
        })
        .count();
    shown as f32 / (GRID * GRID) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: RECT = RECT { left: 100, top: 100, right: 1100, bottom: 700 };

    #[test]
    fn an_uncovered_window_shows_whole() {
        assert_eq!(shown_share(WINDOW, &[]), 1.0);
    }

    #[test]
    fn a_window_behind_a_maximized_one_shows_nothing() {
        assert_eq!(shown_share(WINDOW, &[RECT { left: 0, top: 0, right: 1920, bottom: 1040 }]), 0.0);
    }

    #[test]
    fn windows_beside_it_do_not_cover_it() {
        assert_eq!(shown_share(WINDOW, &[RECT { left: 1100, top: 0, right: 1920, bottom: 1040 }]), 1.0);
    }

    #[test]
    fn overlapping_covers_count_once() {
        let left = RECT { left: 0, top: 0, right: 600, bottom: 1040 };
        let shown = shown_share(WINDOW, &[left, left]);
        assert!((shown - 0.5).abs() < 0.04, "half of it shows, got {shown}");
    }

    #[test]
    fn an_empty_window_shows_nothing() {
        assert_eq!(shown_share(RECT::default(), &[]), 0.0);
    }
}
