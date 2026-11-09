use super::*;

const NOW: u64 = 10_000_000;
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

const FOLDERS: [&str; 8] = [
    "C:\\Docs",
    "D:\\Работа\\Финал",
    "C:\\Users\\Me\\Downloads",
    "E:\\Фото",
    "C:\\Old",
    "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Visual Studio 2022\\Visual Studio Tools",
    "C:\\Program Files\\App",
    "D:\\Работа",
];
const FILES: [&str; 9] = [
    "D:\\Работа\\Финал\\обложка.PNG",
    "C:\\Users\\Me\\Downloads\\фото.jpg",
    "C:\\Users\\Me\\Downloads\\отчёт.pdf",
    "C:\\Users\\Me\\Downloads\\видео.png.crdownload",
    "C:\\Users\\Me\\Downloads\\старое.png",
    "E:\\Фото\\море.jpg",
    "C:\\Program Files\\App\\app.exe",
    "C:\\Users\\Me\\Desktop\\App.lnk",
    "C:\\Old\\план.png",
];

fn kind(path: &str) -> Option<bool> {
    if FOLDERS.contains(&path) {
        Some(true)
    } else if FILES.contains(&path) {
        Some(false)
    } else {
        None
    }
}

fn visit(path: &str, dir: bool, ago: u64) -> Visit {
    Visit { path: path.into(), dir, at: NOW - ago }
}

fn situation(purpose: Purpose) -> Situation {
    Situation {
        purpose,
        current: Some("C:\\Docs".into()),
        came_from: None,
        selection: Vec::new(),
        explorer: Vec::new(),
        fresh: Vec::new(),
        visits: Vec::new(),
        picked: Vec::new(),
        now: NOW,
    }
}

fn images() -> Types {
    Types::Only(vec!["png".into(), "jpg".into()])
}

fn suggested(situation: &Situation) -> Vec<(String, bool)> {
    situation.rank(kind).into_iter().map(|hit| (hit.path, hit.dir)).collect()
}

fn folder(path: &str) -> (String, bool) {
    (path.into(), true)
}

fn file(path: &str) -> (String, bool) {
    (path.into(), false)
}

#[test]
fn program_launches_and_start_menu_folders_are_never_suggested() {
    let tools = "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Visual Studio 2022\\Visual Studio Tools";
    let situation = Situation {
        visits: vec![
            visit(tools, true, 10 * MINUTE),
            visit(tools, true, HOUR),
            visit("C:\\Program Files\\App\\app.exe", false, MINUTE),
            visit("C:\\Program Files\\App\\app.exe", false, 2 * MINUTE),
            visit("C:\\Users\\Me\\Desktop\\App.lnk", false, MINUTE),
        ],
        ..situation(Purpose::Open(Types::Any))
    };
    assert!(suggested(&situation).is_empty());
}

#[test]
fn a_save_dialog_offers_the_explorer_folder_the_user_came_from_first() {
    let situation = Situation {
        came_from: Some("D:\\Работа\\Финал".into()),
        selection: vec!["D:\\Работа\\Финал\\обложка.PNG".into()],
        explorer: vec!["E:\\Фото".into(), "D:\\Работа\\Финал".into(), "C:\\Docs".into(), "C:\\Old".into()],
        ..situation(Purpose::Save(Types::Any))
    };
    assert_eq!(suggested(&situation), [folder("D:\\Работа\\Финал"), folder("E:\\Фото")]);
}

#[test]
fn the_folder_the_dialog_already_shows_is_never_suggested() {
    let situation = Situation {
        came_from: Some("c:\\docs\\".into()),
        explorer: vec!["C:\\Docs".into()],
        ..situation(Purpose::Save(Types::Any))
    };
    assert!(suggested(&situation).is_empty());
}

