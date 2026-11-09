use super::volume::{Builder, Change, Meta, ROOT, Volume};
use super::*;
use parking_lot::RwLock;

use crate::search::{Builtin, Clock, Filter, Grammar};

const ROOT_ID: u64 = 5;

fn grammar() -> Grammar {
    Grammar::new(&Filter::defaults(), Clock { now: 1_784_116_800, offset: 0 })
}

fn parse(text: &str) -> Query {
    Query::parse(text, &grammar()).expect("query is not empty")
}

fn sample() -> Volume {
    let mut builder = Builder::new('C', ROOT_ID);
    builder.push(12, 11, "report.pdf", false);
    builder.push(11, 10, "Documents", true);
    builder.push(10, ROOT_ID, "Users", true);
    builder.push(20, ROOT_ID, "$Recycle.Bin", true);
    builder.push(21, 20, "report-old.pdf", false);
    builder.push(30, ROOT_ID, "Windows", true);
    builder.push(31, 30, "WinSxS", true);
    builder.push(32, 31, "report.dll", false);
    builder.push(40, 999, "orphan-report.txt", false);
    builder.finish(None, false)
}

fn upsert(volume: &mut Volume, id: u64, parent: u64, name: &str, dir: bool) {
    volume.apply(&[Change::Upsert { id, parent, name: name.into(), dir, hidden: false }]);
}

fn remove(volume: &mut Volume, id: u64) {
    volume.apply(&[Change::Remove(id)]);
}

fn index_of(volumes: Vec<Volume>) -> Index {
    let index = Index::default();
    for volume in volumes {
        index.add(volume, Job { generation: 0, fresh: false });
    }
    index
}

fn search(index: &Index, text: &str) -> Vec<String> {
    let query = parse(text);
    index.search(&query, &Boosts::default(), &Focus::Anything, 10).hits.into_iter().map(|hit| hit.path).collect()
}

#[test]
fn builder_links_children_listed_before_parents() {
    let volume = sample();
    let found = volume.matches(&parse("report.pdf"), volume::Prefer::Nothing, &[]);
    let paths: Vec<String> = found.iter().map(|(_, index)| volume.path(*index)).collect();
    assert_eq!(paths, ["C:\\Users\\Documents\\report.pdf"]);
}

#[test]
fn system_folders_and_orphans_stay_out_of_results() {
    let index = index_of(vec![sample()]);
    assert_eq!(search(&index, "report"), ["C:\\Users\\Documents\\report.pdf"]);
}

#[test]
fn kind_alone_lists_matching_entries_in_index() {
    let index = index_of(vec![sample()]);
    let kind = |kind| {
        let query = Query::parse_with("", &grammar(), Some(Filter::builtin(kind).rule())).expect("a kind is a query");
        index
            .search(&query, &Boosts::default(), &Focus::Anything, 10)
            .hits
            .into_iter()
            .map(|hit| hit.path)
            .collect::<Vec<_>>()
    };
    assert_eq!(kind(Builtin::Documents), ["C:\\Users\\Documents\\report.pdf"]);
    assert_eq!(kind(Builtin::Folders), ["C:\\", "C:\\Users", "C:\\Windows", "C:\\Users\\Documents"]);
    assert!(kind(Builtin::Images).is_empty());
}

#[test]
fn journal_changes_update_names_and_parents() {
    let mut volume = sample();
    upsert(&mut volume, 50, 11, "notes.md", false);
    upsert(&mut volume, 12, 10, "summary.pdf", false);
    remove(&mut volume, 50);
    let index = index_of(vec![volume]);
    assert_eq!(search(&index, "summary"), ["C:\\Users\\summary.pdf"]);
    assert!(search(&index, "notes").is_empty());
    assert!(search(&index, "report.pdf").is_empty());
}

#[test]
fn a_checkpoint_behind_the_purged_records_needs_a_new_scan() {
    let at = |usn| volume::Checkpoint { journal: 7, usn };
    assert!(!ntfs::holds(at(1_000), 7, 2_000..=3_000));
    assert!(ntfs::holds(at(2_000), 7, 2_000..=3_000));
    assert!(ntfs::holds(at(3_000), 7, 2_000..=3_000));
    assert!(!ntfs::holds(at(3_001), 7, 2_000..=3_000));
    assert!(!ntfs::holds(at(2_500), 8, 2_000..=3_000));
}

#[test]
fn excluded_folders_drop_their_contents_but_not_their_neighbours() {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "Work", true);
    let old = volume.insert(work, "Old", true);
    let home = volume.insert(ROOT, "Home", true);
    volume.insert(work, "plan.txt", false);
    volume.insert(old, "plan.txt", false);
    volume.insert(home, "plan.txt", false);
    volume.insert(old, "notes.txt", false);
    volume.insert(work, "notes.txt", false);
    let index = index_of(vec![volume]);
    index.rank(Ranking::new(&["D:\\Work\\Old\\".to_owned()], &[], &[], true));
    let mut plans = search(&index, "plan");
    plans.sort();
    assert_eq!(plans, ["D:\\Home\\plan.txt", "D:\\Work\\plan.txt"]);
    assert!(search(&index, "old").is_empty());
    let filter = Filter { keyword: "w".into(), folder: "d:\\work".into(), ..Filter::default() };
    let query = Query::parse_with("notes", &grammar(), Some(filter.rule())).expect("query");
    let found = index.search(&query, &Boosts::default(), &Focus::Folder("D:\\Home".into()), 10);
    assert_eq!(found.hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(), ["D:\\Work\\notes.txt"]);
}

#[test]
fn a_long_match_list_keeps_its_best_and_counts_the_rest() {
    let mut volume = Volume::new('D');
    let folder = volume.insert(ROOT, "Reports", true);
    for number in 0..SHORTLIST * 2 + 5 {
        volume.insert(folder, &format!("report {number:04}.txt"), false);
    }
    volume.insert(folder, "report.txt", false);
    let index = index_of(vec![volume]);
    let results = index.search(&parse("report"), &Boosts::default(), &Focus::Anything, 1);
    assert_eq!(results.total, SHORTLIST * 2 + 6 + 1);
    assert_eq!(results.hits.into_iter().map(|hit| hit.path).collect::<Vec<_>>(), ["D:\\Reports\\report.txt"]);
}

#[test]
fn a_search_stops_once_its_caller_lets_go() {
    let index = index_of(vec![sample()]);
    let cancel = Cancel::new();
    let token = cancel.token();
    let run =
        || index.search_until(&parse("report"), &Boosts::default(), &Focus::Anything, 10, &Facets::default(), &token);
    assert_eq!(run().map(|(results, _)| results.total), Some(1));
    drop(cancel);
    assert!(run().is_none());
}

