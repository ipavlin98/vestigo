use super::*;

const PATH: &str = r"D:\work\src";

#[test]
fn word_left_stops_at_each_folder_of_a_path() {
    assert_eq!(word_start_before(PATH, PATH.len()), 8);
    assert_eq!(word_start_before(PATH, 8), 3);
    assert_eq!(word_start_before(PATH, 3), 0);
}

#[test]
fn word_left_skips_trailing_separators_and_spaces() {
    let text = r"report \work\ ";
    assert_eq!(word_start_before(text, text.len()), 8);
}

#[test]
fn word_right_lands_on_the_next_word_start() {
    assert_eq!(word_start_after(PATH, 0), 3);
    assert_eq!(word_start_after(PATH, 3), 8);
    assert_eq!(word_start_after(PATH, 8), PATH.len());
}

#[test]
fn extension_is_its_own_word() {
    let text = "report.pdf";
    assert_eq!(word_start_before(text, text.len()), 7);
    assert_eq!(word_start_after(text, 0), 7);
}

#[test]
fn cyrillic_words_keep_byte_boundaries() {
    let text = "отчёт за_май.docx";
    let docx = text.len() - "docx".len();
    assert_eq!(word_start_before(text, text.len()), docx);
    assert_eq!(word_start_before(text, docx), "отчёт ".len());
    assert_eq!(word_start_after(text, 0), "отчёт ".len());
}

#[test]
fn combining_marks_stay_inside_their_word() {
    let text = "и\u{306}од file";
    assert_eq!(word_start_before(text, "и\u{306}од".len()), 0);
    assert_eq!(word_start_after(text, 0), "и\u{306}од ".len());
}

#[test]
fn blank_and_out_of_range_offsets_are_safe() {
    assert_eq!(word_start_before("", 0), 0);
    assert_eq!(word_start_after("", 0), 0);
    assert_eq!(word_start_before("   ", 3), 0);
    assert_eq!(word_start_after("   ", 0), 3);
    assert_eq!(word_start_before("ab", 5), 0);
    assert_eq!(word_start_after("ab", 5), 2);
    assert_eq!(word_at("", 0), 0..0);
}

#[test]
fn double_click_selects_the_word_under_the_pointer() {
    assert_eq!(word_at(PATH, 5), 3..7);
    assert_eq!(word_at(PATH, 3), 3..7);
    assert_eq!(word_at("отчёт за", 2), 0.."отчёт".len());
}

#[test]
fn double_click_on_a_separator_selects_the_separator_run() {
    assert_eq!(word_at("a  b", 1), 1..3);
    assert_eq!(word_at(r"a\\b", 2), 1..3);
}

#[test]
fn double_click_past_the_end_selects_the_last_word() {
    assert_eq!(word_at(PATH, PATH.len()), 8..PATH.len());
}