#[test]
fn an_open_dialog_offers_the_selected_file_its_folder_and_a_fresh_download_of_its_type() {
    let situation = Situation {
        came_from: Some("D:\\Работа\\Финал".into()),
        selection: vec!["D:\\Работа\\Финал\\обложка.PNG".into()],
        fresh: vec![
            visit("C:\\Users\\Me\\Downloads\\видео.png.crdownload", false, 10),
            visit("C:\\Users\\Me\\Downloads\\отчёт.pdf", false, 20),
            visit("C:\\Users\\Me\\Downloads\\фото.jpg", false, MINUTE),
            visit("C:\\Users\\Me\\Downloads\\старое.png", false, 3 * HOUR),
        ],
        ..situation(Purpose::Open(images()))
    };
    assert_eq!(
        suggested(&situation),
        [
            file("D:\\Работа\\Финал\\обложка.PNG"),
            folder("D:\\Работа\\Финал"),
            file("C:\\Users\\Me\\Downloads\\фото.jpg"),
        ]
    );
}

#[test]
fn a_file_that_arrived_an_hour_ago_alone_is_not_enough() {
    let situation = Situation {
        fresh: vec![visit("C:\\Users\\Me\\Downloads\\фото.jpg", false, HOUR)],
        ..situation(Purpose::Open(images()))
    };
    assert!(suggested(&situation).is_empty());
}

#[test]
fn a_single_visit_counts_only_while_it_is_part_of_the_current_session() {
    let recent = Situation { visits: vec![visit("C:\\Old", true, 30 * MINUTE)], ..situation(Purpose::Browse) };
    assert_eq!(suggested(&recent), [folder("C:\\Old")]);
    let earlier = Situation { visits: vec![visit("C:\\Old", true, 3 * HOUR)], ..situation(Purpose::Browse) };
    assert!(suggested(&earlier).is_empty());
}

#[test]
fn one_old_visit_is_not_enough_but_a_habit_of_this_week_is() {
    let situation = Situation {
        visits: vec![
            visit("C:\\Old", true, 3 * DAY),
            visit("E:\\Фото\\море.jpg", false, DAY),
            visit("E:\\Фото\\море.jpg", false, 2 * DAY),
            visit("E:\\Фото\\море.jpg", false, 3 * DAY),
            visit("E:\\Фото\\море.jpg", false, 4 * DAY),
        ],
        ..situation(Purpose::Save(Types::Any))
    };
    assert_eq!(suggested(&situation), [folder("E:\\Фото")]);
}

#[test]
fn a_save_dialog_offers_where_files_of_its_type_were_recently_picked() {
    let situation = Situation {
        picked: vec![
            Picked {
                paths: vec!["D:\\Работа\\Финал\\обложка.PNG".into(), "E:\\Фото\\море.jpg".into()],
                at: NOW - HOUR,
            },
            Picked { paths: vec!["C:\\Old\\план.png".into()], at: NOW - 90 * DAY },
        ],
        ..situation(Purpose::Save(images()))
    };
    assert_eq!(suggested(&situation), [folder("D:\\Работа\\Финал"), folder("E:\\Фото")]);
}

#[test]
fn a_folder_visited_for_other_work_is_not_enough_for_a_typed_dialog() {
    let situation = Situation { visits: vec![visit("C:\\Old", true, MINUTE)], ..situation(Purpose::Save(images())) };
    assert!(suggested(&situation).is_empty());
    let untyped = Situation { purpose: Purpose::Save(Types::Any), ..situation };
    assert_eq!(suggested(&untyped), [folder("C:\\Old")]);
}

#[test]
fn a_file_of_another_type_says_nothing_about_an_open_dialog() {
    let situation = Situation {
        visits: vec![visit("C:\\Users\\Me\\Downloads\\отчёт.pdf", false, MINUTE)],
        picked: vec![Picked { paths: vec!["C:\\Users\\Me\\Downloads\\отчёт.pdf".into()], at: NOW }],
        ..situation(Purpose::Open(images()))
    };
    assert!(suggested(&situation).is_empty());
}

#[test]
fn no_more_than_three_suggestions_with_the_explorer_the_user_came_from_first() {
    let situation = Situation {
        came_from: Some("D:\\Работа\\Финал".into()),
        explorer: vec!["E:\\Фото".into(), "C:\\Old".into(), "D:\\Работа".into()],
        visits: vec![visit("C:\\Old", true, MINUTE), visit("D:\\Работа", true, 30 * MINUTE)],
        ..situation(Purpose::Browse)
    };
    assert_eq!(suggested(&situation), [folder("D:\\Работа\\Финал"), folder("C:\\Old"), folder("D:\\Работа")]);
}