#[test]
fn a_located_folder_follows_renames_and_removals() {
    let mut volume = sample();
    let documents = volume.locate("C:\\Users\\Documents").expect("indexed");
    assert_eq!(volume.locate("c:\\users\\documents\\"), Some(documents));
    upsert(&mut volume, 10, ROOT_ID, "People", true);
    assert_eq!(volume.locate("C:\\Users\\Documents"), None);
    assert_eq!(volume.locate("C:\\People\\Documents"), Some(documents));
    remove(&mut volume, 11);
    assert_eq!(volume.locate("C:\\People\\Documents"), None);
}

#[test]
fn a_missing_folder_is_found_once_it_appears() {
    let mut volume = sample();
    assert_eq!(volume.locate("C:\\Users\\Music"), None);
    upsert(&mut volume, 13, 10, "Music", true);
    let music = volume.locate("C:\\Users\\Music").expect("created");
    assert_eq!(volume.locate("C:\\Users\\Songs"), None);
    upsert(&mut volume, 13, 10, "Songs", true);
    assert_eq!(volume.locate("C:\\Users\\Songs"), Some(music));
}

#[test]
fn a_reused_file_record_replaces_the_file_it_held() {
    let mut volume = sample();
    let count = volume.len();
    upsert(&mut volume, 12 | 2 << 48, 11, "summary.pdf", false);
    assert_eq!(volume.len(), count);
    let index = index_of(vec![volume]);
    assert_eq!(search(&index, "summary"), ["C:\\Users\\Documents\\summary.pdf"]);
    assert!(search(&index, "report.pdf").is_empty());
}

fn with_recycled_folder() -> Volume {
    let mut builder = Builder::new('C', ROOT_ID);
    builder.push(10, ROOT_ID, "Users", true);
    builder.push(20, ROOT_ID, "$Recycle.Bin", true);
    builder.push(22, 20, "$RAB12CD", true);
    builder.push(23, 22, "plan.txt", false);
    builder.push(40, 999, "orphan-plan.txt", false);
    builder.finish(Some(volume::Checkpoint { journal: 7, usn: 1 }), false)
}

#[test]
fn folder_moved_to_the_recycle_bin_hides_its_contents() {
    let mut volume = sample();
    upsert(&mut volume, 11, 20, "$R1A2B3C", true);
    let index = index_of(vec![volume]);
    assert!(search(&index, "report").is_empty());
}

#[test]
fn folder_restored_from_the_recycle_bin_shows_its_contents() {
    let mut volume = with_recycled_folder();
    assert!(search(&index_of(vec![with_recycled_folder()]), "plan").is_empty());
    upsert(&mut volume, 22, 10, "Old", true);
    assert_eq!(search(&index_of(vec![volume]), "plan"), ["C:\\Users\\Old\\plan.txt"]);
}

#[test]
fn cache_keeps_recycled_folders_restorable() {
    let path = std::env::temp_dir().join(format!("vestigo-cache-recycled-{}.bin", std::process::id()));
    cache::save(&path, &with_recycled_folder()).expect("cache saved");
    let restored = cache::load(&path, 'C');
    let _ = std::fs::remove_file(&path);
    let mut volume = restored.expect("cache loaded");
    upsert(&mut volume, 22, 10, "Old", true);
    assert_eq!(search(&index_of(vec![volume]), "plan"), ["C:\\Users\\Old\\plan.txt"]);
}

#[test]
fn cache_skips_ignored_system_folders_and_orphans() {
    let path = std::env::temp_dir().join(format!("vestigo-cache-ignored-{}.bin", std::process::id()));
    let mut volume = sample();
    volume.set_checkpoint(volume::Checkpoint { journal: 7, usn: 1 });
    cache::save(&path, &volume).expect("cache saved");
    let restored = cache::load(&path, 'C');
    let _ = std::fs::remove_file(&path);
    assert_eq!(restored.expect("cache loaded").len(), 6);
}

#[test]
fn replayed_changes_do_not_grow_the_index() {
    let mut volume = sample();
    upsert(&mut volume, 50, 11, "notes.md", false);
    let footprint = volume.footprint();
    upsert(&mut volume, 50, 11, "notes.md", false);
    upsert(&mut volume, 50, 11, "notes.md", false);
    assert_eq!(volume.footprint(), footprint);
}

#[test]
fn compaction_drops_removed_entries_and_keeps_the_rest() {
    let mut volume = with_recycled_folder();
    upsert(&mut volume, 30, 10, "draft.txt", false);
    upsert(&mut volume, 31, 10, "final.txt", false);
    remove(&mut volume, 30);
    let mut compacted = volume.compacted();
    assert!(compacted.footprint() < volume.footprint());
    assert_eq!(compacted.len(), 5);
    assert_eq!(compacted.checkpoint(), Some(volume::Checkpoint { journal: 7, usn: 1 }));
    upsert(&mut compacted, 22, 10, "Old", true);
    let index = index_of(vec![compacted]);
    assert_eq!(search(&index, "final"), ["C:\\Users\\final.txt"]);
    assert!(search(&index, "draft").is_empty());
    assert_eq!(search(&index, "plan"), ["C:\\Users\\Old\\plan.txt"]);
}

#[test]
fn removal_then_recreation_keeps_count() {
    let mut volume = sample();
    let before = volume.len();
    remove(&mut volume, 12);
    assert_eq!(volume.len(), before - 1);
    upsert(&mut volume, 12, 11, "report.pdf", false);
    assert_eq!(volume.len(), before);
}

#[test]
fn exact_and_prefix_matches_rank_first() {
    let mut volume = Volume::new('D');
    let projects = volume.insert(ROOT, "projects", true);
    volume.insert(projects, "my-vestigo-notes.txt", false);
    volume.insert(projects, "vestigo", true);
    volume.insert(projects, "vestigobackup.zip", false);
    let index = index_of(vec![volume]);
    assert_eq!(
        search(&index, "vestigo"),
        ["D:\\projects\\vestigo", "D:\\projects\\vestigobackup.zip", "D:\\projects\\my-vestigo-notes.txt"]
    );
}

#[test]
fn path_terms_filter_by_folder() {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "work", true);
    let home = volume.insert(ROOT, "home", true);
    volume.insert(work, "plan.txt", false);
    volume.insert(home, "plan.txt", false);
    let reports = volume.insert(work, "Отчёты 2026", true);
    volume.insert(reports, "plan.txt", false);
    let index = index_of(vec![volume]);
    assert_eq!(search(&index, "work\\plan"), ["D:\\work\\plan.txt", "D:\\work\\Отчёты 2026\\plan.txt"]);
    assert_eq!(search(&index, "home/"), ["D:\\home\\plan.txt"]);
    assert_eq!(search(&index, "отч\\ plan"), ["D:\\work\\Отчёты 2026\\plan.txt"]);
    assert_eq!(search(&index, "d:\\work\\plan"), ["D:\\work\\plan.txt"]);
    assert_eq!(search(&index, "WORK\\ОТЧЁТЫ"), ["D:\\work\\Отчёты 2026"]);
    assert_eq!(search(&index, "2026\\plan"), ["D:\\work\\Отчёты 2026\\plan.txt"]);
}

