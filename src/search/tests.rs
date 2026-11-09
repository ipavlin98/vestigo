use super::*;

const CLOCK: Clock = Clock { now: 1_784_116_800, offset: 0 };

fn grammar() -> Grammar {
    Grammar::new(&Filter::defaults(), CLOCK)
}

fn query(text: &str) -> Query {
    Query::parse(text, &grammar()).expect("query is not empty")
}

fn strict(query: &Query, name: &str, dir: bool) -> Option<i32> {
    query
        .score_name(Name::new(name.as_bytes(), &name.to_lowercase(), name.is_ascii()), dir, Tolerance::Strict)
        .map(|scored| scored.with_focus(0))
}

fn loose(query: &Query, name: &str, dir: bool) -> Option<i32> {
    query
        .score_name(Name::new(name.as_bytes(), &name.to_lowercase(), name.is_ascii()), dir, Tolerance::Loose)
        .map(|scored| scored.with_focus(0))
}

#[test]
fn abbreviations_match_word_starts_and_camel_case() {
    let query = query("vsc");
    assert!(strict(&query, "Visual Studio Code.lnk", false).is_some());
    assert!(strict(&query, "VisualStudioCode.exe", false).is_some());
    assert!(strict(&query, "vs.code", false).is_none());
    assert!(strict(&self::query("notpad"), "no-unnecessary-type-parameters.d.ts", false).is_none());
    assert!(strict(&self::query("vscode"), "Visual Studio Code.lnk", false).is_some());
    assert!(strict(&self::query("ps"), "PowerShell.lnk", false).is_some());
    assert!(strict(&self::query("мк"), "Мой компьютер", true).is_some());
}

#[test]
fn whole_words_rank_above_abbreviations() {
    let query = query("vsc");
    let prefix = strict(&query, "vsc-notes.txt", false).expect("prefix");
    let abbreviation = strict(&query, "Visual Studio Code.lnk", false).expect("abbreviation");
    let inside = strict(&query, "rvsc.txt", false).expect("inside");
    assert!(prefix > abbreviation && abbreviation > inside);
}

#[test]
fn abbreviations_highlight_each_word_start() {
    assert_eq!(query("vsc").highlights("Visual Studio Code.lnk"), [0..1, 7..8, 14..15]);
    assert_eq!(query("vscode").highlights("VisualStudioCode.exe"), [0..1, 6..7, 12..16]);
}

#[test]
fn typos_match_only_close_entries() {
    assert!(strict(&query("noetpad"), "Notepad++.lnk", false).is_none());
    for text in ["notpad", "noetpad", "ntpd"] {
        assert!(loose(&query(text), "Notepad++.lnk", false).is_some(), "{text}");
    }
    assert!(loose(&query("visaul"), "Visual Studio Code.lnk", false).is_some());
    assert!(loose(&query("xyzpad"), "Notepad++.lnk", false).is_none());
    assert!(loose(&query("teh"), "the.txt", false).is_none());
}

#[test]
fn letters_in_order_match_from_a_word_start() {
    let query = query("ctron");
    assert!(strict(&query, "citron.exe", false).is_some());
    assert!(strict(&query, "my-citron-build", true).is_some());
    assert!(strict(&self::query("vsc"), "vascular.txt", false).is_some());
    assert!(strict(&self::query("ntpd"), "Notepad++.lnk", false).is_some());
    assert!(strict(&self::query("tcron"), "citron.exe", false).is_none());
    assert!(strict(&self::query("itrn"), "citron.exe", false).is_none());
    assert!(strict(&query, "FanControl.lnk", false).is_none());
    assert!(strict(&self::query("ct"), "citron.exe", false).is_none());
}

#[test]
fn fewer_breaks_between_letters_rank_higher() {
    let query = query("ctron");
    let together = strict(&query, "cxtronx", false).expect("one break");
    let apart = strict(&query, "cxtxron", false).expect("two breaks");
    assert!(together > apart);
}

#[test]
fn letters_rank_below_abbreviations_and_word_starts() {
    let query = query("vsc");
    let abbreviation = strict(&query, "Visual Studio Code.lnk", false).expect("abbreviation");
    let letters = strict(&query, "vascular.lnk", false).expect("letters");
    assert!(abbreviation > letters);
    let word = strict(&self::query("tron"), "my tron.txt", false).expect("word start");
    let letters = strict(&self::query("tron"), "taron.txt", false).expect("letters");
    assert!(word > letters);
}

