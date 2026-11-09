use memchr::{memchr, memchr_iter};
use std::ops::Range;

const SEPARATORS: &[u8] = b" -_.([+,";
const ABBREVIATION_BUDGET: u32 = 64;
const MIN_TYPO_CHARS: usize = 4;
const TWO_TYPOS_CHARS: usize = 8;
const MAX_TYPO_CHARS: usize = 32;
const MIN_LETTERS_CHARS: usize = 3;
const DIGIT_BIT: u8 = 26;
const NON_ASCII_BIT: u8 = 27;
const NON_ASCII_SPREAD: u8 = 3;
const OTHER_BIT: u8 = 31;

type Column = [(u8, usize); MAX_TYPO_CHARS + 1];

#[derive(Clone, Copy)]
pub(crate) struct Name<'a> {
    pub(super) lower: &'a str,
    original: &'a [u8],
    pub(super) ascii: bool,
}

impl<'a> Name<'a> {
    pub(crate) fn new(original: &'a [u8], lower: &'a str, ascii: bool) -> Self {
        Self { lower, original, ascii }
    }

    fn prefix(self, end: usize) -> Option<Self> {
        let lower = self.lower.get(..end)?;
        let original = if self.original.len() == self.lower.len() { self.original.get(..end)? } else { self.original };
        Some(Self { lower, original, ascii: self.ascii })
    }

    pub(super) fn starts_word(self, position: usize) -> bool {
        let lower = self.lower.as_bytes();
        let Some(previous) = position.checked_sub(1) else { return true };
        let Some(before) = lower.get(previous) else { return true };
        if SEPARATORS.contains(before) {
            return true;
        }
        let original = self.original;
        original.len() == lower.len()
            && original.get(position).is_some_and(u8::is_ascii_uppercase)
            && original.get(previous).is_some_and(u8::is_ascii_lowercase)
    }
}

pub(super) fn abbreviation(term: &[u8], name: Name, record: &mut dyn FnMut(Range<usize>)) -> bool {
    let lower = name.lower.as_bytes();
    if term.first() != lower.first() {
        return false;
    }
    let Some(name) = name.prefix(stem(lower)) else { return false };
    let mut budget = ABBREVIATION_BUDGET;
    chunks(term, name, 0, &mut budget, record)
}

fn chunks(term: &[u8], name: Name, start: usize, budget: &mut u32, record: &mut dyn FnMut(Range<usize>)) -> bool {
    let lower = name.lower.as_bytes();
    let Some(text) = lower.get(start..) else { return false };
    let common = term.iter().zip(text).take_while(|(wanted, found)| wanted == found).count();
    for length in (1..=common).rev() {
        let Some(remaining) = budget.checked_sub(1) else { return false };
        *budget = remaining;
        let Some(rest) = term.get(length..) else { return false };
        if rest.is_empty() || next_word(rest, name, start + length, budget, record) {
            record(start..start + length);
            return true;
        }
    }
    false
}

fn next_word(term: &[u8], name: Name, from: usize, budget: &mut u32, record: &mut dyn FnMut(Range<usize>)) -> bool {
    let (Some(&first), Some(text)) = (term.first(), name.lower.as_bytes().get(from..)) else { return false };
    memchr_iter(first, text)
        .map(|offset| from + offset)
        .any(|start| name.starts_word(start) && chunks(term, name, start, budget, record))
}

