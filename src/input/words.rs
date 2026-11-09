use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Space,
    Punctuation,
}

impl Kind {
    fn of(grapheme: &str) -> Self {
        match grapheme.chars().next() {
            Some(c) if c.is_alphanumeric() || c == '_' => Self::Word,
            Some(c) if c.is_whitespace() => Self::Space,
            _ => Self::Punctuation,
        }
    }
}

fn is_word(grapheme: &str) -> bool {
    Kind::of(grapheme) == Kind::Word
}

pub(super) fn word_start_before(text: &str, offset: usize) -> usize {
    let Some(before) = text.get(..offset) else { return 0 };
    let mut inside = false;
    for (index, grapheme) in before.grapheme_indices(true).rev() {
        let word = is_word(grapheme);
        if inside && !word {
            return index + grapheme.len();
        }
        inside |= word;
    }
    0
}

pub(super) fn word_start_after(text: &str, offset: usize) -> usize {
    let Some(after) = text.get(offset..) else { return text.len() };
    let mut left = false;
    for (index, grapheme) in after.grapheme_indices(true) {
        let word = is_word(grapheme);
        if left && word {
            return offset + index;
        }
        left |= !word;
    }
    text.len()
}

pub(super) fn word_at(text: &str, offset: usize) -> Range<usize> {
    let (Some(before), Some(after)) = (text.get(..offset), text.get(offset..)) else { return offset..offset };
    let Some(kind) = after.graphemes(true).next().or_else(|| before.graphemes(true).next_back()).map(Kind::of) else {
        return offset..offset;
    };
    let start = before
        .grapheme_indices(true)
        .rev()
        .take_while(|(_, grapheme)| Kind::of(grapheme) == kind)
        .last()
        .map_or(offset, |(index, _)| index);
    let end =
        offset + after.graphemes(true).take_while(|grapheme| Kind::of(grapheme) == kind).map(str::len).sum::<usize>();
    start..end
}
