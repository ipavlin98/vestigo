use super::*;

fn at(text: &'static str) -> Snapshot {
    (text.into(), text.len()..text.len())
}

fn text(snapshot: Option<Snapshot>) -> Option<String> {
    snapshot.map(|(text, _)| text.to_string())
}

#[test]
fn typed_run_undoes_in_one_step() {
    let mut history = History::default();
    history.record(at(""), true);
    history.record(at("a"), true);
    history.record(at("ab"), true);
    assert_eq!(text(history.undo(at("abc"))), Some(String::new()));
    assert_eq!(text(history.undo(at(""))), None);
}

#[test]
fn moving_the_caret_splits_a_typed_run() {
    let mut history = History::default();
    history.record(at(""), true);
    history.record(at("a"), true);
    history.pause();
    history.record(at("ab"), true);
    assert_eq!(text(history.undo(at("abc"))), Some("ab".into()));
    assert_eq!(text(history.undo(at("ab"))), Some(String::new()));
}

#[test]
fn redo_brings_back_what_undo_took() {
    let mut history = History::default();
    history.record(at("one"), false);
    history.record(at("two"), false);
    assert_eq!(text(history.undo(at("three"))), Some("two".into()));
    assert_eq!(text(history.undo(at("two"))), Some("one".into()));
    assert_eq!(text(history.redo(at("one"))), Some("two".into()));
    assert_eq!(text(history.redo(at("two"))), Some("three".into()));
    assert_eq!(text(history.redo(at("three"))), None);
    assert_eq!(text(history.undo(at("three"))), Some("two".into()));
}

#[test]
fn a_new_edit_drops_the_redo_chain() {
    let mut history = History::default();
    history.record(at("one"), false);
    assert_eq!(text(history.undo(at("two"))), Some("one".into()));
    history.record(at("one"), true);
    assert_eq!(text(history.redo(at("onex"))), None);
}

#[test]
fn typing_after_undo_starts_a_new_step() {
    let mut history = History::default();
    history.record(at(""), true);
    assert_eq!(text(history.undo(at("a"))), Some(String::new()));
    history.record(at(""), true);
    history.record(at("b"), true);
    assert_eq!(text(history.undo(at("bc"))), Some(String::new()));
}

#[test]
fn history_keeps_only_the_latest_steps() {
    let mut history = History::default();
    for step in 0..=DEPTH {
        history.record((step.to_string().into(), 0..0), false);
    }
    let mut undone = 0;
    let mut last = None;
    while let Some(snapshot) = history.undo(at("")) {
        undone += 1;
        last = Some(snapshot.0.to_string());
    }
    assert_eq!(undone, DEPTH);
    assert_eq!(last, Some("1".into()));
}
