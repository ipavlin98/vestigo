use super::docs::{Doc, Docs, Plan};
use super::query::{Probe, Sifting, sift};
use super::store::{BLOCK_TEXT, OVERLAP, spans};
use super::worker::{Extractor, Job, Step, write_frame};
use super::*;
use crate::index::volume::{Builder, Change, Meta, ROOT, Volume};
use crate::index::{Boosts, Counts, Facets, Focus, Results, Token};
use crate::search::{Clock, Grammar, Listed, Query};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const WHEN: u32 = 1_700_000_000;
const NOW: Clock = Clock { now: 1_700_100_000, offset: 0 };
const WAIT: Duration = Duration::from_secs(20);

static FOLDERS: AtomicU64 = AtomicU64::new(0);

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        let number = FOLDERS.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("vestigo-content-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp folder");
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn candidate(path: &str) -> Candidate {
    Candidate { path: path.to_owned(), size: 100, modified: WHEN }
}

fn extracted(path: &str, text: &str) -> Extracted {
    Extracted { candidate: candidate(path), replaces: None, outcome: Outcome::Text, text: text.to_owned() }
}

fn opened(folder: &Path) -> Index {
    let library = Library::open_or_create(folder, &|| true).expect("opened").expect("not stopped");
    let index = Index::default();
    *index.content.library.write() = Some(library);
    index.content.signal.lock().run = Some(1);
    index
}

fn ingest(index: &Index, batch: Vec<Extracted>) {
    library::ingest(&index.content.library, batch).expect("ingested");
}

fn parse(text: &str) -> Query {
    Query::parse(text, &Grammar::new(&[], NOW)).expect("a query")
}

fn results(index: &Index, text: &str) -> Results {
    index.search(&parse(text), &Boosts::default(), &Focus::Anything, 50)
}

fn found(index: &Index, text: &str) -> Vec<String> {
    let mut paths: Vec<String> = results(index, text).hits.into_iter().map(|hit| hit.path).collect();
    paths.sort();
    paths
}

fn filler(bytes: usize) -> String {
    let words = ["лес ", "река ", "поле ", "дом "];
    let mut text = String::with_capacity(bytes + 8);
    let mut position = 0;
    while text.len() < bytes {
        text.push_str(words.get(position % words.len()).copied().unwrap_or_default());
        position += 1;
    }
    text
}

#[test]
fn a_substring_is_found_in_the_middle_of_a_word() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "предоставление услуг"), extracted("C:\\b.txt", "другой текст")]);
    assert_eq!(found(&index, "full:доставлен"), ["C:\\a.txt"]);
    assert!(found(&index, "full:нетакого").is_empty());
}

#[test]
fn case_does_not_matter_in_cyrillic_and_latin() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "ДОГОВОР Аренды Quarterly REPORT Ёлка")]);
    for query in ["full:договор", "full:аРЕНДЫ", "full:quarterly", "full:report", "full:ёлка"] {
        assert_eq!(found(&index, query), ["C:\\a.txt"], "{query}");
    }
}

#[test]
fn other_scripts_are_found_through_full_lowercasing() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "Παράδειγμα ΚΕΙΜΕΝΟΥ и Договор")]);
    assert_eq!(found(&index, "full:κειμενου"), ["C:\\a.txt"]);
    assert_eq!(found(&index, "full:договор"), ["C:\\a.txt"]);
}

#[test]
fn a_phrase_on_the_seam_of_two_blocks_is_found() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let text = format!("{}договор аренды {}", filler(BLOCK_TEXT - 8), filler(BLOCK_TEXT));
    assert!(spans(&text).count() >= 2);
    ingest(&index, vec![extracted("C:\\a.txt", &text)]);
    assert_eq!(found(&index, "full:\"договор аренды\""), ["C:\\a.txt"]);
}

