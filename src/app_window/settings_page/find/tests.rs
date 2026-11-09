use super::{Finding, Needle, Spot, Step};
use crate::app_window::Section;
use crate::app_window::settings_page::theme_editor::Unfold;

const NO_HINTS: [&str; 0] = [];

fn spot(section: Section) -> Spot {
    Spot { section, needs: Vec::new() }
}

fn finding(spots: Vec<Spot>) -> Finding {
    Finding { needle: Needle::new("x").expect("a word"), spots, current: None, jump: 0 }
}

#[test]
fn a_blank_query_searches_nothing() {
    assert!(Needle::new("").is_none());
    assert!(Needle::new("  \t ").is_none());
}

#[test]
fn every_word_must_appear_in_the_label_or_a_hint_ignoring_case() {
    let needle = Needle::new("ОКНО перетаск").expect("two words");
    assert!(needle.finds("Положение окна", &["Окно перетаскивается за нижнюю строку"]));
    assert!(!needle.finds("Положение окна", &["Двойной клик возвращает его на место"]));
    assert!(Needle::new("тема").expect("a word").finds("ТЕМА", &NO_HINTS));
}

#[test]
fn spans_cover_each_occurrence_in_the_original_text() {
    let needle = Needle::new("ок").expect("a word");
    assert_eq!(needle.spans("Окно, окно"), vec![0..4, 10..14]);
    let overlapping = Needle::new("окн кно").expect("two words");
    assert_eq!(overlapping.spans("Окно"), vec![0..8], "overlapping words merge into one span");
}

#[test]
fn spans_map_back_through_letters_that_grow_when_lowered() {
    let needle = Needle::new("i\u{307}x").expect("a word");
    assert_eq!(needle.spans("a\u{130}x"), vec![1..4]);
}

#[test]
fn landing_in_a_section_picks_its_first_match_and_keeps_one_already_there() {
    let mut found = finding(vec![spot(Section::General), spot(Section::Appearance), spot(Section::Appearance)]);
    found.land(Section::Appearance);
    assert_eq!((found.current, found.jump), (Some(1), 1));
    found.current = Some(2);
    found.land(Section::Appearance);
    assert_eq!((found.current, found.jump), (Some(2), 1), "a match already in the section stays");
    found.land(Section::Hotkeys);
    assert_eq!(found.current, None, "a section without matches has no current one");
}

#[test]
fn steps_wrap_around_across_sections() {
    let mut found = finding(vec![spot(Section::General), spot(Section::Index)]);
    found.current = Some(1);
    assert_eq!(found.step(Step::Next, Section::Index).map(|spot| spot.section), Some(Section::General));
    assert_eq!(found.step(Step::Previous, Section::General).map(|spot| spot.section), Some(Section::Index));
    assert_eq!(found.jump, 2);
}

#[test]
fn a_step_from_a_section_without_matches_goes_to_its_neighbours() {
    let mut found = finding(vec![spot(Section::General), spot(Section::Index)]);
    assert_eq!(found.step(Step::Next, Section::Hotkeys), Some(&spot(Section::Index)));
    found.current = None;
    assert_eq!(found.step(Step::Previous, Section::Hotkeys), Some(&spot(Section::General)));
    found.current = None;
    assert_eq!(found.step(Step::Next, Section::About), Some(&spot(Section::General)));
}

#[test]
fn stepping_without_matches_changes_nothing() {
    let mut found = finding(Vec::new());
    assert_eq!(found.step(Step::Next, Section::General), None);
    assert_eq!((found.current, found.jump), (None, 0));
}

#[test]
fn the_current_match_counts_only_matches_on_screen() {
    let folded = Spot { section: Section::Appearance, needs: vec![Unfold::Advanced] };
    let mut found = finding(vec![spot(Section::General), folded.clone(), spot(Section::Appearance)]);
    found.current = Some(2);
    assert_eq!(
        found.on_screen(Section::Appearance, |needs| needs.is_empty()),
        Some(0),
        "the folded match isn't on screen"
    );
    assert_eq!(found.on_screen(Section::Appearance, |_| true), Some(1));
    found.current = Some(1);
    assert_eq!(
        found.on_screen(Section::Appearance, |needs| needs.is_empty()),
        None,
        "a hidden current match is not on screen"
    );
    assert_eq!(found.on_screen(Section::General, |_| true), None);
}
