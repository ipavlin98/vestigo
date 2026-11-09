use super::load::{Facts, Kind, Loaded, TEXT_BYTES, Visual, kind, load, picture, read_text};
use super::queue::{Job, KEPT, Kept, Key, Need, Queue};
use crate::index::Hit;
use std::path::PathBuf;

fn scratch(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("vestigo-preview-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("scratch file");
    path
}

fn read(name: &str, bytes: &[u8]) -> Option<String> {
    let path = scratch(name, bytes);
    let text = read_text(&path.to_string_lossy());
    let _ = std::fs::remove_file(&path);
    text
}

fn job(path: &str, need: Need) -> Job {
    let hit = Hit::new(path.to_owned(), false);
    Job { key: Key::of(&hit, 64), hit, ratio: 1.0, need }
}

fn started(queue: &mut Queue) -> Vec<String> {
    std::iter::from_fn(|| queue.next()).map(|job| job.hit.path).collect()
}

#[test]
fn text_drops_the_byte_order_mark_and_expands_tabs() {
    assert_eq!(
        read("bom.txt", "\u{feff}fn main()\r\n\tделать();".as_bytes()).as_deref(),
        Some("fn main()\n    делать();")
    );
}

#[test]
fn binary_files_have_no_text() {
    assert_eq!(read("binary.txt", &[b'M', b'Z', 0, 1, 2]), None);
}

#[test]
fn a_character_cut_by_the_byte_limit_is_dropped() {
    let mut bytes = vec![b'a'; TEXT_BYTES as usize - 1];
    bytes.extend("я".as_bytes());
    let text = read("cut.txt", &bytes).expect("text");
    assert_eq!(text.len(), TEXT_BYTES as usize - 1);
    assert!(text.chars().all(|ch| ch == 'a'));
}

#[test]
fn kept_previews_follow_changes_to_the_file_and_forget_the_oldest() {
    let key = |path: &str, modified| {
        let mut hit = Hit::new(path.to_owned(), false);
        hit.modified = Some(modified);
        Key::of(&hit, 64)
    };
    let loaded = || Loaded { visual: Visual::Text("text".into()), facts: Facts::default() };
    let mut kept = Kept::default();
    kept.keep(key("C:\\a.txt", 1), loaded());
    assert!(kept.get(&key("C:\\a.txt", 1)).is_some());
    assert!(kept.get(&key("C:\\a.txt", 2)).is_none());
    for number in 0..KEPT {
        kept.keep(key(&format!("C:\\{number}.txt"), 1), loaded());
    }
    assert!(kept.get(&key("C:\\a.txt", 1)).is_none());
    assert!(kept.get(&key("C:\\0.txt", 1)).is_some());
}

#[test]
fn the_shown_file_starts_loading_while_its_neighbours_wait_for_room() {
    let mut queue = Queue::default();
    queue.want(vec![job("C:\\a.mp4", Need::Shown), job("C:\\b.mp4", Need::Ahead), job("C:\\c.mp4", Need::Ahead)]);
    assert_eq!(started(&mut queue), ["C:\\a.mp4", "C:\\b.mp4"]);
    queue.want(vec![job("C:\\d.mp4", Need::Shown), job("C:\\e.mp4", Need::Ahead)]);
    assert_eq!(started(&mut queue), ["C:\\d.mp4"]);
}

#[test]
fn a_file_already_loading_is_not_loaded_again() {
    let mut queue = Queue::default();
    queue.want(vec![job("C:\\a.mp4", Need::Shown), job("C:\\b.mp4", Need::Ahead)]);
    assert_eq!(started(&mut queue), ["C:\\a.mp4", "C:\\b.mp4"]);
    queue.want(vec![job("C:\\b.mp4", Need::Shown), job("C:\\a.mp4", Need::Ahead), job("C:\\c.mp4", Need::Ahead)]);
    assert_eq!(started(&mut queue), Vec::<String>::new());
    queue.done(&job("C:\\a.mp4", Need::Ahead).key);
    assert_eq!(started(&mut queue), ["C:\\c.mp4"]);
}

#[test]
fn files_passed_by_never_start_and_a_busy_queue_frees_up() {
    let mut queue = Queue::default();
    for path in ["C:\\1.mp4", "C:\\2.mp4", "C:\\3.mp4", "C:\\4.mp4"] {
        queue.want(vec![job(path, Need::Shown)]);
        assert_eq!(started(&mut queue), [path]);
    }
    queue.want(vec![job("C:\\passed.mp4", Need::Shown), job("C:\\passed-too.mp4", Need::Ahead)]);
    assert_eq!(started(&mut queue), Vec::<String>::new());
    queue.want(vec![job("C:\\5.mp4", Need::Shown)]);
    queue.done(&job("C:\\1.mp4", Need::Shown).key);
    assert_eq!(started(&mut queue), ["C:\\5.mp4"]);
}

#[test]
fn kinds_follow_the_extension() {
    assert_eq!(kind(&Hit::new("C:\\Фото\\Пляж.JPG".into(), false)), Kind::Image);
    assert_eq!(kind(&Hit::new("C:\\src\\main.rs".into(), false)), Kind::Text);
    assert_eq!(kind(&Hit::new("C:\\notes.txt".into(), true)), Kind::Other);
    assert_eq!(kind(&Hit::new("C:\\setup.exe".into(), false)), Kind::Other);
}

#[test]
fn a_file_shows_its_size_and_dates() {
    let path = scratch("facts.txt", b"hello");
    let hit = Hit::new(path.to_string_lossy().into_owned(), false);
    let loaded = load(&hit, 1.0, true);
    let _ = std::fs::remove_file(&path);
    assert_eq!(loaded.facts.size, Some(5));
    assert!(loaded.facts.created.is_some() && loaded.facts.modified.is_some());
    assert!(matches!(loaded.visual, Visual::Text(text) if text == "hello"));
}

#[test]
fn a_large_picture_shrinks_to_fit_and_turns_bgra() {
    let path = std::env::temp_dir().join(format!("vestigo-preview-{}-wide.png", std::process::id()));
    image::RgbaImage::from_pixel(400, 100, image::Rgba([255, 0, 0, 255])).save(&path).expect("png");
    let picture = picture(&path.to_string_lossy(), 200);
    let _ = std::fs::remove_file(&path);
    let picture = picture.expect("decoded");
    let size = picture.size(0);
    assert_eq!((size.width.0, size.height.0), (200, 50));
    assert_eq!(picture.as_bytes(0).and_then(|bytes| bytes.get(..4)), Some(&[0, 0, 255, 255][..]));
}