#[test]
fn letters_highlight_each_run() {
    assert_eq!(query("ctron").highlights("Citron.exe"), [0..1, 2..6]);
    assert_eq!(query("ctron").highlights("cat-citron"), [4..5, 6..10]);
    assert_eq!(query("notpad").highlights("Notepad.exe"), [0..3, 4..7]);
}

#[test]
fn exact_letters_rank_above_typos() {
    let query = query("notpad");
    let typo = loose(&query, "Notepad.txt", false).expect("typo");
    let substring = loose(&query, "my notpad notes.txt", false).expect("substring");
    assert!(substring > typo);
}

#[test]
fn typos_highlight_the_closest_part() {
    assert_eq!(query("notapad").highlights("Notepad.exe"), vec![0..7]);
    assert_eq!(query("ntpd").highlights("Notepad"), [0..1, 2..3, 4..5, 6..7]);
}

#[test]
fn blank_input_is_not_a_query() {
    assert!(Query::parse("   ", &grammar()).is_none());
}

#[test]
fn every_term_must_match() {
    let query = query("rep pdf");
    assert!(strict(&query, "report.pdf", false).is_some());
    assert!(strict(&query, "report.docx", false).is_none());
}

#[test]
fn prefix_beats_word_start_beats_inside() {
    let query = query("note");
    let prefix = strict(&query, "notes.txt", false).expect("prefix");
    let word = strict(&query, "my notes.txt", false).expect("word start");
    let inside = strict(&query, "keynotes.txt", false).expect("inside");
    assert!(prefix > word && word > inside);
}

#[test]
fn exact_stem_wins_over_longer_names() {
    let query = query("claude");
    let exact = strict(&query, "claude.lnk", false).expect("exact");
    let longer = strict(&query, "claude-code", true).expect("longer");
    assert!(exact > longer);
}

#[test]
fn highlights_map_back_to_original_case() {
    let query = query("cla md");
    assert_eq!(query.highlights("CLAUDE.md"), [0..3, 7..9]);
}

#[test]
fn highlights_handle_multibyte_names() {
    let query = query("отч");
    assert_eq!(query.highlights("Мой Отчёт"), vec![7..13]);
}

#[test]
fn overlapping_highlights_merge() {
    let query = query("abc bcd");
    assert_eq!(query.highlights("xabcdx"), vec![1..5]);
}

#[test]
fn a_query_typed_in_the_other_layout_still_matches() {
    assert!(strict(&query("куфвьу"), "readme.md", false).is_some());
    assert!(strict(&query("jnxtn"), "отчет.docx", false).is_some());
    assert!(strict(&query("куфвьу md"), "readme.md", false).is_some());
    assert!(strict(&query("{jkjl"), "холод.txt", false).is_some());
    assert!(strict(&query("куфвьу"), "license.md", false).is_none());
}

#[test]
fn the_typed_layout_ranks_above_the_other_layout() {
    let query = query("куфвьу");
    let typed = strict(&query, "куфвьу-старый-длинный-вариант.txt", false).expect("typed");
    let swapped = strict(&query, "readme", true).expect("swapped");
    assert!(typed > swapped);
}

#[test]
fn highlights_follow_the_layout_that_matched() {
    assert_eq!(query("куфвьу").highlights("README.md"), vec![0..6]);
    assert_eq!(query("ыуе").highlights("Setup Сет.exe"), vec![0..3]);
}

#[test]
fn path_words_match_in_the_other_layout() {
    let query = query("цщкл\\");
    assert!(query.paths_within("d:\\work\\"));
    assert!(!query.paths_within("d:\\home\\"));
}

fn rule(builtin: Builtin) -> Option<Rule> {
    Some(Filter::builtin(builtin).rule())
}

#[test]
fn kind_filters_by_extension_and_folder() {
    let images = Query::parse_with("photo", &grammar(), rule(Builtin::Images)).expect("query");
    assert!(strict(&images, "photo.jpeg", false).is_some());
    assert!(strict(&images, "photo.pdf", false).is_none());
    assert!(strict(&images, "photo.png", true).is_none());
    let folders = Query::parse_with("photo", &grammar(), rule(Builtin::Folders)).expect("query");
    assert!(strict(&folders, "photos", true).is_some());
    assert!(strict(&folders, "photos", false).is_none());
    let apps = Query::parse_with("tool", &grammar(), rule(Builtin::Apps)).expect("query");
    assert!(strict(&apps, "tool.appref-ms", false).is_some());
    assert!(strict(&apps, "tool.exe.txt", false).is_none());
}