#[test]
fn a_phrase_longer_than_the_overlap_is_found_across_the_seam() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let phrase = (0..OVERLAP).map(|number| format!("w{number} ")).collect::<String>();
    let phrase = phrase.trim_end();
    assert!(phrase.len() > OVERLAP);
    let text = format!("{}{phrase} {}", filler(BLOCK_TEXT - 100), filler(BLOCK_TEXT));
    ingest(&index, vec![extracted("C:\\a.txt", &text)]);
    assert_eq!(found(&index, &format!("full:\"{phrase}\"")), ["C:\\a.txt"]);
    assert!(found(&index, &format!("full:\"{phrase}x\"")).is_empty());
}

#[test]
fn words_may_lie_in_different_blocks_of_one_document() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let text = format!("начало {}{}конец", filler(2 * BLOCK_TEXT), filler(1));
    ingest(&index, vec![extracted("C:\\a.txt", &text), extracted("C:\\b.txt", "только начало")]);
    assert_eq!(found(&index, "full:начало конец"), ["C:\\a.txt"]);
    assert_eq!(found(&index, "full:начало"), ["C:\\a.txt", "C:\\b.txt"]);
}

#[test]
fn an_excluded_word_in_another_block_drops_the_document() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let far = format!("договор {}черновик", filler(2 * BLOCK_TEXT));
    ingest(&index, vec![extracted("C:\\a.txt", &far), extracted("C:\\b.txt", "договор подписан")]);
    assert_eq!(found(&index, "full:договор -черновик"), ["C:\\b.txt"]);
}

#[test]
fn short_words_are_only_checked() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "договор на год"), extracted("C:\\b.txt", "договор навсегда")]);
    assert_eq!(found(&index, "full:договор год"), ["C:\\a.txt"]);
}

#[test]
fn only_short_words_ask_for_more() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    assert_eq!(results(&index, "full:да").listed, Listed::ContentShort);
    assert_eq!(results(&index, "full:").listed, Listed::ContentShort);
    assert_eq!(results(&index, "full:-договор").listed, Listed::ContentShort);
}

#[test]
fn turned_off_search_says_so() {
    let index = Index::default();
    assert_eq!(results(&index, "full:договор").listed, Listed::ContentOff);
}

#[test]
fn names_filters_and_places_narrow_the_files() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let mut old = extracted("C:\\work\\docs\\старый отчёт.txt", "договор аренды");
    old.candidate.modified = 1_500_000_000;
    let batch = vec![
        extracted("C:\\work\\docs\\отчёт.pdf", "договор аренды"),
        extracted("C:\\work\\docs\\письмо.txt", "договор аренды"),
        extracted("C:\\home\\отчёт.txt", "договор аренды"),
        old,
    ];
    ingest(&index, batch);
    let mut volume = Volume::new('C');
    let work = volume.insert(ROOT, "work", true);
    volume.insert(work, "docs", true);
    index.volumes.write().push(Arc::new(RwLock::new(volume)));
    assert_eq!(
        found(&index, "отчёт full:аренды"),
        ["C:\\home\\отчёт.txt", "C:\\work\\docs\\отчёт.pdf", "C:\\work\\docs\\старый отчёт.txt"]
    );
    assert_eq!(found(&index, "отчёт full:аренды ext:pdf"), ["C:\\work\\docs\\отчёт.pdf"]);
    assert_eq!(
        found(&index, "full:аренды date:>2020"),
        ["C:\\home\\отчёт.txt", "C:\\work\\docs\\отчёт.pdf", "C:\\work\\docs\\письмо.txt",]
    );
    assert_eq!(found(&index, "full:аренды C:\\work\\docs date:<2020"), ["C:\\work\\docs\\старый отчёт.txt"]);
    assert!(found(&index, "full:аренды C:\\нет").is_empty());
}

#[test]
fn a_snippet_shows_the_match_with_its_highlights() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", &format!("{}Договор Аренды помещения", filler(400)))]);
    let results = results(&index, "full:аренды");
    let snippet = results.hits.first().and_then(|hit| hit.snippet.clone()).expect("a snippet");
    assert!(snippet.text.starts_with('…'));
    let range = snippet.ranges.first().cloned().expect("a highlight");
    assert_eq!(snippet.text.get(range), Some("Аренды"));
}

