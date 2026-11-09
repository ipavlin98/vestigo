use std::ops::Range;

const CYRILLIC_A: u8 = 0xD0;
const CYRILLIC_R: u8 = 0xD1;
const LATIN_SYMBOLS: u8 = 0xC2;
const LATIN_LETTERS: u8 = 0xC3;
const PUNCTUATION: u8 = 0xE2;
const MULTIPLICATION: u8 = 0x97;
const NUMERO: u8 = 0x96;

pub(crate) fn fold(text: &mut [u8]) -> bool {
    let mut exact = true;
    let mut rest = text;
    loop {
        let plain = rest.iter().position(|byte| !byte.is_ascii()).unwrap_or(rest.len());
        let (ascii, tail) = std::mem::take(&mut rest).split_at_mut(plain);
        ascii.make_ascii_lowercase();
        let width = match tail {
            [] => return exact,
            [lead @ CYRILLIC_A, second, ..] => {
                match *second {
                    0x80..=0x8F => (*lead, *second) = (CYRILLIC_R, *second + 0x10),
                    0x90..=0x9F => *second += 0x20,
                    0xA0..=0xAF => (*lead, *second) = (CYRILLIC_R, *second - 0x20),
                    _ => {}
                }
                2
            }
            [CYRILLIC_R, second, ..] => {
                exact &= *second < 0xA0;
                2
            }
            [LATIN_SYMBOLS, _, ..] => 2,
            [LATIN_LETTERS, second, ..] => {
                if matches!(*second, 0x80..=0x9E) && *second != MULTIPLICATION {
                    *second += 0x20;
                }
                2
            }
            [PUNCTUATION, 0x80..=0x83, _, ..] | [PUNCTUATION, 0x84, NUMERO, ..] => 3,
            [_, ..] => {
                exact = false;
                1
            }
        };
        rest = tail.get_mut(width..).unwrap_or_default();
    }
}

pub(crate) fn lowered(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

pub(crate) fn folded(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    if fold(&mut bytes) { String::from_utf8(bytes).unwrap_or_else(|_| lowered(text)) } else { lowered(text) }
}

pub(crate) fn with_origins(text: &str) -> (String, Vec<(usize, usize)>) {
    let mut lower = String::with_capacity(text.len());
    let mut origins = Vec::with_capacity(text.len());
    for (offset, ch) in text.char_indices() {
        for low in ch.to_lowercase() {
            for _ in 0..low.len_utf8() {
                origins.push((offset, offset + ch.len_utf8()));
            }
            lower.push(low);
        }
    }
    (lower, origins)
}

pub(crate) fn origin(origins: &[(usize, usize)], span: Range<usize>) -> Option<Range<usize>> {
    let (from, _) = *origins.get(span.start)?;
    let (_, to) = *origins.get(span.end.checked_sub(1)?)?;
    Some(from..to)
}

pub(crate) fn collapse(text: &str, out: &mut String) {
    out.clear();
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_cyrillic_fold_in_place_like_lowercase() {
        for text in ["Hello WORLD 123", "АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ", "ЁЖИК Ёлка ЂЃЄЅІЇЈЉЊЋЌЍЎЏ", "ÀÉÎÕÜ×ßÞ"]
        {
            let mut bytes = text.as_bytes().to_vec();
            assert!(fold(&mut bytes), "{text}");
            assert_eq!(bytes.len(), text.len(), "{text}");
            assert_eq!(String::from_utf8(bytes).expect("utf-8"), text.to_lowercase());
        }
    }

    #[test]
    fn typographic_marks_stay_exact_in_russian_text() {
        let text = "«ДОГОВОР» № 5 — Аренда, цена 100\u{a0}€";
        let mut bytes = text.as_bytes().to_vec();
        assert!(fold(&mut bytes));
        assert_eq!(String::from_utf8(bytes).expect("utf-8"), text.to_lowercase());
    }

    #[test]
    fn other_scripts_fall_back_to_full_lowercasing() {
        for text in ["ΑΒΓ слово", "İstanbul", "Ѡмега", "\u{212A}elvin"] {
            let mut bytes = text.as_bytes().to_vec();
            assert!(!fold(&mut bytes), "{text}");
            assert_eq!(folded(text), lowered(text));
        }
    }

    #[test]
    fn a_cut_character_at_the_end_is_left_alone() {
        let mut bytes = vec![b'A', CYRILLIC_A];
        assert!(!fold(&mut bytes));
        assert_eq!(bytes, [b'a', CYRILLIC_A]);
    }

    #[test]
    fn origins_lead_back_to_the_original_text() {
        let (lower, origins) = with_origins("İa");
        let at = lower.find('a').expect("a");
        assert_eq!(origin(&origins, at..at + 1), Some(2..3));
        assert_eq!(origin(&origins, 0..0), None);
    }

    #[test]
    fn whitespace_collapses_into_single_spaces() {
        let mut out = String::from("old");
        collapse("  первая\r\n\tвторая   третья\n", &mut out);
        assert_eq!(out, "первая вторая третья");
    }
}
