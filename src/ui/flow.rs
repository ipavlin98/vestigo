use crate::theme::pixels;
use gpui::{Bounds, Hsla, IntoElement, ParentElement, Pixels, Styled, canvas, div, fill};
use std::cell::Cell;
use std::rc::Rc;

/// The selection fill of one list. The chosen row places a `spot` inside its fill box, the
/// `indicator`, painted under the rows, fills it, and the rows' fades read where it is through
/// `under`, so text fading into the fill always fades into what is really under it. Rows never
/// paint the selected fill themselves, so the fill can later move on its own.
#[derive(Clone, Default)]
pub(crate) struct Mark {
    target: Rc<Cell<Option<Bounds<Pixels>>>>,
    shown: Rc<Cell<Option<Bounds<Pixels>>>>,
}

pub(crate) struct Look {
    pub(crate) radius: f32,
    pub(crate) color: Hsla,
}

impl Mark {
    pub(crate) fn spot(&self) -> impl IntoElement + use<> {
        let target = self.target.clone();
        canvas(move |bounds, _, _| target.set(Some(bounds)), |_, _, _, _| {}).absolute().top_0().left_0().size_full()
    }

    pub(crate) fn shown(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> {
        self.shown.clone()
    }

    pub(crate) fn indicator(&self, look: Look) -> impl IntoElement + use<> {
        let (target, shown) = (self.target.clone(), self.shown.clone());
        let paint = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let Some(rect) = target.get() else { return };
                window.paint_quad(fill(rect, look.color).corner_radii(pixels(look.radius, window)));
                shown.set(Some(rect));
            },
        );
        div().absolute().top_0().left_0().size_full().overflow_hidden().child(paint.size_full())
    }
}