#[test]
fn a_deleted_document_is_gone_and_a_changed_one_is_found_by_its_new_text() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "старая редакция"), extracted("C:\\b.txt", "ненужный файл")]);
    let mut changed = extracted("C:\\a.txt", "новая редакция");
    changed.replaces = Some(0);
    ingest(&index, vec![changed]);
    index.content.library.write().as_mut().expect("library").docs.retire(1);
    assert!(found(&index, "full:старая").is_empty());
    assert_eq!(found(&index, "full:новая"), ["C:\\a.txt"]);
    assert!(found(&index, "full:ненужный").is_empty());
}

#[test]
fn compaction_keeps_what_search_finds_and_drops_the_waste() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let big = |seed: u32| (0..20_000u32).map(|n| format!("{} ", n.wrapping_mul(seed))).collect::<String>();
    ingest(
        &index,
        vec![extracted("C:\\a.txt", &format!("живой документ {}", big(7))), extracted("C:\\b.txt", &big(11))],
    );
    let before = std::fs::metadata(folder.0.join("blocks.bin")).expect("store").len();
    index.content.library.write().as_mut().expect("library").docs.retire(1);
    library::compact(&index.content.library, &|| true).expect("compacted");
    assert!(std::fs::metadata(folder.0.join("blocks.bin")).expect("store").len() < before);
    assert_eq!(found(&index, "full:живой"), ["C:\\a.txt"]);
    assert_eq!(index.content.library.read().as_ref().expect("library").docs.waste, 0);
    ingest(&index, vec![extracted("C:\\c.txt", "после уплотнения")]);
    assert_eq!(found(&index, "full:уплотнения"), ["C:\\c.txt"]);
    drop(index);
    let index = opened(&folder.0);
    assert_eq!(found(&index, "full:живой"), ["C:\\a.txt"]);
}

#[test]
fn saving_and_loading_keeps_every_search() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "сохранённый текст")]);
    library::save(&index.content.library).expect("saved");
    ingest(&index, vec![extracted("C:\\b.txt", "дописанный текст")]);
    library::save(&index.content.library).expect("saved");
    drop(index);
    let index = opened(&folder.0);
    assert_eq!(found(&index, "full:текст"), ["C:\\a.txt", "C:\\b.txt"]);
    ingest(&index, vec![extracted("C:\\c.txt", "третий текст")]);
    assert_eq!(found(&index, "full:текст"), ["C:\\a.txt", "C:\\b.txt", "C:\\c.txt"]);
}

#[test]
fn spilled_runs_are_searched_and_merged() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    for number in 0..3 {
        ingest(&index, vec![extracted(&format!("C:\\{number}.txt"), &format!("общий текст номер{number}"))]);
        let run = index.content.library.read().as_ref().expect("library").grams.spilled().expect("spilled");
        index.content.library.write().as_mut().expect("library").grams.attach_pending(run);
    }
    ingest(&index, vec![extracted("C:\\3.txt", "общий текст номер3")]);
    assert_eq!(found(&index, "full:общий").len(), 4);
    library::settle(&index.content.library, true).expect("settled");
    let grams_runs = index.content.library.read().as_ref().expect("library").grams.runs();
    assert_eq!(grams_runs, 0);
    assert_eq!(found(&index, "full:общий").len(), 4);
    assert_eq!(found(&index, "full:номер1"), ["C:\\1.txt"]);
}

#[test]
fn text_written_after_the_last_save_is_cut_off() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "сохранённый текст")]);
    library::save(&index.content.library).expect("saved");
    ingest(&index, vec![extracted("C:\\b.txt", "пропавший текст")]);
    let saved = index.content.library.read().as_ref().expect("library").docs.blocks.len();
    assert_eq!(saved, 2);
    drop(index);
    let index = opened(&folder.0);
    assert_eq!(found(&index, "full:текст"), ["C:\\a.txt"]);
    ingest(&index, vec![extracted("C:\\c.txt", "новый текст")]);
    assert_eq!(found(&index, "full:текст"), ["C:\\a.txt", "C:\\c.txt"]);
}

