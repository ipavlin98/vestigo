use super::SearchPage;
use super::table::{Align, Column};
use crate::app_window::menus::dropdown;
use crate::app_window::settings_page::Menu;
use crate::app_window::view::{status_bar, top_bar};
use crate::assets;
use crate::i18n::{Language, Strings};
use crate::index::{Hit, Status};
use crate::search::Period;
use crate::settings::Settings;
use crate::theme::{Theme, rem};
use crate::ui::{Browser, Look, Mark, Under};
use crate::ui::{
    Choice, DETAIL_LINE, DETAIL_SIZE, Edge, GUTTER, INDENT, INSET, Listing, NAME_LINE, PREVIEW_WIDTH, Surface, Tip,
    TipAnchor, TogglePreview, divider, faded, glyph, highlighted_name, icon_tip, icon_toggle, note_click, path_label,
    result_hints,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, Render,
    SharedString, Window, div, prelude::*, uniform_list,
};

pub(in crate::app_window) const ICON: f32 = 16.0;
const HEADER: f32 = 32.0;
const HEADER_TOP: f32 = 4.0;
const ROW: f32 = 36.0;
const COLUMN_GAP: f32 = 24.0;
const ICON_GAP: f32 = 8.0;
const SORT_GAP: f32 = 4.0;
const SORT_GLYPH: f32 = 12.0;
const NAME_SHARE: f32 = 2.0;
const FOLDER_SHARE: f32 = 3.0;
const SIZE_COLUMN: f32 = 80.0;
const MODIFIED_COLUMN: f32 = 112.0;

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let language = *cx.global::<Language>();
        self.page(theme, language, window, cx)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::pick_next))
            .on_action(cx.listener(Self::pick_previous))
            .on_action(cx.listener(Self::confirm_selected))
            .on_action(cx.listener(Self::reveal_selected))
            .on_action(cx.listener(Self::run_action))
            .on_action(cx.listener(Self::show_actions_selected))
            .on_action(cx.listener(Self::enter_folder))
            .on_action(cx.listener(Self::leave_folder))
            .on_action(cx.listener(Self::enter_containing))
            .on_action(cx.listener(Self::jump))
            .on_action(cx.listener(Self::set_mark))
            .on_action(cx.listener(Self::go_to_mark))
            .on_action(cx.listener(Self::back))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::toggle_preview))
    }
}