#[test]
fn folder_words_match_parts_of_ancestor_names_in_order() {
    let mut volume = Volume::new('D');
    let projects = volume.insert(ROOT, "Projects", true);
    let traydex = volume.insert(projects, "Traydex", true);
    let source = volume.insert(traydex, "src", true);
    volume.insert(source, "main.rs", false);
    let other = volume.insert(projects, "other", true);
    let other_source = volume.insert(other, "src", true);
    volume.insert(other_source, "main.rs", false);
    let index = index_of(vec![volume]);
    let found = ["D:\\Projects\\Traydex\\src\\main.rs"];
    assert_eq!(search(&index, "tray\\ main"), found);
    assert_eq!(search(&index, "main tray\\"), found);
    assert_eq!(search(&index, "proj\\tray\\main"), found);
    assert_eq!(search(&index, "src\\ tray\\ main"), found);
    assert_eq!(search(&index, "src\\tray\\ main"), Vec::<String>::new());
}

#[test]
fn plain_words_missing_from_the_name_may_name_its_folders() {
    let mut volume = Volume::new('D');
    let projects = volume.insert(ROOT, "Projects", true);
    let traydex = volume.insert(projects, "Traydex", true);
    let source = volume.insert(traydex, "src", true);
    volume.insert(source, "main.rs", false);
    let other = volume.insert(projects, "other", true);
    volume.insert(other, "main.rs", false);
    volume.insert(other, "Tray main notes.txt", false);
    let отчёты = volume.insert(ROOT, "Отчёты", true);
    volume.insert(отчёты, "май.docx", false);
    let index = index_of(vec![volume]);
    assert_eq!(
        search(&index, "tray main"),
        ["D:\\Projects\\Traydex\\src\\main.rs", "D:\\Projects\\other\\Tray main notes.txt"]
    );
    assert_eq!(search(&index, "main tray src"), ["D:\\Projects\\Traydex\\src\\main.rs"]);
    assert_eq!(search(&index, "ОТЧЁТ май"), ["D:\\Отчёты\\май.docx"]);
    assert_eq!(search(&index, "tray"), ["D:\\Projects\\Traydex", "D:\\Projects\\other\\Tray main notes.txt"]);
    assert_eq!(search(&index, "traydex projects"), ["D:\\Projects\\Traydex"]);
    assert_eq!(search(&index, "tray absent"), Vec::<String>::new());
}

fn folders() -> Index {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "work", true);
    volume.insert(work, "b.txt", false);
    let tools = volume.insert(work, "a-tools", true);
    volume.insert(work, "C.txt", false);
    let hidden = volume.insert(work, "hidden", true);
    volume.set_meta(hidden, Meta::default(), true);
    let reports = volume.insert(work, "Отчёты", true);
    volume.insert(tools, "deep.txt", false);
    volume.insert(reports, "deep.txt", false);
    let workshop = volume.insert(ROOT, "workshop", true);
    volume.insert(workshop, "deep.txt", false);
    let home = volume.insert(ROOT, "home", true);
    volume.insert(home, "a-tools", true);
    let archive = volume.insert(ROOT, "archive", true);
    let old = volume.insert(archive, "old", true);
    volume.insert(old, "a-tools", true);
    index_of(vec![volume])
}

fn listing(index: &Index, text: &str) -> (Listed, Vec<String>) {
    let results = index.search(&parse(text), &Boosts::default(), &Focus::Anything, 10);
    (results.listed, results.hits.into_iter().map(|hit| hit.path).collect())
}

#[test]
fn a_lone_place_lists_its_folder_folders_first() {
    let index = folders();
    let contents = ["D:\\work\\a-tools", "D:\\work\\Отчёты", "D:\\work\\hidden", "D:\\work\\b.txt", "D:\\work\\C.txt"];
    assert_eq!(listing(&index, "\\work"), (Listed::Contents, contents.map(String::from).to_vec()));
    assert_eq!(listing(&index, "\\WORK/"), (Listed::Contents, contents.map(String::from).to_vec()));
    assert_eq!(listing(&index, "d:\\work"), (Listed::Contents, contents.map(String::from).to_vec()));
    assert_eq!(listing(&index, "\\work\\ОТЧЁТЫ"), (Listed::Contents, vec!["D:\\work\\Отчёты\\deep.txt".to_owned()]));
    assert_eq!(index.search(&parse("\\work"), &Boosts::default(), &Focus::Anything, 2).total, 5);
}

#[test]
fn a_large_folder_lists_the_rows_around_the_one_to_keep() {
    let mut volume = Volume::new('D');
    let big = volume.insert(ROOT, "big", true);
    for row in 0..60 {
        volume.insert(big, &format!("f{row:02}.txt"), false);
    }
    let index = index_of(vec![volume]);
    let names = |query: &Query| {
        let results = index.search(query, &Boosts::default(), &Focus::Anything, 10);
        results.hits.iter().map(|hit| hit.name().to_owned()).collect::<Vec<_>>()
    };
    let mut query = parse("\\big\\");
    assert_eq!(names(&query).first().map(String::as_str), Some("f00.txt"));
    query.keep("D:\\big\\F55.txt");
    let kept = names(&query);
    assert_eq!((kept.first().map(String::as_str), kept.len()), (Some("f50.txt"), 10));
    query.keep("D:\\big\\f20.txt");
    assert_eq!(names(&query).first().map(String::as_str), Some("f15.txt"));
}

#[test]
fn a_typed_tail_filters_the_parent_until_it_names_a_folder() {
    let index = folders();
    assert_eq!(listing(&index, "\\work\\a-t"), (Listed::Matches, vec!["D:\\work\\a-tools".to_owned()]));
    assert_eq!(listing(&index, "\\work\\a-tools"), (Listed::Contents, vec!["D:\\work\\a-tools\\deep.txt".to_owned()]));
    assert_eq!(listing(&index, "\\work\\nothing\\"), (Listed::Matches, Vec::new()));
}

#[test]
fn a_folder_name_found_in_several_places_lists_those_folders() {
    let index = folders();
    let all = ["D:\\home\\a-tools", "D:\\work\\a-tools", "D:\\archive\\old\\a-tools"].map(String::from).to_vec();
    assert_eq!(listing(&index, "\\a-tools"), (Listed::Matches, all.clone()));
    assert_eq!(listing(&index, "\\a-tools\\"), (Listed::Matches, all));
}