#[test]
fn a_lost_gram_index_is_rebuilt_from_the_stored_text() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "восстановленный текст")]);
    library::settle(&index.content.library, true).expect("settled");
    library::save(&index.content.library).expect("saved");
    drop(index);
    std::fs::remove_file(folder.0.join("grams.bin")).expect("removed");
    let index = opened(&folder.0);
    assert_eq!(found(&index, "full:восстановленный"), ["C:\\a.txt"]);
}

#[test]
fn a_foreign_or_cut_document_table_is_rejected() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    ingest(&index, vec![extracted("C:\\a.txt", "текст")]);
    library::save(&index.content.library).expect("saved");
    drop(index);
    let path = folder.0.join("docs.bin");
    let bytes = std::fs::read(&path).expect("table");
    assert!(Docs::load(&path).is_ok());
    std::fs::write(&path, bytes.get(..bytes.len() - 3).expect("cut")).expect("written");
    assert!(Docs::load(&path).is_err());
    std::fs::write(&path, b"somebody else's file with a long enough header to be read at all, sure").expect("written");
    assert!(Docs::load(&path).is_err());
    let index = opened(&folder.0);
    assert!(found(&index, "full:текст").is_empty(), "a damaged cache starts over");
}

fn doc(path: &str, size: u64, modified: u32) -> Doc {
    Doc { path: path.to_owned(), size, modified, outcome: Outcome::Text, blocks: 0..0 }
}

fn table(rows: Vec<Doc>) -> Docs {
    Docs { docs: rows, ..Docs::default() }
}

fn seen(path: &str, size: u64, modified: u32) -> Candidate {
    Candidate { path: path.to_owned(), size, modified }
}

#[test]
fn reconciling_finds_what_was_added_changed_and_removed() {
    let docs = table(vec![doc("C:\\same.txt", 1, 1), doc("C:\\changed.txt", 2, 2), doc("C:\\gone.txt", 3, 3)]);
    let plan =
        reconcile(&docs, vec![seen("C:\\same.txt", 1, 1), seen("C:\\changed.txt", 2, 9), seen("C:\\new.txt", 4, 4)]);
    assert_eq!(plan.add, [seen("C:\\new.txt", 4, 4)]);
    assert_eq!(plan.update, [(1, seen("C:\\changed.txt", 2, 9))]);
    assert_eq!(plan.remove, [2]);
    assert!(plan.moved.is_empty());
}

#[test]
fn a_moved_folder_changes_paths_without_reading_again() {
    let docs = table(vec![doc("C:\\old\\a.pdf", 5, 5), doc("C:\\old\\b.pdf", 5, 5)]);
    let mut plan = reconcile(&docs, vec![seen("C:\\new\\b.pdf", 5, 5), seen("C:\\new\\a.pdf", 5, 5)]);
    plan.moved.sort();
    assert_eq!(plan.moved, [(0, "C:\\new\\a.pdf".to_owned()), (1, "C:\\new\\b.pdf".to_owned())]);
    assert!(plan.add.is_empty() && plan.remove.is_empty());
}

#[test]
fn a_same_sized_file_with_another_name_is_not_a_move() {
    let docs = table(vec![doc("C:\\old.pdf", 5, 5)]);
    let plan = reconcile(&docs, vec![seen("C:\\new.pdf", 5, 5)]);
    assert_eq!(plan.add, [seen("C:\\new.pdf", 5, 5)]);
    assert_eq!(plan.remove, [0]);
}

#[test]
fn an_applied_plan_leaves_nothing_to_do() {
    let mut docs = table(vec![doc("C:\\old\\a.pdf", 5, 5), doc("C:\\gone.pdf", 6, 6)]);
    let candidates = vec![seen("C:\\new\\a.pdf", 5, 5)];
    let plan = reconcile(&docs, candidates.clone());
    docs.apply(&plan);
    docs.apply(&plan);
    assert_eq!(reconcile(&docs, candidates), Plan::default());
}

