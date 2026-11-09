use crate::theme::{BASE_REM, FADE, Theme, device_pixels, fade, pixels, snap};
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, Render, ShapedLine, SharedString, Style, Task, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};
use history::History;
use std::ops::Range;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use words::{word_at, word_start_after, word_start_before};

mod history;
mod words;

const CONTEXT: &str = "TextInput";
const CARET_WIDTH: f32 = 1.5;
const UNDERLINE: f32 = 1.0;
const SCROLLED: f32 = 0.5;
const BLINK_INTERVAL: Duration = Duration::from_millis(530);

actions!(
    text_input,
    [
        Backspace,
        DeleteWordLeft,
        DeleteWordRight,
        Delete,
        Left,
        Right,
        WordLeft,
        WordRight,
        SelectLeft,
        SelectRight,
        SelectWordLeft,
        SelectWordRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        Paste,
        Cut,
        Copy,
        Undo,
        Redo,
        NextField,
        PreviousField
    ]
);

pub(crate) const FIELD_CONTEXT: &str = "Field";

pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("ctrl-backspace", DeleteWordLeft, Some(CONTEXT)),
        KeyBinding::new("alt-backspace", Undo, Some(CONTEXT)),
        KeyBinding::new("ctrl-delete", DeleteWordRight, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("left", Left, Some(CONTEXT)),
        KeyBinding::new("right", Right, Some(CONTEXT)),
        KeyBinding::new("ctrl-left", WordLeft, Some(CONTEXT)),
        KeyBinding::new("ctrl-right", WordRight, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-right", SelectWordRight, Some(CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("home", Home, Some(CONTEXT)),
        KeyBinding::new("end", End, Some(CONTEXT)),
        KeyBinding::new("ctrl-home", Home, Some(CONTEXT)),
        KeyBinding::new("ctrl-end", End, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-home", SelectHome, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-end", SelectEnd, Some(CONTEXT)),
        KeyBinding::new("ctrl-z", Undo, Some(CONTEXT)),
        KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-z", Redo, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        KeyBinding::new("shift-insert", Paste, Some(CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        KeyBinding::new("shift-delete", Cut, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        KeyBinding::new("ctrl-insert", Copy, Some(CONTEXT)),
        KeyBinding::new("tab", NextField, Some(FIELD_CONTEXT)),
        KeyBinding::new("shift-tab", PreviousField, Some(FIELD_CONTEXT)),
    ]);
}

pub(crate) struct Changed;

pub(crate) struct TextInput {
    focus: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    under: Hsla,
    selected: Range<usize>,
    reversed: bool,
    marked: Option<Range<usize>>,
    layout: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    scroll: Pixels,
    selecting: bool,
    blink: Option<Blink>,
    history: History,
}

struct Blink {
    visible: bool,
    _task: Task<()>,
}

impl EventEmitter<Changed> for TextInput {}