#[test]
fn kind_without_words_lists_every_file_of_that_kind() {
    assert!(Query::parse_with("  ", &grammar(), None).is_none());
    let audio = Query::parse_with("  ", &grammar(), rule(Builtin::Audio)).expect("a kind alone is a query");
    assert!(strict(&audio, "track.flac", false).is_some());
    assert!(strict(&audio, "track.mkv", false).is_none());
}

#[test]
fn kind_masks_agree_with_each_rule() {
    let mut filters = Filter::defaults();
    filters.push(Filter { extensions: "tar.gz".into(), ..Filter::default() });
    filters.push(Filter { folder: "D:\\Work".into(), ..Filter::default() });
    let rules: Vec<Rule> = filters.iter().map(Filter::rule).collect();
    let kinds = Kinds::new(&rules);
    let names = [
        ("photo.JPG", false),
        ("photo.jpg", true),
        ("Projects", true),
        ("backup.tar.gz", false),
        ("tar.gz", false),
        (".png", false),
        ("notes.pdf.png", false),
        ("Отчёт.DOCX", false),
        ("readme", false),
    ];
    for (name, dir) in names {
        let lower = name.to_lowercase();
        let mask = kinds.of(&lower, dir);
        for (position, rule) in rules.iter().enumerate() {
            assert_eq!(mask & 1 << position != 0, rule.accepts(&lower, dir), "{name} (dir: {dir}), rule {position}");
        }
    }
}