pub(super) fn typo(pattern: &[char], text: &str) -> Option<Range<usize>> {
    let allowed: u8 = match pattern.len() {
        length if !(MIN_TYPO_CHARS..=MAX_TYPO_CHARS).contains(&length) => return None,
        length if length < TWO_TYPOS_CHARS => 1,
        _ => 2,
    };
    let mut before: Column = [(0, 0); MAX_TYPO_CHARS + 1];
    for (row, cell) in before.iter_mut().enumerate() {
        *cell = (row as u8, 0);
    }
    let mut older = before;
    let mut previous_char = None;
    let mut best: Option<(u8, Range<usize>)> = None;
    for (offset, ch) in text.char_indices() {
        let end = offset + ch.len_utf8();
        let mut current: Column = [(0, end); MAX_TYPO_CHARS + 1];
        for row in 1..=pattern.len() {
            let (Some(&wanted), Some(&(diagonal, from)), Some(&(left, left_from)), Some(&(up, up_from))) =
                (pattern.get(row - 1), before.get(row - 1), before.get(row), current.get(row - 1))
            else {
                break;
            };
            let mut cell = (diagonal + u8::from(wanted != ch), from);
            if left + 1 < cell.0 {
                cell = (left + 1, left_from);
            }
            if up + 1 < cell.0 {
                cell = (up + 1, up_from);
            }
            if previous_char == Some(wanted)
                && row >= 2
                && pattern.get(row - 2) == Some(&ch)
                && let Some(&(swapped, swapped_from)) = older.get(row - 2)
                && swapped + 1 < cell.0
            {
                cell = (swapped + 1, swapped_from);
            }
            if let Some(slot) = current.get_mut(row) {
                *slot = cell;
            }
        }
        if let Some(&(cost, from)) = current.get(pattern.len())
            && cost <= allowed
            && best.as_ref().is_none_or(|(lowest, _)| cost < *lowest)
        {
            best = Some((cost, from..end));
        }
        older = before;
        before = current;
        previous_char = Some(ch);
    }
    best.map(|(_, range)| range)
}

pub(super) fn letters(pattern: &str, name: Name, record: &mut dyn FnMut(Range<usize>)) -> Option<u32> {
    if pattern.chars().count() < MIN_LETTERS_CHARS {
        return None;
    }
    let first = pattern.chars().next()?;
    let lower = name.lower.get(..stem(name.lower.as_bytes()))?;
    let mut best: Option<(u32, usize)> = None;
    for (start, _) in lower.match_indices(first).filter(|&(start, _)| name.starts_word(start)) {
        let Some(breaks) = chunks_from(pattern, lower, start, &mut |_| {}) else { break };
        if best.is_none_or(|(fewest, _)| breaks < fewest) {
            best = Some((breaks, start));
        }
    }
    let (breaks, start) = best?;
    chunks_from(pattern, lower, start, record);
    Some(breaks)
}

pub(super) fn in_order(pattern: &[u8], text: &[u8]) -> bool {
    let Some((&first, rest)) = pattern.split_first() else { return true };
    let Some(start) = memchr(first, text) else { return false };
    let mut wanted = rest.iter();
    let Some(mut next) = wanted.next() else { return true };
    for byte in text.get(start + 1..).unwrap_or_default() {
        if byte == next {
            let Some(after) = wanted.next() else { return true };
            next = after;
        }
    }
    false
}

fn chunks_from(pattern: &str, lower: &str, start: usize, record: &mut dyn FnMut(Range<usize>)) -> Option<u32> {
    let mut breaks = 0;
    let mut chunk = start;
    let mut at = start;
    for ch in pattern.chars() {
        let found = at + lower.get(at..)?.find(ch)?;
        if found != at {
            record(chunk..at);
            breaks += 1;
            chunk = found;
        }
        at = found + ch.len_utf8();
    }
    record(chunk..at);
    Some(breaks)
}

pub(crate) fn alphabet(lower: &[u8]) -> u32 {
    lower.iter().fold(0, |mask, &byte| {
        let bit = match byte {
            b'a'..=b'z' => byte - b'a',
            b'0'..=b'9' => DIGIT_BIT,
            0x80.. => NON_ASCII_BIT + (byte & NON_ASCII_SPREAD),
            _ => OTHER_BIT,
        };
        mask | 1 << bit
    })
}

fn stem(lower: &[u8]) -> usize {
    memchr(b'.', lower).filter(|&dot| dot > 0).unwrap_or(lower.len())
}