#[test]
fn a_place_with_words_searches_its_whole_subtree_only() {
    let index = folders();
    let mut deep = search(&index, "\\work deep");
    deep.sort();
    assert_eq!(deep, ["D:\\work\\a-tools\\deep.txt", "D:\\work\\Отчёты\\deep.txt"]);
    assert_eq!(search(&index, "\\work\\a-tools\\ deep"), ["D:\\work\\a-tools\\deep.txt"]);
    assert!(search(&index, "\\nowhere deep").is_empty());
}

#[test]
fn cyrillic_names_match_case_insensitively() {
    let mut volume = Volume::new('D');
    volume.insert(ROOT, "Отчёт за Май.docx", false);
    let index = index_of(vec![volume]);
    assert_eq!(search(&index, "май отчёт"), ["D:\\Отчёт за Май.docx"]);
}

#[test]
fn boosts_lift_recently_opened_items() {
    let mut volume = Volume::new('D');
    volume.insert(ROOT, "alpha.txt", false);
    volume.insert(ROOT, "alpha-long-name.txt", false);
    let index = index_of(vec![volume]);
    let mut boosts = Boosts::default();
    boosts.insert("d:\\alpha-long-name.txt".into(), 100);
    let query = parse("alpha");
    let hits = index.search(&query, &boosts, &Focus::Anything, 10).hits;
    assert_eq!(hits.first().map(|hit| hit.path.as_str()), Some("D:\\alpha-long-name.txt"));
}

fn search_with(index: &Index, text: &str, boosts: &Boosts, focus: Focus) -> Vec<String> {
    let query = parse(text);
    index.search(&query, boosts, &focus, 10).hits.into_iter().map(|hit| hit.path).collect()
}

fn with_start_menu() -> Volume {
    let mut volume = Volume::new('C');
    let data = volume.insert(ROOT, "ProgramData", true);
    let start = volume.insert(data, "Start Menu", true);
    let programs = volume.insert(start, "Programs", true);
    volume.insert(programs, "Visual Studio Code.lnk", false);
    volume.insert(programs, "code-readme.txt", false);
    volume.insert(ROOT, "code", true);
    volume
}

#[test]
fn programs_rank_first_in_the_launcher_only() {
    let index = index_of(vec![with_start_menu()]);
    let shortcut = "C:\\ProgramData\\Start Menu\\Programs\\Visual Studio Code.lnk";
    let launcher = search_with(&index, "code", &Boosts::default(), Focus::Programs);
    assert_eq!(launcher.first().map(String::as_str), Some(shortcut));
    assert_eq!(launcher.get(1).map(String::as_str), Some("C:\\code"));
    let docked = search_with(&index, "code", &Boosts::default(), Focus::Anything);
    assert_eq!(docked.first().map(String::as_str), Some("C:\\code"));
}

fn with_loose_tools() -> Volume {
    let mut builder = Builder::new('C', ROOT_ID);
    builder.push(14, 13, "Paint Tool.lnk", false);
    builder.push(13, ROOT_ID, "tools", true);
    builder.push(12, 11, "Programs", true);
    builder.push(11, 10, "Start Menu", true);
    builder.push(10, ROOT_ID, "Users", true);
    builder.push(15, ROOT_ID, "paint", true);
    builder.finish(None, false)
}

fn first(index: &Index, text: &str) -> Option<String> {
    search_with(index, text, &Boosts::default(), Focus::Programs).into_iter().next()
}

#[test]
fn shortcuts_moved_into_the_start_menu_become_programs() {
    assert_eq!(first(&index_of(vec![with_loose_tools()]), "paint").as_deref(), Some("C:\\paint"));
    let mut volume = with_loose_tools();
    upsert(&mut volume, 13, 12, "tools", true);
    let moved = Some("C:\\Users\\Start Menu\\Programs\\tools\\Paint Tool.lnk");
    assert_eq!(first(&index_of(vec![volume]), "paint").as_deref(), moved);
}

#[test]
fn the_current_folder_ranks_first() {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "Work", true);
    let deep = volume.insert(work, "old", true);
    volume.insert(deep, "my plan draft.txt", false);
    volume.insert(ROOT, "plan.txt", false);
    let other = Volume::new('E');
    let index = index_of(vec![volume, other]);
    assert_eq!(
        search_with(&index, "plan", &Boosts::default(), Focus::Folder("d:\\WORK\\".into())),
        ["D:\\Work\\old\\my plan draft.txt", "D:\\plan.txt"]
    );
    assert_eq!(
        search_with(&index, "plan", &Boosts::default(), Focus::Folder("E:\\Work".into())),
        ["D:\\plan.txt", "D:\\Work\\old\\my plan draft.txt"]
    );
}

#[test]
fn wrong_layout_matches_come_after_every_typed_match() {
    let mut volume = Volume::new('D');
    let data = volume.insert(ROOT, "ProgramData", true);
    let start = volume.insert(data, "Start Menu", true);
    let programs = volume.insert(start, "Programs", true);
    volume.insert(programs, "readme.lnk", false);
    volume.insert(ROOT, "README.md", false);
    let notes = volume.insert(ROOT, "notes", true);
    volume.insert(notes, "старый куфвьу-список.txt", false);
    let index = index_of(vec![volume]);
    let mut boosts = Boosts::default();
    boosts.insert("d:\\readme.md".into(), 100);
    assert_eq!(
        search_with(&index, "куфвьу", &boosts, Focus::Programs),
        ["D:\\notes\\старый куфвьу-список.txt", "D:\\ProgramData\\Start Menu\\Programs\\readme.lnk", "D:\\README.md"]
    );
}

#[test]
fn programs_forgive_abbreviations_and_typos() {
    let index = index_of(vec![with_start_menu()]);
    let shortcut = Some("C:\\ProgramData\\Start Menu\\Programs\\Visual Studio Code.lnk");
    for text in ["vsc", "vscode", "visaul", "vsco", "viscode"] {
        assert_eq!(first(&index, text).as_deref(), shortcut, "{text}");
    }
    let mut builder = Builder::new('C', ROOT_ID);
    builder.push(10, ROOT_ID, "ProgramData", true);
    builder.push(11, 10, "Microsoft", true);
    builder.push(12, 11, "Windows", true);
    builder.push(13, 12, "Start Menu", true);
    builder.push(14, 13, "Programs", true);
    builder.push(15, 14, "Accessories", true);
    builder.push(16, 15, "Notepad.lnk", false);
    let index = index_of(vec![builder.finish(None, false)]);
    let notepad = Some("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Accessories\\Notepad.lnk");
    assert_eq!(first(&index, "notpad").as_deref(), notepad);
    assert!(search_with(&index, "visaul", &Boosts::default(), Focus::Anything).is_empty());
}

