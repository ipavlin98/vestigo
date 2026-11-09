use super::find::{Needle, Spot};
use super::theme_editor::Unfold;
use super::{FIELD_PADDING, FIELD_ROW, FOLD_END, ROW, ROW_GAP, ROW_PADDING, SECTION_TITLE, Section, column};
use crate::assets;
use crate::theme::{Theme, rem};
use crate::ui::{DETAIL_LINE, DETAIL_SIZE, DISCLOSURE_GROUP, Edge, GLYPH, INSET, LABEL_GAP, disclosure, divider};
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, HighlightStyle, InteractiveText, SharedString, Stateful,
    StyledText, Window, div, prelude::*, svg,
};

const DIMMED: f32 = 0.4;
const NO_HINTS: [&str; 0] = [];

pub(super) type Toggle = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
pub(super) type Grip = Box<dyn FnOnce(Stateful<Div>) -> Stateful<Div>>;
pub(super) type Follow = Box<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy)]
pub(super) enum Heading {
    Plain,
    Title(&'static str),
    Note(&'static str),
    Linked { note: &'static str, link: &'static str },
}

pub(super) struct Block {
    heading: Heading,
    rows: Vec<Row>,
    needs: Vec<Unfold>,
    follow: Option<Follow>,
}

impl Block {
    pub(super) fn follow(mut self, follow: Follow) -> Self {
        self.follow = Some(follow);
        self
    }

    pub(super) fn under(mut self, needs: &[Unfold]) -> Self {
        self.needs = needs.to_vec();
        self
    }

    pub(super) fn spots(&self, section: Section, needle: &Needle, spots: &mut Vec<Spot>) {
        if let Heading::Title(title) = self.heading
            && needle.finds(title, &NO_HINTS)
        {
            spots.push(Spot { section, needs: self.needs.clone() });
        }
        for row in &self.rows {
            row.spots(section, needle, &self.needs, spots);
        }
    }
}

pub(super) fn section(heading: Heading, rows: Vec<Row>) -> Block {
    Block { heading, rows, needs: Vec::new(), follow: None }
}

pub(super) struct Row {
    label: SharedString,
    hints: Vec<SharedString>,
    id: Option<ElementId>,
    grip: Option<Grip>,
    body: Body,
}

enum Body {
    Setting(Vec<AnyElement>),
    Field(Vec<AnyElement>),
    Fold(Box<Folding>),
    Plain(AnyElement),
}

pub(super) struct Folding {
    pub(super) id: ElementId,
    pub(super) open: bool,
    pub(super) controls: Vec<AnyElement>,
    pub(super) toggle: Toggle,
    pub(super) inner: Vec<Row>,
    pub(super) unfold: Option<Unfold>,
}

impl Row {
    pub(super) fn plain(element: impl IntoElement) -> Self {
        Self {
            label: SharedString::default(),
            hints: Vec::new(),
            id: None,
            grip: None,
            body: Body::Plain(element.into_any_element()),
        }
    }

    pub(super) fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub(super) fn grip(mut self, grip: Grip) -> Self {
        self.grip = Some(grip);
        self
    }

    fn found_by(&self, needle: &Needle) -> bool {
        match self.body {
            Body::Setting(_) | Body::Field(_) | Body::Fold(_) => needle.finds(&self.label, &self.hints),
            Body::Plain(_) => false,
        }
    }

    fn spots(&self, section: Section, needle: &Needle, needs: &[Unfold], spots: &mut Vec<Spot>) {
        if self.found_by(needle) {
            spots.push(Spot { section, needs: needs.to_vec() });
        }
        if let Body::Fold(folding) = &self.body
            && folding.open
            && let Some(unfold) = folding.unfold
        {
            let needs: Vec<Unfold> = needs.iter().copied().chain([unfold]).collect();
            for row in &folding.inner {
                row.spots(section, needle, &needs, spots);
            }
        }
    }

    fn render(self, needle: Option<Found>, theme: &Theme) -> (AnyElement, Option<Inner>) {
        let Self { label, hints, id, grip, body } = self;
        let (element, inner) = match body {
            Body::Setting(controls) => match grip {
                Some(grip) => {
                    let lead = Some(handle(theme));
                    let id = id.clone().unwrap_or_else(|| ElementId::Name(format!("grip-{label}").into()));
                    let line = div().id(id).group(DISCLOSURE_GROUP).relative().flex().flex_col();
                    (
                        grip(line.child(setting_line(label, hints, lead, controls, needle, theme))).into_any_element(),
                        None,
                    )
                }
                None => (setting_line(label, hints, None, controls, needle, theme), None),
            },
            Body::Field(controls) => (field_line(label, controls, needle, theme), None),
            Body::Fold(folding) => {
                let Folding { id, open, controls, toggle, inner, unfold } = *folding;
                let controls = controls.into_iter().chain([disclosure(open, theme).into_any_element()]).collect();
                let lead = grip.is_some().then(|| handle(theme));
                let header = div()
                    .id(id)
                    .group(DISCLOSURE_GROUP)
                    .relative()
                    .flex()
                    .flex_col()
                    .cursor_pointer()
                    .block_mouse_except_scroll()
                    .on_click(move |event, window, cx| toggle(event, window, cx))
                    .child(setting_line(label, hints, lead, controls, needle, theme));
                let header = match grip {
                    Some(grip) => grip(header),
                    None => header,
                };
                (header.into_any_element(), open.then_some(Inner { rows: inner, searched: unfold.is_some() }))
            }
            Body::Plain(element) => (element, None),
        };
        let element = match id {
            Some(id) => div().id(id).child(element).into_any_element(),
            None => element,
        };
        (element, inner)
    }
}

#[derive(Clone, Copy)]
struct Found<'a> {
    needle: &'a Needle,
    current: bool,
}

struct Inner {
    rows: Vec<Row>,
    searched: bool,
}

pub(super) fn setting(label: impl Into<SharedString>, hint: &'static str, control: impl IntoElement) -> Row {
    setting_with(label, vec![hint.into()], control)
}

pub(super) fn setting_with(label: impl Into<SharedString>, hints: Vec<SharedString>, control: impl IntoElement) -> Row {
    setting_row(label, hints, vec![control.into_any_element()])
}

pub(super) fn setting_row(label: impl Into<SharedString>, hints: Vec<SharedString>, controls: Vec<AnyElement>) -> Row {
    Row { label: label.into(), hints: shown(hints), id: None, grip: None, body: Body::Setting(controls) }
}

pub(super) fn fold_row((label, hint): (SharedString, SharedString), folding: Folding) -> Row {
    Row { label, hints: shown(vec![hint]), id: None, grip: None, body: Body::Fold(Box::new(folding)) }
}

pub(super) fn field_row(label: &'static str, control: impl IntoElement) -> Row {
    field_row_with(label.into(), vec![control.into_any_element()])
}

pub(super) fn field_row_with(label: SharedString, controls: Vec<AnyElement>) -> Row {
    Row { label, hints: Vec::new(), id: None, grip: None, body: Body::Field(controls) }
}

fn shown(hints: Vec<SharedString>) -> Vec<SharedString> {
    hints.into_iter().filter(|hint| !hint.is_empty()).collect()
}

fn marked(text: SharedString, needle: Option<Found>, theme: &Theme) -> StyledText {
    let colors = theme.settings;
    let spans = needle.map(|found| found.needle.spans(&text)).unwrap_or_default();
    let fill = match needle {
        Some(Found { current: true, .. }) => colors.found,
        Some(Found { current: false, .. }) | None => colors.matched,
    };
    let style = HighlightStyle { background_color: Some(fill), ..Default::default() };
    StyledText::new(text).with_highlights(spans.into_iter().map(|span| (span, style)))
}

fn handle(theme: &Theme) -> AnyElement {
    let colors = theme.icon_button;
    svg()
        .path(assets::GRIP)
        .size(rem(GLYPH))
        .flex_none()
        .text_color(colors.glyph)
        .group_hover(DISCLOSURE_GROUP, |style| style.text_color(colors.hover_glyph))
        .into_any_element()
}

fn setting_line(
    label: SharedString,
    hints: Vec<SharedString>,
    lead: Option<AnyElement>,
    controls: Vec<AnyElement>,
    needle: Option<Found>,
    theme: &Theme,
) -> AnyElement {
    let colors = theme.settings;
    let hints = hints.into_iter().map(|hint| {
        div()
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
            .text_color(colors.hint)
            .child(marked(hint, needle, theme))
    });
    let text = div()
        .flex_1()
        .w_0()
        .min_w_0()
        .flex()
        .flex_col()
        .text_color(colors.label)
        .child(marked(label, needle, theme))
        .children(hints);
    let text = match lead {
        Some(lead) => div().flex_1().w_0().min_w_0().flex().items_center().gap(rem(LABEL_GAP)).child(lead).child(text),
        None => text,
    };
    div()
        .relative()
        .w_full()
        .min_h(rem(ROW))
        .py(rem(ROW_PADDING))
        .flex()
        .items_center()
        .gap(rem(ROW_GAP))
        .child(text)
        .children(controls)
        .into_any_element()
}

fn field_line(label: SharedString, controls: Vec<AnyElement>, needle: Option<Found>, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .min_h(rem(FIELD_ROW))
        .py(rem(FIELD_PADDING))
        .flex()
        .items_center()
        .gap(rem(ROW_GAP))
        .child(
            div()
                .flex_1()
                .w_0()
                .min_w_0()
                .text_size(rem(DETAIL_SIZE))
                .line_height(rem(DETAIL_LINE))
                .text_color(theme.settings.hint)
                .child(marked(label, needle, theme)),
        )
        .children(controls)
        .into_any_element()
}

fn band(heading: Heading, found: Option<Found>, follow: Option<Follow>, theme: &Theme) -> Option<AnyElement> {
    let colors = theme.settings;
    let band = || {
        div()
            .min_h(rem(SECTION_TITLE))
            .flex()
            .items_end()
            .pb(rem(INSET))
            .text_size(rem(DETAIL_SIZE))
            .line_height(rem(DETAIL_LINE))
    };
    match heading {
        Heading::Plain => None,
        Heading::Title(title) => Some(
            band()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(colors.section)
                .child(marked(title.into(), found, theme))
                .into_any_element(),
        ),
        Heading::Note(note) => Some(band().text_color(colors.hint).child(note).into_any_element()),
        Heading::Linked { note, link } => {
            let start = note.find(link).unwrap_or(note.len());
            let range = start..start + link.len().min(note.len() - start);
            let style = HighlightStyle { color: Some(colors.label), ..HighlightStyle::default() };
            let text = StyledText::new(note).with_highlights([(range.clone(), style)]);
            let text = InteractiveText::new("heading-link", text).on_click(vec![range], move |_, window, cx| {
                if let Some(follow) = follow.as_ref() {
                    follow(window, cx);
                }
            });
            Some(band().text_color(colors.hint).child(text).into_any_element())
        }
    }
}

pub(super) struct Laid {
    pub(super) title: StyledText,
    pub(super) lines: Vec<AnyElement>,
    pub(super) target: Option<usize>,
}

pub(super) fn lay_out(
    title: &'static str,
    blocks: Vec<Block>,
    needle: Option<&Needle>,
    on_screen: Option<usize>,
    theme: &Theme,
) -> Laid {
    let empty = Laid { title: StyledText::new(title), lines: Vec::new(), target: None };
    let mut layout = Layout { needle, on_screen, found: 0, theme, laid: empty };
    let titled = layout.hit(|needle| needle.finds(title, &NO_HINTS));
    if titled.is_some_and(|found| found.current) {
        layout.laid.target = Some(0);
    }
    layout.laid.title = marked(title.into(), titled, theme);
    for block in blocks {
        let plain = matches!(block.heading, Heading::Plain);
        let heading = match block.heading {
            Heading::Title(text) => layout.hit(|needle| needle.finds(text, &NO_HINTS)),
            Heading::Plain | Heading::Note(_) | Heading::Linked { .. } => None,
        };
        if heading.is_some_and(|found| found.current) {
            layout.laid.target = Some(layout.laid.lines.len());
        }
        if let Some(band) = band(block.heading, heading, block.follow, theme) {
            let dimmed = needle.is_some() && heading.is_none();
            layout.laid.lines.push(column().when(dimmed, |line| line.opacity(DIMMED)).child(band).into_any_element());
        }
        for (position, row) in block.rows.into_iter().enumerate() {
            layout.row(row, Place { lined: position > 0, padded: plain && position == 0, searched: true });
        }
    }
    layout.laid
}

#[derive(Clone, Copy)]
struct Place {
    lined: bool,
    padded: bool,
    searched: bool,
}

struct Layout<'a> {
    needle: Option<&'a Needle>,
    on_screen: Option<usize>,
    found: usize,
    theme: &'a Theme,
    laid: Laid,
}