#[test]
fn the_best_ranked_documents_come_first_and_many_matches_are_counted_as_a_minimum() {
    let folder = Folder::new();
    let index = opened(&folder.0);
    let batch: Vec<Extracted> =
        (0..600).map(|number| extracted(&format!("C:\\{number}.txt"), "общее слово во всех файлах")).collect();
    ingest(&index, batch);
    let content = parse("full:слово");
    let content = content.content().expect("content");
    let guard = index.content.library.read();
    let library = guard.as_ref().expect("library");
    let (probe, docs) = Probe::new(library, content).expect("probe");
    assert_eq!(docs.len(), 600);
    let ranked: Vec<u32> = docs.iter().rev().copied().collect();
    let token = Token::default();
    let mut counts = Counts::default();
    let hurried = Sifting { limit: 5, budget: Duration::ZERO, sorter: None, token: &token };
    let verified = sift(&probe, &ranked, &hurried, &mut counts, Instant::now()).expect("not cancelled");
    assert!(!verified.exact, "the count stops at the budget");
    assert!(verified.total < 600);
    let order: Vec<u32> = verified.found.iter().map(|&(doc, _)| doc).collect();
    assert_eq!(order, [599, 598, 597, 596, 595]);
    let patient = Sifting { limit: 5, budget: Duration::from_secs(3600), sorter: None, token: &token };
    let verified = sift(&probe, &ranked, &patient, &mut counts, Instant::now()).expect("not cancelled");
    assert!(verified.exact);
    assert_eq!(verified.total, 600);
}

fn crashing_extractor() -> Spawn {
    Arc::new(|| {
        let (requests, input) = std::io::pipe()?;
        let (output, mut replies) = std::io::pipe()?;
        std::thread::spawn(move || {
            let mut line = String::new();
            let mut requests = BufReader::new(requests);
            while matches!(requests.read_line(&mut line), Ok(read) if read > 0) {
                let Some((id, path)) = worker::parse_request(&line) else { return };
                if path.contains("bad") {
                    return;
                }
                let _ = write_frame(&mut replies, id, Outcome::Text, path.as_bytes());
                line.clear();
            }
        });
        Ok(Pipe { input: Box::new(input), output: Box::new(output), stop: Box::new(|| {}) })
    })
}

#[test]
fn a_file_that_keeps_crashing_the_extractor_fails_alone() {
    let spawn = crashing_extractor();
    let stopper = Mutex::new(None);
    let jobs = ["a", "b", "bad", "c", "d"].map(|name| Job { candidate: candidate(name), replaces: None });
    let mut done = Vec::new();
    let mut sink = |step: Step| {
        if let Step::Done(job, outcome, _) = step {
            done.push((job.candidate.path, outcome));
        }
        Ok(())
    };
    Extractor::new(&spawn, &stopper).run(jobs.into_iter().collect(), &|| true, &mut sink).expect("finished");
    done.sort_by(|left, right| left.0.cmp(&right.0));
    let outcomes: Vec<(&str, Outcome)> = done.iter().map(|(path, outcome)| (path.as_str(), *outcome)).collect();
    assert_eq!(
        outcomes,
        [
            ("a", Outcome::Text),
            ("b", Outcome::Text),
            ("bad", Outcome::Failed),
            ("c", Outcome::Text),
            ("d", Outcome::Text),
        ]
    );
}

fn in_process_extractor() -> Spawn {
    Arc::new(|| {
        let (requests, input) = std::io::pipe()?;
        let (output, replies) = std::io::pipe()?;
        std::thread::spawn(move || extract::serve_on(BufReader::new(requests), replies, 1251));
        Ok(Pipe { input: Box::new(input), output: Box::new(output), stop: Box::new(|| {}) })
    })
}

