use crate::theme::Theme;
use crate::ui::{self, Choice, Picker};
use gpui::{App, ClickEvent, Div, MouseDownEvent, Point, ScrollHandle, WeakEntity, Window};

pub(in crate::app_window) trait MenuId: Copy + PartialEq + 'static {
    fn id(self) -> &'static str;
}

pub(in crate::app_window) struct Menus<M> {
    open: Option<M>,
    scroll: ScrollHandle,
}

impl<M> Default for Menus<M> {
    fn default() -> Self {
        Self { open: None, scroll: ScrollHandle::default() }
    }
}

impl<M: MenuId> Menus<M> {
    pub(in crate::app_window) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    fn toggle(&mut self, menu: M) {
        self.open = if self.open == Some(menu) { None } else { Some(menu) };
        self.scroll.set_offset(Point::default());
    }

    pub(in crate::app_window) fn picker<T>(
        &self,
        menu: M,
        choices: Vec<Choice<T>>,
        current: T,
        theme: &Theme,
    ) -> Picker<T> {
        let open = self.open == Some(menu);
        Picker { id: menu.id(), choices, current, open, under: theme.app.background, scroll: self.scroll.clone() }
    }
}

pub(in crate::app_window) fn dropdown<V: 'static, M: MenuId, T: Clone + PartialEq + 'static>(
    this: WeakEntity<V>,
    menus: fn(&mut V) -> &mut Menus<M>,
    menu: M,
    picker: Picker<T>,
    apply: impl Fn(T, &mut App) + 'static,
    theme: &Theme,
    window: &Window,
) -> Div {
    let close = move |this: &WeakEntity<V>, cx: &mut App| {
        let _ = this.update(cx, |this, cx| {
            if menus(this).close() {
                cx.notify();
            }
        });
    };
    let (toggling, leaving) = (this.clone(), this.clone());
    ui::dropdown(
        picker,
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let _ = toggling.update(cx, |this, cx| {
                menus(this).toggle(menu);
                cx.notify();
            });
        },
        move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| close(&leaving, cx),
        move |value, _, cx| {
            apply(value, cx);
            close(&this, cx);
        },
        theme,
        window,
    )
}