impl<'a> Layout<'a> {
    fn hit(&mut self, matches: impl FnOnce(&Needle) -> bool) -> Option<Found<'a>> {
        let needle = self.needle.filter(|needle| matches(needle))?;
        self.found += 1;
        Some(Found { needle, current: self.on_screen == Some(self.found - 1) })
    }

    fn row(&mut self, row: Row, place: Place) {
        let found = self.hit(|needle| place.searched && row.found_by(needle));
        if found.is_some_and(|found| found.current) {
            self.laid.target = Some(self.laid.lines.len());
        }
        let dimmed = self.needle.is_some() && found.is_none();
        let (element, inner) = row.render(found, self.theme);
        let divider = place.lined.then(|| divider(Edge::Top, self.theme.settings.divider));
        let line = column()
            .relative()
            .when(dimmed, |line| line.opacity(DIMMED))
            .when(place.padded, |line| line.pt(rem(INSET)))
            .child(element)
            .children(divider);
        self.laid.lines.push(line.into_any_element());
        if let Some(Inner { rows, searched }) = inner {
            for row in rows {
                self.row(row, Place { lined: false, padded: false, searched: place.searched && searched });
            }
            self.laid.lines.push(column().flex_none().h(rem(FOLD_END)).into_any_element());
        }
    }
}