impl TextInput {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: SharedString::default(),
            under: Hsla::default(),
            selected: 0..0,
            reversed: false,
            marked: None,
            layout: None,
            bounds: None,
            scroll: px(0.),
            selecting: false,
            blink: None,
            history: History::default(),
        }
    }

    pub(crate) fn field(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(cx);
        input.focus = input.focus.clone().tab_stop(true);
        input
    }

    pub(crate) fn set_blinking(&mut self, active: bool, cx: &mut Context<Self>) {
        self.blink = active.then(|| Self::blink(cx));
        cx.notify();
    }

    pub(crate) fn blinking(&self) -> bool {
        self.blink.is_some()
    }

    fn show_caret(&mut self, cx: &mut Context<Self>) {
        if self.blink.is_some() {
            self.blink = Some(Self::blink(cx));
        }
    }

    fn blink(cx: &mut Context<Self>) -> Blink {
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let toggled = this.update(cx, |input, cx| {
                    if let Some(blink) = input.blink.as_mut() {
                        blink.visible = !blink.visible;
                        cx.notify();
                    }
                });
                if toggled.is_err() {
                    break;
                }
            }
        });
        Blink { visible: true, _task: task }
    }

    pub(crate) fn text(&self) -> &str {
        &self.content
    }

    pub(crate) fn set_placeholder(&mut self, placeholder: &'static str) {
        self.placeholder = placeholder.into();
    }

    pub(crate) fn set_under(&mut self, under: Hsla, cx: &mut Context<Self>) {
        self.under = under;
        cx.notify();
    }

    pub(crate) fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let end = text.len();
        self.history.clear();
        self.replace(0..self.content.len(), text, end..end, cx);
    }

    fn remember(&mut self, range: &Range<usize>, text: &str) {
        if self.marked.is_some() {
            return;
        }
        let typing = range.is_empty() && !text.is_empty();
        self.history.record((self.content.clone(), self.selected.clone()), typing);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let restored = self.history.undo((self.content.clone(), self.selected.clone()));
        self.restore(restored, cx);
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let restored = self.history.redo((self.content.clone(), self.selected.clone()));
        self.restore(restored, cx);
    }

    fn restore(&mut self, snapshot: Option<(SharedString, Range<usize>)>, cx: &mut Context<Self>) {
        if let Some((text, selected)) = snapshot {
            self.replace(0..self.content.len(), &text, selected, cx);
        }
    }

    pub(crate) fn append(&mut self, text: &str, cx: &mut Context<Self>) {
        let end = self.content.len();
        self.replace(end..end, text, end + text.len()..end + text.len(), cx);
    }

    pub(crate) fn caret_to_end(&mut self, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    pub(crate) fn select_everything(&mut self, cx: &mut Context<Self>) {
        self.selected = 0..self.content.len();
        self.reversed = false;
        cx.notify();
    }

    fn replace(&mut self, range: Range<usize>, text: &str, selected: Range<usize>, cx: &mut Context<Self>) {
        let (Some(before), Some(after)) = (self.content.get(..range.start), self.content.get(range.end..)) else {
            return;
        };
        let changed = range.start != range.end || !text.is_empty();
        self.content = format!("{before}{text}{after}").into();
        self.selected = selected;
        self.reversed = false;
        self.marked = None;
        self.show_caret(cx);
        cx.notify();
        if changed {
            cx.emit(Changed);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() && self.selected.start == 0 {
            cx.propagate();
            return;
        }
        let target = if self.selected.is_empty() { self.previous_boundary(self.cursor()) } else { self.selected.start };
        self.move_to(target, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() && self.selected.end == self.content.len() {
            cx.propagate();
            return;
        }
        let target = if self.selected.is_empty() { self.next_boundary(self.selected.end) } else { self.selected.end };
        self.move_to(target, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.previous_word(self.cursor()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.next_word(self.cursor()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_word(self.cursor()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_word(self.cursor()), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_word(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_everything(cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.is_empty() {
            cx.propagate();
            return;
        }
        if self.selected.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.previous_word(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace(['\r', '\n'], " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        match self.content.get(self.selected.clone()).filter(|text| !text.is_empty()) {
            Some(text) => cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned())),
            None => cx.propagate(),
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_empty() {
            cx.propagate();
            return;
        }
        self.copy(&Copy, window, cx);
        self.replace_text_in_range(None, "", window, cx);
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.stop_propagation();
        let index = self.index_for_position(event.position);
        self.selecting = event.click_count < 2;
        match event.click_count {
            2 => self.select_range(word_at(&self.content, self.index_under(event.position).unwrap_or(index)), cx),
            3.. => self.select_everything(cx),
            _ if event.modifiers.shift => self.select_to(index, cx),
            _ => self.move_to(index, cx),
        }
    }

    fn select_range(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        self.history.pause();
        self.selected = range;
        self.reversed = false;
        self.show_caret(cx);
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn mouse_down_out(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected.is_empty() {
            self.move_to(self.cursor(), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.select_range(offset..offset, cx);
    }

    fn cursor(&self) -> usize {
        if self.reversed { self.selected.start } else { self.selected.end }
    }

    fn index_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(bounds), Some(line)) = (self.bounds.as_ref(), self.layout.as_ref()) else {
            return 0;
        };
        if self.content.is_empty() || position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.closest_index_for_x(position.x - bounds.left() + self.scroll)
    }

    fn index_under(&self, position: Point<Pixels>) -> Option<usize> {
        let (bounds, line) = (self.bounds.as_ref()?, self.layout.as_ref()?);
        line.index_for_x(position.x - bounds.left() + self.scroll)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.reversed {
            self.selected.start = offset;
        } else {
            self.selected.end = offset;
        }
        if self.selected.end < self.selected.start {
            self.reversed = !self.reversed;
            self.selected = self.selected.end..self.selected.start;
        }
        self.show_caret(cx);
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        self.content.get(..offset).map_or(0, |before| before.chars().map(char::len_utf16).sum())
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(index, _)| (index < offset).then_some(index)).unwrap_or(0)
    }

    fn previous_word(&self, offset: usize) -> usize {
        word_start_before(&self.content, offset)
    }

    fn next_word(&self, offset: usize) -> usize {
        word_start_after(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn target_range(&self, range_utf16: Option<&Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selected.clone())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        self.content.get(range).map(str::to_owned)
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected), reversed: self.reversed })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.target_range(range_utf16.as_ref());
        self.remember(&range, text);
        let end = range.start + text.len();
        self.replace(range, text, end..end, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.target_range(range_utf16.as_ref());
        self.remember(&range, text);
        let start = range.start;
        let selected = selected_utf16
            .map(|selected| self.range_from_utf16(&selected))
            .map_or(start + text.len()..start + text.len(), |selected| start + selected.start..start + selected.end);
        self.replace(range, text, selected, cx);
        self.marked = (!text.is_empty()).then(|| start..start + text.len());
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let left = bounds.left() - self.scroll;
        Some(Bounds::from_corners(
            point(left + layout.x_for_index(range.start), bounds.top()),
            point(left + layout.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let local = self.bounds?.localize(&position)?;
        let index = self.layout.as_ref()?.index_for_x(local.x + self.scroll)?;
        Some(self.offset_to_utf16(index))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_down_out(cx.listener(Self::mouse_down_out))
            .child(TextElement { input: cx.entity() })
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct Prepaint {
    line: Option<ShapedLine>,
    scroll: Pixels,
    fades: [Option<PaintQuad>; 2],
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let colors = cx.global::<Theme>().input;
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), colors.placeholder)
        } else {
            (input.content.clone(), colors.text)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked.as_ref().filter(|_| !input.content.is_empty()) {
            Some(marked) => [
                TextRun { len: marked.start, ..run.clone() },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle { color: Some(color), thickness: px(UNDERLINE), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: text.len() - marked.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(text, font_size, &runs, None);
        let cursor = if input.content.is_empty() { 0. } else { f32::from(line.x_for_index(input.cursor())) };
        let scale = window.scale_factor();
        let zoom = f32::from(window.rem_size()) / BASE_REM;
        let caret_width = device_pixels((CARET_WIDTH * zoom * scale).round(), scale);
        let room = f32::from(bounds.size.width - caret_width);
        let scroll = if input.focus.is_focused(window) {
            px(follow(f32::from(input.scroll), cursor, f32::from(line.width), room))
        } else {
            px(0.)
        };
        let left = bounds.left() - scroll;
        let fade_width = pixels(FADE, window).min(bounds.size.width / 2.);
        let edge = size(fade_width, bounds.size.height);
        let fades = [
            (scroll > px(SCROLLED)).then(|| fill(Bounds::new(bounds.origin, edge), fade(input.under, 270.))),
            (line.width - scroll > bounds.size.width + px(SCROLLED)).then(|| {
                let origin = point(bounds.right() - fade_width, bounds.top());
                fill(Bounds::new(origin, edge), fade(input.under, 90.))
            }),
        ];
        let (caret, selection) = if input.selected.is_empty() {
            let caret = fill(
                Bounds::new(point(snap(left + px(cursor), scale), bounds.top()), size(caret_width, bounds.size.height)),
                colors.caret,
            );
            let shown = input.blink.as_ref().is_some_and(|blink| blink.visible);
            (shown.then_some(caret), None)
        } else {
            let selection = fill(
                Bounds::from_corners(
                    point(snap(left + line.x_for_index(input.selected.start), scale), bounds.top()),
                    point(snap(left + line.x_for_index(input.selected.end), scale), bounds.bottom()),
                ),
                colors.selection,
            );
            (None, Some(selection))
        };
        Prepaint { line: Some(line), scroll, fades, caret, selection }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let input = self.input.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && input.read(cx).selecting {
                input.update(cx, |input, cx| input.select_to(input.index_for_position(event.position), cx));
            }
        });
        let Some(line) = prepaint.line.take() else { return };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            let origin = point(bounds.left() - prepaint.scroll, bounds.top());
            if let Err(error) = line.paint(origin, window.line_height(), TextAlign::Left, None, window, cx) {
                log::warn!("Couldn't paint the query: {error:#}");
            }
            for edge in prepaint.fades.iter_mut().filter_map(Option::take) {
                window.paint_quad(edge);
            }
            if focus.is_focused(window)
                && let Some(caret) = prepaint.caret.take()
            {
                window.paint_quad(caret);
            }
        });
        let scroll = prepaint.scroll;
        self.input.update(cx, |input, _| {
            input.layout = Some(line);
            input.bounds = Some(bounds);
            input.scroll = scroll;
        });
    }
}

fn follow(scroll: f32, cursor: f32, width: f32, room: f32) -> f32 {
    let room = room.max(0.);
    scroll.clamp(cursor - room, cursor).clamp(0., (width - room).max(0.))
}

#[cfg(test)]
mod tests {
    use super::follow;

    #[test]
    fn short_text_never_scrolls() {
        assert_eq!(follow(0., 40., 60., 100.), 0.);
        assert_eq!(follow(30., 40., 60., 100.), 0.);
    }

    #[test]
    fn caret_past_the_right_edge_scrolls_it_into_view() {
        assert_eq!(follow(0., 250., 250., 100.), 150.);
    }

    #[test]
    fn caret_left_of_the_view_scrolls_back() {
        assert_eq!(follow(150., 20., 250., 100.), 20.);
    }

    #[test]
    fn deleting_text_pulls_the_view_back() {
        assert_eq!(follow(150., 120., 120., 100.), 20.);
    }

    #[test]
    fn zero_width_field_does_not_panic() {
        assert_eq!(follow(5., 10., 20., -1.5), 10.);
    }
}