#[test]
fn start_menu_shortcuts_missing_from_the_mft_scan_are_read_from_disk() {
    let root = std::env::temp_dir().join(format!("vestigo-fill-{}", std::process::id()));
    let programs = root.join("Start Menu").join("Programs");
    std::fs::create_dir_all(programs.join("Accessories")).expect("test folders");
    std::fs::write(programs.join("Accessories").join("Notepad.lnk"), b"").expect("shortcut");
    std::fs::write(programs.join("Known.lnk"), b"").expect("shortcut");
    let mut parts = programs.to_str().expect("utf-8 path").split('\\');
    let letter = parts.next().and_then(|drive| drive.chars().next()).expect("drive letter");
    let mut volume = Volume::new(letter);
    let folder = parts.fold(ROOT, |parent, part| volume.insert(parent, part, true));
    volume.insert(folder, "Known.lnk", false);
    let before = volume.len();
    volume.fill(std::slice::from_ref(&programs));
    let once = volume.len();
    volume.fill(std::slice::from_ref(&programs));
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!((once - before, volume.len()), (2, once));
    let index = index_of(vec![volume]);
    let found = first(&index, "notpad").expect("the shortcut is indexed");
    assert!(found.ends_with("\\Programs\\Accessories\\Notepad.lnk"), "{found}");
}

#[test]
fn fuzzy_matches_in_programs_do_not_outrank_closer_matches() {
    let mut volume = with_start_menu();
    let programs = volume.locate("c:\\programdata\\start menu\\programs").expect("start menu");
    volume.insert(programs, "NVIDIA Control Panel.lnk", false);
    volume.insert(programs, "Control Panel.lnk", false);
    volume.insert(programs, "Remote Desktop Connection.lnk", false);
    let games = volume.insert(ROOT, "Games", true);
    let citron = volume.insert(games, "Citron", true);
    volume.insert(citron, "citron.exe", false);
    let index = index_of(vec![volume]);
    assert_eq!(first(&index, "ctron").as_deref(), Some("C:\\Games\\Citron\\citron.exe"));
    assert_eq!(first(&index, "control").as_deref(), Some("C:\\ProgramData\\Start Menu\\Programs\\Control Panel.lnk"));
}

#[test]
fn the_current_folder_forgives_typos() {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "Work", true);
    volume.insert(work, "Отчёт за май.docx", false);
    let index = index_of(vec![volume]);
    let near = Focus::Folder("D:\\Work".into());
    assert_eq!(search_with(&index, "отчот", &Boosts::default(), near), ["D:\\Work\\Отчёт за май.docx"]);
    assert!(search_with(&index, "отчот", &Boosts::default(), Focus::Anything).is_empty());
}

#[test]
fn shortcut_labels_hide_their_extension() {
    let shortcut = Hit::new("C:\\Start Menu\\Visual Studio Code.lnk".into(), false);
    assert_eq!(shortcut.label(true), "Visual Studio Code");
    assert_eq!(shortcut.label(false), "Visual Studio Code.lnk");
    assert_eq!(Hit::new("C:\\Games\\Portal.URL".into(), false).label(true), "Portal");
    assert_eq!(Hit::new("C:\\links.lnk".into(), true).label(true), "links.lnk");
    assert_eq!(Hit::new("C:\\.lnk".into(), false).label(true), ".lnk");
    assert_eq!(Hit::new("C:\\notes.txt".into(), false).label(true), "notes.txt");
}

#[test]
fn hits_split_name_and_folder() {
    let hit = Hit::new("C:\\Users\\Alex".into(), true);
    assert_eq!((hit.folder(), hit.name()), ("C:\\Users", "Alex"));
    let root = Hit::new("C:\\".into(), true);
    assert_eq!((root.folder(), root.name()), ("", "C:\\"));
}

#[test]
fn cache_round_trip_keeps_paths_and_checkpoint() {
    let mut volume = sample();
    volume.set_checkpoint(volume::Checkpoint { journal: 7, usn: 1234 });
    upsert(&mut volume, 13, 11, "notes.md", false);
    upsert(&mut volume, 14, 11, "Отчёт за май.docx", false);
    remove(&mut volume, 12);
    let path = std::env::temp_dir().join(format!("vestigo-cache-test-{}.bin", std::process::id()));
    cache::save(&path, &volume).expect("cache saved");
    let restored = cache::load(&path, 'C').expect("cache loaded");
    let _ = std::fs::remove_file(&path);
    assert_eq!(restored.checkpoint(), Some(volume::Checkpoint { journal: 7, usn: 1234 }));
    let index = index_of(vec![restored]);
    assert_eq!(search(&index, "notes"), ["C:\\Users\\Documents\\notes.md"]);
    assert_eq!(search(&index, "ОТЧЁТ"), ["C:\\Users\\Documents\\Отчёт за май.docx"]);
    assert!(search(&index, "report").is_empty());
}

#[test]
fn a_foreign_truncated_or_padded_cache_is_refused() {
    let mut volume = sample();
    volume.set_checkpoint(volume::Checkpoint { journal: 7, usn: 1234 });
    let path = std::env::temp_dir().join(format!("vestigo-damaged-cache-{}.bin", std::process::id()));
    cache::save(&path, &volume).expect("cache saved");
    let saved = std::fs::read(&path).expect("cache read");
    let refused = |bytes: &[u8], letter| {
        std::fs::write(&path, bytes).expect("cache written");
        cache::load(&path, letter).is_err()
    };
    let mut padded = saved.clone();
    padded.insert(saved.len() - 20, b'x');
    let results = [
        refused(&saved, 'C'),
        refused(&saved, 'D'),
        refused(&saved[..saved.len() - 1], 'C'),
        refused(&saved[..40], 'C'),
        refused(&padded, 'C'),
    ];
    let _ = std::fs::remove_file(&path);
    assert_eq!(results, [false, true, true, true, true]);
}