#[test]
fn quoted_phrases_match_exactly_with_spaces() {
    let query = query(r#""q3 plan""#);
    assert!(strict(&query, "Budget Q3 Plan.xlsx", false).is_some());
    assert!(loose(&query, "q3-plan.xlsx", false).is_none());
    assert!(loose(&query, "q3 plna.xlsx", false).is_none());
    assert_eq!(query.highlights("Budget Q3 Plan.xlsx"), [std::ops::Range { start: 7, end: 14 }]);
}

#[test]
fn an_unclosed_quote_runs_to_the_end() {
    let query = query(r#"report "final v"#);
    assert!(strict(&query, "report final v2.pdf", false).is_some());
    assert!(strict(&query, "report final.pdf", false).is_none());
}

#[test]
fn minus_excludes_words_and_phrases() {
    let query = query(r#"report -draft -"old version""#);
    assert!(strict(&query, "report.pdf", false).is_some());
    assert!(strict(&query, "report DRAFT.pdf", false).is_none());
    assert!(strict(&query, "report old version.pdf", false).is_none());
    assert!(strict(&self::query("my-file"), "my-file.txt", false).is_some());
    let excluded_only = self::query("-tmp");
    assert!(strict(&excluded_only, "notes.txt", false).is_some());
    assert!(strict(&excluded_only, "notes.tmp", false).is_none());
}

#[test]
fn extensions_filter_files_and_combine_with_or() {
    let query = query("report ext:pdf;docx");
    assert!(strict(&query, "report.pdf", false).is_some());
    assert!(strict(&query, "report.DOCX", false).is_some());
    assert!(strict(&query, "report.txt", false).is_none());
    assert!(strict(&query, "report.pdf", true).is_none());
    assert!(strict(&self::query("*.pdf"), "anything.pdf", false).is_some());
    assert!(strict(&self::query("ext:tar.gz"), "backup.tar.gz", false).is_some());
    assert!(strict(&self::query("ext:tar.gz"), "tar.gz", false).is_none());
    let excluded = self::query("report -ext:tmp -*.bak");
    assert!(strict(&excluded, "report.txt", false).is_some());
    assert!(strict(&excluded, "report.tmp", false).is_none());
    assert!(strict(&excluded, "report.bak", false).is_none());
}

#[test]
fn filter_keywords_narrow_by_type() {
    let query = query("doc: report");
    assert!(strict(&query, "report.pdf", false).is_some());
    assert!(strict(&query, "report.png", false).is_none());
    let joined = self::query("pic:holiday");
    assert!(strict(&joined, "holiday.jpg", false).is_some());
    assert!(strict(&joined, "holiday.pdf", false).is_none());
    assert!(strict(&self::query("folder:"), "Projects", true).is_some());
    assert!(strict(&self::query("folder:"), "Projects.txt", false).is_none());
    let unknown = self::query("todo:list");
    assert!(strict(&unknown, "todo:list.md", false).is_some());
}

#[test]
fn disabled_and_custom_filters_follow_the_settings() {
    let mut filters = Filter::defaults();
    if let Some(documents) = filters.iter_mut().find(|filter| filter.builtin == Some(Builtin::Documents)) {
        documents.enabled = false;
    }
    filters.push(Filter {
        name: "Code".into(),
        keyword: "code".into(),
        extensions: "rs; .toml".into(),
        ..Filter::default()
    });
    let grammar = Grammar::new(&filters, CLOCK);
    let code = Query::parse("code: main", &grammar).expect("query");
    assert!(strict(&code, "main.rs", false).is_some());
    assert!(strict(&code, "main.py", false).is_none());
    let plain = Query::parse("doc:report", &grammar).expect("query");
    assert!(strict(&plain, "report.pdf", false).is_none());
}

#[test]
fn a_drive_path_is_not_a_filter_keyword() {
    let mut filters = Filter::defaults();
    filters.push(Filter { keyword: "d".into(), extensions: "zip".into(), ..Filter::default() });
    let query = Query::parse("d:\\photo", &Grammar::new(&filters, CLOCK)).expect("query");
    let place = query.place().expect("a drive path is a place");
    assert_eq!((place.anchor(), place.path()), (Some("d:"), vec!["photo"]));
    assert!(strict(&query, "sea.jpg", false).is_some());
}

#[test]
fn a_place_alone_keeps_its_last_typed_name_as_a_tail() {
    let typed = query("\\Sfs/Src");
    let place = typed.place().expect("place");
    assert_eq!((place.anchor(), place.parents(), place.path()), (None, vec!["sfs"], vec!["sfs", "src"]));
    assert!(matches!(place.reach(), Reach::Contents(Some(tail)) if tail.name == "src"));
    let open = query("\\sfs\\");
    let place = open.place().expect("place");
    assert_eq!(place.path(), ["sfs"]);
    assert!(matches!(place.reach(), Reach::Contents(None)));
    assert!(Query::parse("\\", &grammar()).is_none());
}

#[test]
fn a_tilde_stands_for_the_home_folder() {
    let home = Grammar::new(&Filter::defaults(), CLOCK).with_home(Some("C:\\Users\\Игорь".to_owned()));
    let desktop = Query::parse("~/Desktop/", &home).expect("query");
    let place = desktop.place().expect("place");
    assert_eq!((place.anchor(), place.path()), (Some("c:"), vec!["users", "игорь", "desktop"]));
    let alone = Query::parse("~", &home).expect("query");
    assert_eq!(alone.place().map(Place::path), Some(vec!["users", "игорь"]));
    assert!(Query::parse("~notes", &home).expect("query").place().is_none());
    assert_eq!(super::up("~\\Desktop\\"), Some(("~\\".to_owned(), Some("~\\Desktop".to_owned()))));
}

#[test]
fn a_place_with_words_reaches_its_whole_subtree() {
    let typed = query("\\sfs\\src main");
    let place = typed.place().expect("place");
    assert_eq!(place.path(), ["sfs", "src"]);
    assert!(matches!(place.reach(), Reach::Subtree));
    assert!(!typed.needs_full_path());
    assert!(strict(&typed, "main.rs", false).is_some());
    assert!(strict(&typed, "lib.rs", false).is_none());
}

#[test]
fn folders_named_by_the_query_are_marked_in_a_path() {
    let folder = "C:\\Users\\Игорь\\Desktop\\SFS\\src\\ui";
    let marked = |text: &str| -> Vec<&str> {
        query(text).marks(folder).into_iter().filter_map(|range| folder.get(range)).collect()
    };
    assert_eq!(marked("sfs\\main"), ["SFS"]);
    assert_eq!(marked("игорь\\sfs\\ main"), ["Игорь", "SFS"]);
    assert_eq!(marked("~\\desktop\\src main"), ["src"]);
    assert_eq!(marked("main"), Vec::<&str>::new());
}

#[test]
fn entering_a_folder_extends_the_typed_place() {
    let enter = |text, listed, path, name| super::enter(text, listed, path, name);
    assert_eq!(enter("\\sfs", Listed::Contents, "C:\\a\\sfs\\src", "src"), "\\sfs\\src\\");
    assert_eq!(enter("\\sfs\\", Listed::Contents, "C:\\a\\sfs\\src", "src"), "\\sfs\\src\\");
    assert_eq!(enter("C:\\", Listed::Contents, "C:\\Users", "Users"), "C:\\Users\\");
    assert_eq!(enter("\\sfs/sr", Listed::Matches, "C:\\a\\sfs\\src", "src"), "\\sfs\\src\\");
    assert_eq!(enter("\\sf", Listed::Matches, "C:\\a\\sfs", "sfs"), "\\sfs\\");
    assert_eq!(enter("\\SFS", Listed::Matches, "C:\\b\\sfs", "sfs"), "C:\\b\\sfs\\");
    assert_eq!(enter("sfs", Listed::Matches, "C:\\a\\sfs", "sfs"), "C:\\a\\sfs\\");
    assert_eq!(enter("\\sfs main", Listed::Matches, "C:\\a\\sfs\\src", "src"), "C:\\a\\sfs\\src\\");
}

#[test]
fn leaving_a_folder_returns_to_the_listing_it_was_entered_from() {
    assert_eq!(super::origin("\\sfs/sr", Listed::Matches, "src"), "\\sfs\\");
    assert_eq!(super::origin("C:\\Us", Listed::Matches, "Users"), "C:\\");
    assert_eq!(super::origin("\\sf", Listed::Matches, "sfs"), "\\sf");
    assert_eq!(super::origin("\\sfs", Listed::Matches, "sfs"), "\\sfs");
    assert_eq!(super::origin("\\sfs\\src", Listed::Contents, "index"), "\\sfs\\src");
    assert_eq!(super::origin(" sublime ", Listed::Matches, "sublime"), "sublime");
}

#[test]
fn custom_filters_can_be_limited_to_a_folder() {
    let filter = Filter { keyword: "work".into(), folder: "D:/Work/".into(), ..Filter::default() };
    let query = Query::parse_with("", &grammar(), Some(filter.rule())).expect("a folder filter is a query");
    assert_eq!(query.folders().collect::<Vec<_>>(), ["d:\\work"]);
    assert!(strict(&query, "anything.txt", false).is_some());
}

#[test]
fn dates_and_sizes_need_details_and_skip_folder_sizes() {
    let query = query("size:>1mb date:2026");
    assert!(query.needs_details());
    let in_2026 = 1_780_000_000;
    assert!(query.accepts_details(false, Some((2 << 20, in_2026))));
    assert!(!query.accepts_details(false, Some((1 << 20, in_2026))));
    assert!(!query.accepts_details(false, Some((2 << 20, 1_700_000_000))));
    assert!(!query.accepts_details(true, Some((2 << 20, in_2026))));
    assert!(!query.accepts_details(false, None));
    assert!(self::query("report").accepts_details(false, None));
    assert!(!self::query("report").needs_details());
    assert!(Query::parse("date:someday", &grammar()).is_none());
    assert!(self::query("report size:huge").accepts_details(false, Some((0, 0))));
}

#[test]
fn several_dates_narrow_each_other() {
    let query = query("date:2026 date:2026-07");
    assert!(query.accepts_details(false, Some((0, 1_783_000_000))));
    assert!(!query.accepts_details(false, Some((0, 1_770_000_000))));
}

fn content_terms(text: &str) -> (Vec<String>, Vec<String>) {
    let query = self::query(text);
    let content = query.content().expect("a content query");
    (content.terms().to_vec(), content.excluded().to_vec())
}

#[test]
fn full_sends_the_rest_of_the_query_to_the_file_contents() {
    assert_eq!(content_terms("full:Договор Аренды"), (vec!["договор".to_owned(), "аренды".to_owned()], Vec::new()));
    assert_eq!(content_terms("отчёт full:аренда").0, ["аренда"]);
    assert!(self::query("отчёт full:аренда").scores_names());
    assert!(!self::query("full:аренда").scores_names());
}

#[test]
fn full_keeps_phrases_and_exclusions_for_the_contents() {
    assert_eq!(content_terms("full:\"Договор   аренды\"").0, ["договор аренды"]);
    assert_eq!(content_terms("full:договор -аренды"), (vec!["договор".to_owned()], vec!["аренды".to_owned()]));
    assert_eq!(
        content_terms("full:-\"старый вариант\" слово"),
        (vec!["слово".to_owned()], vec!["старый вариант".to_owned()])
    );
    assert_eq!(content_terms("full:and/or").0, ["and/or"]);
}

#[test]
fn operators_after_full_stay_operators() {
    let query = self::query("full:договор ext:pdf size:>1mb");
    assert_eq!(query.content().expect("content").terms(), ["договор"]);
    assert_eq!(query.extensions, [".pdf"]);
    assert!(query.needs_details());
    assert!(self::query("full:договор \\work\\docs").place().is_some());
}

#[test]
fn an_empty_or_short_full_is_a_query_that_cannot_search_yet() {
    let empty = self::query("full:");
    assert!(!empty.content().expect("content").searchable());
    let short = self::query("full:да");
    assert!(!short.content().expect("content").searchable());
    let mixed = self::query("full:да договор");
    assert!(mixed.content().expect("content").searchable());
    assert!(self::query("-full:слово").content().is_none());
}