fn wait_until(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn switching_on_indexes_a_folder_and_follows_its_changes() {
    let cache = Folder::new();
    let files = Folder::new();
    let root = files.0.to_str().expect("utf-8 path").to_owned();
    std::fs::write(files.0.join("договор.txt"), "Договор аренды помещения").expect("file");
    std::fs::write(files.0.join("photo.jpg"), "договор").expect("file");
    std::fs::create_dir(files.0.join("node_modules")).expect("folder");
    std::fs::write(files.0.join("node_modules").join("lib.js"), "договор").expect("file");
    let index = Arc::new(Index::default().extracting(cache.0.join("content"), in_process_extractor()));
    index.start(Vec::new(), vec![root.clone()], |_| {});
    index.full_text(true);
    let ready = |files: usize| {
        let index = index.clone();
        move || matches!(index.status().content, ContentStatus::Ready { files: found, .. } if found == files)
    };
    wait_until("the first index", ready(1));
    assert!(index.status().content_cache > 0);
    let path = files.0.join("договор.txt").to_string_lossy().into_owned();
    assert_eq!(found(&index, "full:аренды"), [path.as_str()]);

    index.full_text(false);
    wait_until("the switch off", || index.status().content == ContentStatus::Off);
    assert_eq!(results(&index, "full:аренды").listed, Listed::ContentOff);
    assert!(index.status().content_cache > 0, "turning off keeps the cache");

    std::fs::write(files.0.join("второй.txt"), "Второй договор").expect("file");
    index.refresh_folder(&root);
    wait_until("the new walk", || index.folders().iter().any(|status| status.entries >= 4 && !status.walking));
    index.full_text(true);
    wait_until("the updated index", ready(2));
    assert_eq!(found(&index, "full:аренды"), [path.as_str()], "the cached text needs no second reading");
    assert_eq!(found(&index, "full:договор").len(), 2);

    let marker = cache.0.join("content").join("marker");
    std::fs::write(&marker, "gone with the cache").expect("file");
    index.delete_content();
    wait_until("the deleted cache", || !marker.exists());
    wait_until("the rebuilt cache", ready(2));
    assert_eq!(found(&index, "full:договор").len(), 2);
    index.close_content();
    assert_eq!(index.status().content, ContentStatus::Off);
}

#[test]
fn only_visible_documents_outside_the_noise_are_taken() {
    let mut volume = Volume::new('C');
    let meta = Meta { size: 10, modified: WHEN };
    let users = volume.insert(ROOT, "Users", true);
    let cache = volume.insert(users, "AppData", true);
    let deep = volume.insert(cache, "notes", true);
    let go = volume.insert(users, "go", true);
    let pkg = volume.insert(go, "pkg", true);
    volume.insert_entry(users, "читать.txt", false, false, meta);
    volume.insert_entry(users, "скрытый.txt", false, true, meta);
    volume.insert_entry(users, "пустой.txt", false, false, Meta::default());
    volume.insert_entry(users, "фото.jpg", false, false, meta);
    volume.insert_entry(deep, "внутри.txt", false, false, meta);
    volume.insert_entry(pkg, "модуль.txt", false, false, meta);
    volume.insert_entry(go, "рядом.txt", false, false, meta);
    let mut taken = Vec::new();
    let wanted = |lower: &str, _: u64| Format::of(lower).is_some();
    volume.documents(&CONTENT_NOISE, wanted, |index, _| taken.push(volume.path(index)));
    taken.sort();
    assert_eq!(taken, ["C:\\Users\\go\\рядом.txt", "C:\\Users\\читать.txt"]);
}

#[test]
fn only_journal_edits_that_can_change_documents_wake_the_content_index() {
    let mut builder = Builder::new('C', 5);
    builder.push(10, 5, "Users", true);
    builder.push(11, 10, "Documents", true);
    builder.push(12, 11, "report.docx", false);
    builder.push(13, 10, "AppData", true);
    builder.push(14, 13, "Local", true);
    builder.push(15, 14, "chrome.log", false);
    builder.push(16, 10, "Projects", true);
    let mut volume = builder.finish(None, true);
    let mut wakes = |change: Change| {
        volume.apply(&[change]);
        let edits = volume.take_edits();
        volume.edits_documents(&edits, &CONTENT_NOISE, document)
    };
    let upsert = |id, parent, name: &str, dir| Change::Upsert { id, parent, name: name.into(), dir, hidden: false };
    assert!(!wakes(Change::Touch(15)), "a log written under AppData");
    assert!(!wakes(upsert(17, 14, "cache.json", false)), "a file created under AppData");
    assert!(!wakes(upsert(18, 10, "New folder", true)), "an empty folder");
    assert!(!wakes(upsert(19, 11, "photo.jpg", false)), "a file that is no document");
    assert!(wakes(Change::Touch(12)), "a document written");
    assert!(wakes(upsert(12, 11, "report.tmp", false)), "a document renamed away");
    assert!(wakes(upsert(20, 11, "заметки.txt", false)), "a document created");
    assert!(wakes(Change::Remove(20)), "a document deleted");
    assert!(wakes(upsert(16, 10, "Work", true)), "a folder renamed");
    assert!(!wakes(upsert(14, 13, "LocalLow", true)), "a folder renamed under AppData");
}

#[test]
fn unread_files_are_counted_by_their_reason() {
    let rows = [Outcome::Text, Outcome::Denied, Outcome::Denied, Outcome::Empty, Outcome::Unsupported, Outcome::Failed];
    let mut docs = Docs::default();
    for (number, outcome) in rows.into_iter().enumerate() {
        docs.add(seen(&format!("C:\\{number}.txt"), 1, 1), outcome, Vec::new());
    }
    docs.add(seen("C:\\gone.txt", 1, 1), Outcome::Encrypted, Vec::new());
    docs.retire(6);
    let expected = Unread { denied: 2, empty: 1, binary: 1, encrypted: 0, failed: 1 };
    assert_eq!(docs.counts(), (1, expected));
}

#[test]
fn the_estimate_weighs_documents_by_how_much_text_they_hold() {
    let candidates = [seen("C:\\a.txt", 1000, 1), seen("C:\\b.pdf", 1000, 1), seen("C:\\c.docx", 1000, 1)];
    let estimate = estimate(&candidates);
    assert_eq!(estimate.files, 3);
    assert_eq!(estimate.text, 1430);
    assert_eq!(estimate.cache, 543);
    assert_eq!(estimate.seconds.1, estimate.seconds.0 * 3);
}

#[test]
fn documents_are_read_first_and_the_newest_of_each_group_leads() {
    let plan = Plan {
        add: vec![
            seen("C:\\old.pdf", 1, 1),
            seen("C:\\new.txt", 1, 9),
            seen("C:\\new.PDF", 1, 9),
            seen("C:\\old.txt", 1, 1),
        ],
        ..Plan::default()
    };
    let order: Vec<String> = jobs_of(plan).into_iter().map(|job| job.candidate.path).collect();
    assert_eq!(order, ["C:\\new.PDF", "C:\\old.pdf", "C:\\new.txt", "C:\\old.txt"]);
}

#[test]
fn a_query_without_a_library_is_empty_while_the_index_opens() {
    let index = Index::default();
    index.content.signal.lock().run = Some(1);
    let results = results(&index, "full:договор");
    assert_eq!(results.listed, Listed::Matches);
    assert!(results.hits.is_empty());
    let (_, counts) = index
        .search_until(
            &parse("full:договор"),
            &Boosts::default(),
            &Focus::Anything,
            10,
            &Facets::default(),
            &Token::default(),
        )
        .expect("not cancelled");
    assert_eq!(counts, Counts::default());
}

const SPEED_QUERIES: [&str; 20] = [
    "договор",
    "аренды",
    "размер",
    "password",
    "\"error handling\"",
    "vestigo",
    "the",
    "fn main",
    "import",
    "function",
    "настройки",
    "Windows",
    "\"public static\"",
    "todo",
    "http",
    "print",
    "отчёт",
    "config",
    "поиск файлов",
    "zzzqqq",
];
const SPEED_ROUNDS: usize = 5;
const BUILD_LIMIT: Duration = Duration::from_secs(60 * 60);

fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    sorted.get(percent * sorted.len() / 100).or(sorted.last()).copied().unwrap_or_default()
}