#[test]
#[ignore = "reads the real system drive; needs administrator rights"]
fn system_drive_scan_and_search_speed() {
    let _ = flexi_logger::Logger::try_with_str("info").map(|logger| logger.start());
    let journal = ntfs::Journal::open('C').expect("administrator rights to open the journal");
    let started = Instant::now();
    let volume = journal.scan().expect("the MFT is readable");
    let scanned = started.elapsed();
    let path = std::env::temp_dir().join("vestigo-speed-test.bin");
    cache::save(&path, &volume).expect("cache saved");
    let started = Instant::now();
    let mut volume = cache::load(&path, 'C').expect("cache loaded");
    volume.fill(&drive::start_menus('C'));
    let loaded = started.elapsed();
    let _ = std::fs::remove_file(&path);
    let entries = volume.len();
    let volume = RwLock::new(volume);
    let started = Instant::now();
    assert!(details::fill(&volume), "sizes and dates are read");
    let detailed = started.elapsed();
    let volume = volume.into_inner();
    let started = Instant::now();
    cache::save(&path, &volume).expect("cache with details saved");
    let saved = started.elapsed();
    let started = Instant::now();
    let reloaded = cache::load(&path, 'C').expect("cache with details loaded");
    let reloaded_in = started.elapsed();
    let _ = std::fs::remove_file(&path);
    drop(reloaded);
    let index = index_of(vec![volume]);
    println!("{entries} entries: scanned in {scanned:?}, loaded from cache in {loaded:?}");
    println!("sizes and dates read in {detailed:?}; cache with them saved in {saved:?}, loaded in {reloaded_in:?}");
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let cases = [
        ("claude", Focus::Anything),
        ("a", Focus::Anything),
        ("readme", Focus::Anything),
        ("notepad", Focus::Anything),
        ("claude", Focus::Programs),
        ("сдфгву", Focus::Programs),
        ("vsc", Focus::Programs),
        ("notepad", Focus::Programs),
        ("notpad", Focus::Programs),
        ("ctron", Focus::Anything),
        ("ctron", Focus::Programs),
        ("visualstudiocode", Focus::Programs),
        ("a", Focus::Programs),
        ("a", Focus::Folder(home.clone())),
        ("readme", Focus::Folder(home.clone())),
        ("size:>100mb", Focus::Anything),
        ("date:today", Focus::Anything),
        ("ext:pdf", Focus::Anything),
        ("report -tmp", Focus::Anything),
        ("doc: report date:year", Focus::Anything),
        ("\\system32", Focus::Anything),
        ("c:\\windows\\system32\\", Focus::Anything),
        ("\\windows\\sys", Focus::Anything),
        ("\\windows readme", Focus::Programs),
        ("win\\ notepad", Focus::Anything),
        ("users\\ readme", Focus::Anything),
        ("s\\ a", Focus::Anything),
        ("users readme", Focus::Anything),
        ("a e", Focus::Anything),
    ];
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64);
    let clock = Clock { now, offset: 0 };
    let today = Grammar::new(&Filter::defaults(), clock);
    for (text, focus) in cases {
        let query = Query::parse(text, &today).expect("query is not empty");
        let mut fastest = Duration::MAX;
        let mut results = Results::default();
        for _ in 0..5 {
            let started = Instant::now();
            results = index.search(&query, &Boosts::default(), &focus, 50);
            fastest = fastest.min(started.elapsed());
        }
        println!("{text:?} {focus:?}: {} matches, fastest of 5 in {fastest:?}", results.total);
        for hit in results.hits.iter().take(5) {
            println!("  {}", hit.path);
        }
    }
    let facets = |kind: Option<usize>| Facets {
        kinds: Filter::defaults().iter().map(Filter::rule).collect(),
        dates: crate::search::Period::ALL.into_iter().map(|period| period.range(clock)).collect(),
        chosen: Facets::mask(kind),
        date: None,
    };
    let everything =
        Query::parse_with("", &grammar(), Some(crate::search::Rule::default())).expect("a scope is a query");
    for (label, query, kind) in [("a", parse("a"), None), ("a", parse("a"), Some(4)), ("images", everything, Some(4))] {
        let mut fastest = Duration::MAX;
        let mut total = 0;
        for _ in 0..5 {
            let started = Instant::now();
            total = index.search_with(&query, &Boosts::default(), &Focus::Programs, 400, &facets(kind)).0.total;
            fastest = fastest.min(started.elapsed());
        }
        println!("window {label:?} kind {kind:?}: {total} matches with counts, fastest of 5 in {fastest:?}");
    }
    let fastest = |query: &Query, focus: &Focus| {
        let mut fastest = Duration::MAX;
        let mut total = 0;
        for _ in 0..5 {
            let started = Instant::now();
            total = index.search(query, &Boosts::default(), focus, 50).total;
            fastest = fastest.min(started.elapsed());
        }
        (total, fastest)
    };
    let scope = Filter { keyword: "home".into(), folder: home.clone(), ..Filter::default() };
    for (text, rule) in
        [("a", Some(scope.rule())), ("readme", Some(scope.rule())), ("windows\\a", None), ("system32\\drivers", None)]
    {
        let query = Query::parse_with(text, &today, rule).expect("query is not empty");
        let (total, time) = fastest(&query, &Focus::Anything);
        println!("scoped {text:?}: {total} matches, fastest of 5 in {time:?}");
    }
    let excluded = [format!("{home}\\AppData"), "C:\\Windows".to_owned(), "C:\\Program Files".to_owned()];
    index.rank(Ranking::new(&excluded, &[], &[], true));
    for (text, focus) in [("a", Focus::Anything), ("readme", Focus::Anything), ("a", Focus::Folder(home))] {
        let (total, time) = fastest(&parse(text), &focus);
        println!("excluding {text:?} {focus:?}: {total} matches, fastest of 5 in {time:?}");
    }
}

const FILE_IDS: u64 = 1_000;

