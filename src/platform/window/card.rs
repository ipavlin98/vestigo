use super::floating::Floating;
use super::{PANEL, physical, scale, work_area};
use crate::theme::{Chrome, zoom_at};
use std::cell::RefCell;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromRect};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardLayout {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) gap: f32,
    pub(crate) zoom: f32,
    pub(crate) chrome: Chrome,
}

thread_local! {
    static CARD: RefCell<Option<Floating>> = const { RefCell::new(None) };
    static LAYOUT: RefCell<Option<CardLayout>> = const { RefCell::new(None) };
}

pub(crate) fn attach_card(window: &gpui::Window, cx: &mut gpui::App) {
    CARD.set(Floating::attach(window, false, "preview", cx));
}

pub(crate) fn place_card(layout: Option<CardLayout>) {
    LAYOUT.set(layout);
    follow();
}

pub(super) fn follow() {
    let layout = LAYOUT.with_borrow(Clone::clone);
    let (Some(layout), Some(panel)) = (layout, PANEL.get()) else {
        hide();
        return;
    };
    let monitor = unsafe { MonitorFromRect(&panel, MONITOR_DEFAULTTONEAREST) };
    let scale = scale(monitor);
    let ratio = scale * zoom_at(layout.zoom, scale);
    let rect = beside(panel, work_area(monitor), [layout.width, layout.height, layout.gap].map(|v| physical(v, ratio)));
    with_card(|card| card.place(rect, ratio, &layout.chrome, None));
}

fn beside(panel: RECT, work: RECT, [width, height, gap]: [i32; 3]) -> RECT {
    let right = panel.right + gap;
    let left = panel.left - gap - width;
    let x = if right + width <= work.right || left < work.left { right } else { left };
    let y = panel.top.min(work.bottom - height).max(work.top);
    RECT { left: x, top: y, right: x + width, bottom: y + height }
}

pub(super) fn hide() {
    with_card(|card| card.hide());
}

fn with_card(job: impl FnOnce(&mut Floating)) {
    CARD.with(|cell| {
        if let Ok(mut card) = cell.try_borrow_mut()
            && let Some(card) = card.as_mut()
        {
            job(card);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT { left: 0, top: 0, right: 1000, bottom: 800 };

    #[test]
    fn the_card_sits_right_of_the_panel_when_it_fits() {
        let panel = RECT { left: 100, top: 100, right: 500, bottom: 300 };
        assert_eq!(beside(panel, WORK, [300, 400, 10]), RECT { left: 510, top: 100, right: 810, bottom: 500 });
    }

    #[test]
    fn the_card_flips_left_when_the_right_side_is_short() {
        let panel = RECT { left: 500, top: 100, right: 900, bottom: 300 };
        assert_eq!(beside(panel, WORK, [300, 400, 10]), RECT { left: 190, top: 100, right: 490, bottom: 500 });
    }

    #[test]
    fn the_card_stays_inside_the_work_area_vertically() {
        let panel = RECT { left: 100, top: 600, right: 500, bottom: 700 };
        assert_eq!(beside(panel, WORK, [300, 400, 10]).bottom, WORK.bottom);
    }
}