fn peak_memory() -> String {
    let script = format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id());
    let output = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output();
    let bytes = output.map(|output| String::from_utf8_lossy(&output.stdout).trim().parse::<u64>().unwrap_or(0));
    format!("{} MB", bytes.unwrap_or(0) >> 20)
}

#[test]
#[ignore = "reads the real system drive and its documents; needs administrator rights"]
fn content_speed_on_the_real_drive() {
    let _ = flexi_logger::Logger::try_with_str("warn").map(|logger| logger.start());
    let journal = crate::index::ntfs::Journal::open('C').expect("administrator rights to open the journal");
    let volume = RwLock::new(journal.scan().expect("the MFT is readable"));
    assert!(crate::index::details::fill(&volume), "sizes and dates are read");
    let cache = Folder::new();
    let index = Arc::new(Index::default().extracting(cache.0.join("content"), in_process_extractor()));
    index.volumes.write().push(Arc::new(volume));

    let started = Instant::now();
    let candidates = index.content_candidates().expect("the volume is ready");
    let listed = started.elapsed();
    let files = candidates.len();
    let started = Instant::now();
    let plan = reconcile(&Docs::default(), candidates);
    println!("{files} candidates listed in {listed:?}, first plan in {:?}", started.elapsed());
    println!("estimate: {:?}", index.content_estimate());
    assert_eq!(plan.add.len(), files);

    let started = Instant::now();
    index.full_text(true);
    while !matches!(index.status().content, ContentStatus::Ready { files, .. } if files > 0) {
        assert!(started.elapsed() < BUILD_LIMIT, "the build took too long");
        std::thread::sleep(Duration::from_millis(100));
    }
    while index.content.library.read().as_ref().is_none_or(|library| library.docs.built == 0) {
        assert!(started.elapsed() < BUILD_LIMIT, "the build took too long");
        std::thread::sleep(Duration::from_millis(100));
    }
    let status = index.status();
    println!("built in {:?}: {:?}, cache {} MB", started.elapsed(), status.content, status.content_cache >> 20);
    println!("peak memory of the whole test process: {}", peak_memory());

    let started = Instant::now();
    let candidates = index.content_candidates().expect("the volume is ready");
    let listed = started.elapsed();
    let guard = index.content.library.read();
    let library = guard.as_ref().expect("library");
    let started = Instant::now();
    let plan = reconcile(&library.docs, candidates);
    println!("sweep over the built index: listed in {listed:?}, reconciled in {:?}", started.elapsed());
    println!("left to do: {} added, {} updated", plan.add.len(), plan.update.len());
    drop(guard);

    let mut every = Vec::new();
    for text in SPEED_QUERIES {
        let query = parse(&format!("full:{text}"));
        let mut times = Vec::new();
        let mut total = 0;
        let mut listed = Listed::Matches;
        for _ in 0..SPEED_ROUNDS {
            let started = Instant::now();
            let found = index.search(&query, &Boosts::default(), &Focus::Anything, 50);
            times.push(started.elapsed());
            (total, listed) = (found.total, found.listed);
        }
        times.sort();
        println!("{text:>20}: {total:>6} {listed:?}, median {:?}", percentile(&times, 50));
        every.extend(times);
    }
    every.sort();
    let (p50, p90, p99) = (percentile(&every, 50), percentile(&every, 90), percentile(&every, 99));
    println!("all queries: p50 {p50:?}, p90 {p90:?}, p99 {p99:?}");

    let started = Instant::now();
    index.close_content();
    println!("closed and saved in {:?}", started.elapsed());
    let started = Instant::now();
    index.full_text(true);
    while !matches!(index.status().content, ContentStatus::Ready { .. }) {
        assert!(started.elapsed() < BUILD_LIMIT, "opening took too long");
        std::thread::sleep(Duration::from_millis(1));
    }
    println!("opened again in {:?}", started.elapsed());
    index.close_content();
}