fn scratch_volume(name: &str, files: &[(&str, usize)]) -> (PathBuf, Volume, u32) {
    let root = std::env::temp_dir().join(format!("vestigo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("test folder");
    let mut parts = root.to_str().expect("utf-8 path").split('\\');
    let letter = parts.next().and_then(|drive| drive.chars().next()).expect("drive letter");
    let mut builder = Builder::new(letter, ROOT_ID);
    let mut parent = ROOT_ID;
    for (position, part) in parts.enumerate() {
        let id = 100 + position as u64;
        builder.push(id, parent, part, true);
        parent = id;
    }
    for (position, (file, size)) in files.iter().enumerate() {
        std::fs::write(root.join(file), vec![b'x'; *size]).expect("test file");
        builder.push(FILE_IDS + position as u64, parent, file, false);
    }
    let volume = builder.finish(None, false);
    let folder = volume.locate(root.to_str().expect("utf-8 path")).expect("the folder is indexed");
    (root, volume, folder)
}

fn search_in(volume: Volume, text: &str) -> Vec<String> {
    let index = index_of(vec![volume]);
    search(&index, text).into_iter().map(|path| path.rsplit('\\').next().unwrap_or_default().to_owned()).collect()
}

#[test]
fn sizes_and_dates_are_read_from_disk_for_filters() {
    let (root, volume, _) = scratch_volume("details", &[("small-note.txt", 10), ("big-note.txt", 3 << 20)]);
    let volume = RwLock::new(volume);
    assert!(details::fill(&volume));
    let volume = volume.into_inner();
    assert!(volume.details_complete());
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(search_in(volume, "note size:>1mb date:today"), ["big-note.txt"]);
}

#[test]
fn unknown_details_never_pass_a_date_filter() {
    let (root, volume, _) = scratch_volume("unknown", &[("plan.txt", 10)]);
    let _ = std::fs::remove_dir_all(&root);
    assert!(search_in(volume, "plan date:<2100").is_empty());
}

#[test]
fn touched_files_are_read_again() {
    let (root, volume, folder) = scratch_volume("touched", &[("log.txt", 10)]);
    let volume = RwLock::new(volume);
    assert!(details::fill(&volume));
    std::fs::write(root.join("log.txt"), vec![b'x'; 2 << 20]).expect("grown file");
    let file = volume.read().family().children(folder).first().copied().expect("the file is indexed");
    {
        let mut volume = volume.write();
        volume.apply(&[Change::Touch(FILE_IDS)]);
    }
    details::refresh(&volume);
    let size = volume.read().meta(file).map(|meta| meta.size);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(size, Some(2 << 20));
}

#[test]
fn cache_keeps_sizes_dates_and_hidden_flags() {
    let mut builder = Builder::new('C', ROOT_ID);
    builder.push(10, ROOT_ID, "Users", true);
    builder.push_record(volume::Record {
        id: 11,
        parent: 10,
        name: "secret.txt",
        dir: false,
        hidden: true,
        meta: volume::Meta { size: 5 << 20, modified: 1_780_000_000 },
    });
    let volume = builder.finish(Some(volume::Checkpoint { journal: 7, usn: 1 }), true);
    let path = std::env::temp_dir().join(format!("vestigo-cache-details-{}.bin", std::process::id()));
    cache::save(&path, &volume).expect("cache saved");
    let restored = cache::load(&path, 'C');
    let _ = std::fs::remove_file(&path);
    let restored = restored.expect("cache loaded");
    assert!(restored.details_complete());
    let found = restored.matches(&parse("secret size:>4mb date:2026"), volume::Prefer::Nothing, &[]);
    let [(_, file)] = found.as_slice() else { panic!("one match expected, got {found:?}") };
    assert!(restored.is_hidden(*file));
}

#[test]
fn hidden_entries_rank_below_visible_ones() {
    let mut builder = Builder::new('C', ROOT_ID);
    let hidden = volume::Record {
        id: 11,
        parent: ROOT_ID,
        name: "notes-old",
        dir: true,
        hidden: true,
        meta: volume::Meta::default(),
    };
    builder.push_record(hidden);
    builder.push(12, ROOT_ID, "notes-new", true);
    let index = index_of(vec![builder.finish(None, false)]);
    assert_eq!(search(&index, "notes"), ["C:\\notes-new", "C:\\notes-old"]);
}

#[test]
fn folder_filters_keep_results_inside_their_folder() {
    let mut volume = Volume::new('D');
    let work = volume.insert(ROOT, "Work", true);
    let home = volume.insert(ROOT, "Home", true);
    volume.insert(work, "plan.txt", false);
    volume.insert(home, "plan.txt", false);
    let index = index_of(vec![volume, Volume::new('E')]);
    let filter = Filter { keyword: "w".into(), folder: "d:\\work".into(), ..Filter::default() };
    let query = Query::parse_with("plan", &grammar(), Some(filter.rule())).expect("query");
    let found: Vec<String> =
        index.search(&query, &Boosts::default(), &Focus::Anything, 10).hits.into_iter().map(|hit| hit.path).collect();
    assert_eq!(found, ["D:\\Work\\plan.txt"]);
}

#[test]
fn facets_narrow_results_and_count_each_choice_against_the_other() {
    let mut builder = Builder::new('C', ROOT_ID);
    let file = |id, name, modified| volume::Record {
        id,
        parent: ROOT_ID,
        name,
        dir: false,
        hidden: false,
        meta: volume::Meta { size: 1, modified },
    };
    builder.push_record(file(11, "trip-new.jpg", 2_000));
    builder.push_record(file(12, "trip-old.jpg", 100));
    builder.push_record(file(13, "trip-new.pdf", 2_000));
    builder.push_record(file(14, "trip-old.pdf", 100));
    builder.push_record(file(15, "trip-new.txt", 2_000));
    let index = index_of(vec![builder.finish(None, true)]);
    let facets = Facets {
        kinds: vec![Filter::builtin(Builtin::Images).rule(), Filter::builtin(Builtin::Documents).rule()],
        dates: vec![1_000..=i64::MAX, i64::MIN..=999],
        chosen: Facets::mask([0]),
        date: Some(0),
    };
    let (results, counts) = index.search_with(&parse("trip"), &Boosts::default(), &Focus::Anything, 10, &facets);
    let paths: Vec<String> = results.hits.into_iter().map(|hit| hit.path).collect();
    assert_eq!(paths, ["C:\\trip-new.jpg"]);
    assert_eq!(results.total, 1);
    assert_eq!(counts, Counts { kinds: vec![1, 2], dates: vec![1, 1], any_kind: 3, any_date: 2 });
    let both = Facets { chosen: Facets::mask([0, 1]), ..facets };
    let (results, counts) = index.search_with(&parse("trip"), &Boosts::default(), &Focus::Anything, 10, &both);
    let mut paths: Vec<String> = results.hits.into_iter().map(|hit| hit.path).collect();
    paths.sort();
    assert_eq!(paths, ["C:\\trip-new.jpg", "C:\\trip-new.pdf", "C:\\trip-new.txt"]);
    assert_eq!(counts, Counts { kinds: vec![1, 2], dates: vec![3, 2], any_kind: 3, any_date: 5 });
}

const CALCULATOR: &str = "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App";
const PHOTOS: &str = "shell:AppsFolder\\Microsoft.Windows.Photos_8wekyb3d8bbwe!App";

fn with_store_apps(volumes: Vec<Volume>) -> Index {
    let index = index_of(volumes);
    index.list_apps(vec![
        StoreApp { name: "Калькулятор".into(), path: CALCULATOR.into() },
        StoreApp { name: "Photos".into(), path: PHOTOS.into() },
    ]);
    index
}

#[test]
fn store_apps_are_found_by_name_in_the_launcher_only() {
    let mut volume = Volume::new('C');
    volume.insert(ROOT, "photos", true);
    let index = with_store_apps(vec![volume]);
    let hits = index.search(&parse("КАЛЬК"), &Boosts::default(), &Focus::Programs, 10).hits;
    let found: Vec<(&str, &str)> = hits.iter().map(|hit| (hit.path.as_str(), hit.name())).collect();
    assert_eq!(found, [(CALCULATOR, "Калькулятор")]);
    let highlights: Vec<(usize, usize)> =
        hits.iter().flat_map(|hit| &hit.highlights).map(|range| (range.start, range.end)).collect();
    assert_eq!(highlights, [(0, "кальк".len())]);
    assert_eq!(first(&index, "rfkmr").as_deref(), Some(CALCULATOR));
    assert_eq!(first(&index, "photos").as_deref(), Some(PHOTOS));
    assert_eq!(search_with(&index, "photos", &Boosts::default(), Focus::Anything), ["C:\\photos"]);
    assert_eq!(search_with(&index, "photos ext:jpg", &Boosts::default(), Focus::Programs), Vec::<String>::new());
}

#[test]
fn store_apps_rank_with_shortcuts_and_history() {
    let mut volume = Volume::new('C');
    let start = volume.insert(ROOT, "Start Menu", true);
    let programs = volume.insert(start, "Programs", true);
    volume.insert(programs, "Photoshop.lnk", false);
    let index = with_store_apps(vec![volume]);
    let shortcut = "C:\\Start Menu\\Programs\\Photoshop.lnk";
    assert_eq!(search_with(&index, "photo", &Boosts::default(), Focus::Programs), [PHOTOS, shortcut]);
    let boosts = Boosts::from_iter([(shortcut.to_lowercase(), 50)]);
    assert_eq!(search_with(&index, "photo", &boosts, Focus::Programs), [shortcut, PHOTOS]);
    let boosts = Boosts::from_iter([(PHOTOS.to_lowercase(), 50)]);
    assert_eq!(search_with(&index, "photoshop", &boosts, Focus::Programs), [shortcut]);
}

#[test]
fn store_apps_count_as_apps_in_the_facets() {
    let index = with_store_apps(Vec::new());
    let facets = |kind| Facets {
        kinds: vec![Filter::builtin(Builtin::Apps).rule(), Filter::builtin(Builtin::Images).rule()],
        dates: Vec::new(),
        chosen: Facets::mask([kind]),
        date: None,
    };
    let (apps, counts) = index.search_with(&parse("photos"), &Boosts::default(), &Focus::Programs, 10, &facets(0));
    assert_eq!(apps.hits.iter().map(|hit| hit.path.as_str()).collect::<Vec<_>>(), [PHOTOS]);
    assert_eq!(counts.kinds, [1, 0]);
    let (images, _) = index.search_with(&parse("photos"), &Boosts::default(), &Focus::Programs, 10, &facets(1));
    assert_eq!(images.total, 0);
}

#[test]
fn diagnosis_tells_why_a_path_is_not_found() {
    let mut volume = Volume::new('C');
    let work = volume.insert(ROOT, "Work", true);
    volume.insert(work, "отчёт.txt", false);
    volume.insert_entry(work, "secret.txt", false, true, Meta::default());
    let old = volume.insert(work, "Old", true);
    volume.insert(old, "plan.txt", false);
    let index = index_of(vec![volume]);
    index.rank(Ranking::new(&["C:\\Work\\Old".to_owned()], &[], &["c:/work".to_owned()], true));
    let plain = Diagnosis::Indexed { hidden: false, low: true };
    assert_eq!(index.diagnose("C:\\Work\\отчёт.txt", true), plain);
    assert_eq!(index.diagnose(" \"c:/WORK/ОТЧЁТ.txt\" ", true), plain);
    assert_eq!(index.diagnose("C:\\Work\\secret.txt", true), Diagnosis::Indexed { hidden: true, low: true });
    assert_eq!(index.diagnose("C:\\Work\\отчёт.txt", false), Diagnosis::Gone);
    assert_eq!(index.diagnose("C:\\Work\\new.txt", false), Diagnosis::Missing);
    assert_eq!(index.diagnose("C:\\Work\\new.txt", true), Diagnosis::Unseen { live: false });
    assert_eq!(index.diagnose("C:\\Work\\Old\\plan.txt", true), Diagnosis::Excluded("c:\\work\\old".to_owned()));
    assert_eq!(index.diagnose("C:\\Windows\\WinSxS\\amd64\\a.dll", true), Diagnosis::System);
    assert_eq!(index.diagnose("C:\\System Volume Information\\a", true), Diagnosis::System);
    assert_eq!(index.diagnose("C:\\$Recycle.Bin\\S-1-5\\$R1.txt", true), Diagnosis::Recycled);
    assert_eq!(index.diagnose("C:\\Work\\WinSxS\\a.dll", true), Diagnosis::Unseen { live: false });
    assert_eq!(index.diagnose("D:\\a.txt", true), Diagnosis::Uncovered);
    assert_eq!(index.diagnose("D:\\a.txt", false), Diagnosis::Missing);
}

const SETTLE: Duration = Duration::from_secs(10);

struct WalkedFolder {
    root: PathBuf,
    index: Arc<Index>,
    updates: mpsc::Receiver<Update>,
}

impl WalkedFolder {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("vestigo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test folder");
        std::fs::write(root.join("first.txt"), b"x").expect("test file");
        let index = Arc::new(Index::default());
        let (sender, updates) = mpsc::channel();
        index.start(Vec::new(), vec![root.to_str().expect("utf-8 path").to_owned()], move |update| {
            let _ = sender.send(update);
        });
        let folder = Self { root, index, updates };
        folder.settle(|index| search(index, "first.txt").len() == 1);
        folder
    }

    fn path(&self) -> &str {
        self.root.to_str().expect("utf-8 path")
    }

    fn settle(&self, done: impl Fn(&Index) -> bool) -> bool {
        let deadline = Instant::now() + SETTLE;
        while !done(&self.index) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { return false };
            let _ = self.updates.recv_timeout(left);
        }
        true
    }
}

impl Drop for WalkedFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn refreshing_a_folder_walks_it_again_when_it_is_never_rewalked() {
    let folder = WalkedFolder::new("refresh");
    std::fs::write(folder.root.join("second.txt"), b"x").expect("test file");
    assert!(search(&folder.index, "second.txt").is_empty());
    folder.index.refresh_folder(folder.path());
    assert!(folder.settle(|index| search(index, "second.txt").len() == 1));
    let status = folder.index.folders();
    assert_eq!((status.len(), status[0].entries, status[0].walking), (1, 2, false));
}

#[test]
fn a_new_rewalk_interval_applies_to_a_waiting_folder() {
    let folder = WalkedFolder::new("rewalk");
    std::fs::write(folder.root.join("second.txt"), b"x").expect("test file");
    folder.index.rewalk_every(Some(Duration::from_millis(1)));
    assert!(folder.settle(|index| search(index, "second.txt").len() == 1));
}