impl SearchPage {
    fn page(&self, theme: &Theme, language: Language, window: &Window, cx: &Context<Self>) -> Div {
        let status = *cx.global::<Status>();
        let shows_preview = Self::shows_preview(cx);
        let band = TipAnchor::default();
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                top_bar(self.input.clone(), theme, window)
                    .relative()
                    .gap(rem(GUTTER))
                    .child(band.track())
                    .child(self.date_picker(theme, language, window, cx))
                    .child(
                        icon_tip(
                            icon_toggle("preview", assets::PREVIEW, shows_preview, theme),
                            Tip::keys(&TogglePreview, window, cx).map(|tip| tip.along(band.clone())),
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(&TogglePreview, window, cx))),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(self.header(theme, language.strings(), cx))
                            .child(self.results(theme, language, cx)),
                    )
                    .when(shows_preview, |page| {
                        let surface = Surface { under: theme.app.background, line: theme.app.divider };
                        page.child(
                            div()
                                .relative()
                                .w(rem(PREVIEW_WIDTH))
                                .flex_none()
                                .h_full()
                                .child(self.preview.render(
                                    theme,
                                    language,
                                    surface,
                                    cx.global::<Settings>().hide_shortcut_extensions,
                                ))
                                .child(divider(Edge::Left, theme.app.divider)),
                        )
                    })
                    .child(divider(Edge::Top, theme.app.divider)),
            )
            .child(self.footer(theme, language, status, window))
    }

    fn date_picker(&self, theme: &Theme, language: Language, window: &Window, cx: &Context<Self>) -> Div {
        let strings = language.strings();
        let counts = self.counts.as_ref();
        let note = |count: Option<usize>| count.map(|count| language.number(count).into());
        let any = Choice {
            value: None,
            label: strings.period_any.into(),
            note: note(counts.map(|counts| counts.any_date)),
            swatch: None,
        };
        let periods = Period::ALL.into_iter().enumerate().map(|(position, period)| {
            let label = match period {
                Period::Today => strings.period_today,
                Period::Week => strings.period_week,
                Period::Month => strings.period_month,
                Period::Year => strings.period_year,
                Period::Older => strings.period_older,
            };
            let found = counts.and_then(|counts| counts.dates.get(position).copied());
            Choice { value: Some(period), label: label.into(), note: note(found), swatch: None }
        });
        let choices = std::iter::once(any).chain(periods).collect();
        let picker = self.menus.picker(Menu::Date, choices, self.date, theme);
        let this = cx.weak_entity();
        let page = this.clone();
        let apply = move |date, cx: &mut gpui::App| {
            let _ = this.update(cx, |this, cx| this.choose_date(date, cx));
        };
        dropdown(page, |this: &mut Self| &mut this.menus, Menu::Date, picker, apply, theme, window)
    }

    fn header(&self, theme: &Theme, strings: &Strings, cx: &Context<Self>) -> impl IntoElement {
        let colors = theme.header;
        let cells = Self::shown_columns(cx).map(|column| {
            let label = match column {
                Column::Name => strings.column_name,
                Column::Folder => strings.column_folder,
                Column::Size => strings.column_size,
                Column::Modified => strings.column_modified,
            };
            let arrow = self.sort.filter(|sort| sort.column == column).map(|sort| {
                glyph(if sort.descending { assets::ARROW_DOWN } else { assets::ARROW_UP }, SORT_GLYPH, colors.arrow)
            });
            let cell = column_cell(div().id(("column", column as usize)), column)
                .h_full()
                .gap(rem(SORT_GAP))
                .cursor_pointer()
                .block_mouse_except_scroll()
                .hover(|style| style.text_color(colors.hover_text));
            match column.align() {
                Align::Start => cell.child(label).children(arrow),
                Align::End => cell.children(arrow).child(label),
            }
            .on_click(cx.listener(move |this, _, _, cx| this.sort_by(column, cx)))
        });
        div()
            .relative()
            .h(rem(HEADER_TOP + HEADER))
            .flex_none()
            .pt(rem(HEADER_TOP))
            .px(rem(INSET))
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(colors.text)
            .child(columns(div().h_full()).children(cells))
    }

    fn results(&self, theme: &Theme, language: Language, cx: &Context<Self>) -> AnyElement {
        let container = div().flex_1().min_h_0().relative().pt(rem(INSET));
        if self.results.hits().is_empty() {
            let message = self.results.empty_message(language);
            return container
                .child(
                    div()
                        .h(rem(ROW))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.results.empty)
                        .child(message),
                )
                .into_any_element();
        }
        let mark = Mark::default();
        let indicator = mark.indicator(Look { radius: theme.radius.large, color: theme.results.selected });
        let list = uniform_list(
            "files",
            self.results.hits().len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let theme = cx.global::<Theme>();
                range
                    .filter_map(|position| {
                        let hit = this.results.hits().get(position)?;
                        Some(this.row(position, hit, &mark, theme, language, cx))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.results.scroll())
        .size_full();
        let glide = self.results.glide(ROW);
        container.child(div().relative().size_full().child(glide).child(indicator).child(list)).into_any_element()
    }

    fn row(
        &self,
        position: usize,
        hit: &Hit,
        mark: &Mark,
        theme: &Theme,
        language: Language,
        cx: &Context<Self>,
    ) -> AnyElement {
        let colors = theme.results;
        let selected = self.results.is_selected(position);
        let picked = self.results.is_picked(position);
        let base = if picked && !selected { colors.picked } else { theme.app.background };
        let details = self.details.get(&hit.path).copied().unwrap_or_default();
        let size = details.size.map(|size| language.size(size)).unwrap_or_default();
        let modified = details.modified.map(|stamp| language.date(stamp)).unwrap_or_default();
        let detail = |content: String, column: Column| {
            column_cell(div(), column)
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(colors.detail)
                .child(content)
        };
        let plain = cx.global::<Settings>().hide_shortcut_extensions;
        let anchor = TipAnchor::default();
        let name = column_cell(div(), Column::Name)
            .gap(rem(ICON_GAP))
            .child(self.results.icon(hit, ICON, theme, cx))
            .child(
                faded(
                    "name",
                    highlighted_name(hit, plain, theme),
                    Tip::new(hit.label(plain).to_owned()).along(anchor.clone()).at_text(),
                    NAME_LINE,
                    Under::marked(base, colors.selected, mark),
                )
                .flex_1()
                .min_w_0(),
            )
            .when(picked, |cell| cell.child(glyph(assets::CHECK, DETAIL_LINE, colors.shortcut)));
        let folder = hit.folder().to_owned();
        let folder = column_cell(div(), Column::Folder).child(
            path_label(
                "folder",
                SharedString::from(folder.clone()),
                hit.marks.clone(),
                Tip::new(folder).along(anchor.clone()).at_text(),
                DETAIL_LINE,
            )
            .flex_1()
            .min_w_0()
            .text_size(rem(DETAIL_SIZE))
            .text_color(colors.path),
        );
        let radius = theme.radius.large;
        div()
            .id(position)
            .w_full()
            .h(rem(ROW))
            .px(rem(INSET))
            .block_mouse_except_scroll()
            .child(
                columns(
                    div()
                        .relative()
                        .size_full()
                        .rounded(rem(radius))
                        .cursor_pointer()
                        .when(picked && !selected, |row| row.bg(base))
                        .when(selected, |row| row.child(mark.spot())),
                )
                .child(anchor.track())
                .child(name)
                .when(!Self::shows_preview(cx), |row| row.child(folder))
                .child(detail(size, Column::Size))
                .child(detail(modified, Column::Modified)),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                this.point_at(position, event.position, window.bounds().origin, cx);
                this.results.drag(event, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, _| {
                    note_click(event);
                    this.results.press(position, event.position);
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| this.click(position, event, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| this.show_actions(position, cx)))
            .into_any_element()
    }

    fn shown_columns(cx: &App) -> impl Iterator<Item = Column> + use<> {
        let folder = !Self::shows_preview(cx);
        Column::ALL.into_iter().filter(move |&column| folder || column != Column::Folder)
    }

    fn footer(&self, theme: &Theme, language: Language, status: Status, window: &Window) -> impl IntoElement {
        let strings = language.strings();
        let shown = self.results.hits().len();
        let counted = match self.results.listing() {
            Listing::Found(total) => (total > shown).then(|| language.matches(total)),
            Listing::AtLeast(total) => (total > shown).then(|| language.at_least(total)),
            Listing::Contents(total) => (total > shown).then(|| language.items(total)),
            Listing::Recent
            | Listing::Folders
            | Listing::Searching
            | Listing::Actions
            | Listing::ContentOff
            | Listing::ContentShort => None,
        };
        let plain = !status.scanning && self.results.notice().is_none() && self.results.picked_count() == 0;
        let status = match counted.filter(|_| plain) {
            Some(counted) => format!("{counted}, {} {}", strings.shown_first, shown),
            None => self.results.status(status, language),
        };
        status_bar(theme)
            .child(
                faded("status", status.clone(), Tip::new(status), DETAIL_LINE, theme.app.background).flex_1().min_w_0(),
            )
            .when(!self.results.hits().is_empty(), |footer| footer.child(result_hints(strings, theme, window)))
    }
}

fn columns(row: Div) -> Div {
    row.flex().items_center().gap(rem(COLUMN_GAP)).px(rem(INDENT))
}

fn column_cell<E: Styled>(cell: E, column: Column) -> E {
    let share = |mut cell: E, share: f32| {
        cell.style().flex_grow = Some(share);
        cell.flex_basis(rem(0.)).flex_shrink(1.).min_w_0()
    };
    let cell = match column {
        Column::Name => share(cell, NAME_SHARE),
        Column::Folder => share(cell, FOLDER_SHARE),
        Column::Size => cell.flex_none().w(rem(SIZE_COLUMN)),
        Column::Modified => cell.flex_none().w(rem(MODIFIED_COLUMN)),
    };
    let cell = cell.flex().items_center();
    match column.align() {
        Align::Start => cell,
        Align::End => cell.justify_end(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_lands_on_whole_pixels_at_common_scales() {
        crate::ui::assert_whole_pixels(&[
            ICON,
            HEADER,
            HEADER_TOP,
            ROW,
            COLUMN_GAP,
            ICON_GAP,
            SORT_GAP,
            SORT_GLYPH,
            SIZE_COLUMN,
            MODIFIED_COLUMN,
        ]);
    }
}
