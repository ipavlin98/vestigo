#![windows_subsystem = "windows"]

mod app;
mod app_window;
mod assets;
mod i18n;
mod index;
mod input;
mod launcher;
mod morph;
mod platform;
mod search;
mod settings;
mod theme;
mod ui;
mod update;

use futures::channel::mpsc;
use gpui::{App, Application, TextRenderingMode};
use index::{Index, Update};
use platform::{Event, Remote};
use settings::{History, Settings};
use std::{rc::Rc, sync::Arc};
use theme::Catalog;

pub(crate) const ELEVATED_FLAG: &str = "--elevated";
const EXTRACT_FLAG: &str = "--extract";
const MORPH_FLAG: &str = "--morph";
const INDEX_DIR: &str = "index";
const CONTENT_DIR: &str = "content";
const LOG_LIMIT: u64 = 1 << 20;
const LOG_FILES: usize = 3;

fn main() {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some(EXTRACT_FLAG) => {
            platform::enter_background();
            index::serve(arguments.next().and_then(|code_page| code_page.parse().ok()).unwrap_or_default());
            return;
        }
        Some(MORPH_FLAG) => {
            morph::run();
            return;
        }
        _ => {}
    }
    if std::env::args().any(|argument| argument == update::WAIT_FLAG) {
        platform::await_previous_exit();
    }
    let remote = std::env::args().find_map(|argument| Remote::from_flag(&argument)).unwrap_or(Remote::Toggle);
    if platform::signal_running_instance(remote) || remote == Remote::Quit {
        return;
    }
    let elevated = platform::is_elevated();
    let asked = std::env::args().any(|argument| argument == ELEVATED_FLAG);
    if !elevated && !asked && platform::relaunch_elevated(ELEVATED_FLAG) {
        return;
    }
    let Some(_instance) = platform::single_instance() else {
        return;
    };
    let data = settings::adopt_legacy_data();
    let cache = settings::adopt_legacy_cache();
    let _logger = start_logging();
    adopt_legacy(data, cache);
    update::sweep();
    if !elevated {
        log::warn!("Running without administrator rights: drives are walked instead of read from the MFT");
    }
    run();
}

fn adopt_legacy(data: anyhow::Result<bool>, cache: anyhow::Result<bool>) {
    if let Err(error) = &cache {
        log::warn!("Couldn't take over the old index cache: {error:#}");
    }
    match data {
        Ok(true) => {
            log::info!("Took over the settings and history of sfs");
            if let Err(error) = platform::adopt_legacy_autostart() {
                log::warn!("Couldn't move the start with Windows over from sfs: {error:#}");
            }
        }
        Ok(false) => {}
        Err(error) => log::warn!("Couldn't take over the settings of sfs: {error:#}"),
    }
}

fn run() {
    let (sender, receiver) = mpsc::unbounded();
    platform::connect(sender);
    let settings = Settings::load();
    platform::configure(app::watching(&settings));
    settings::retire_themes(&theme::RETIRED);
    let catalog = Catalog::load(settings::theme_files());
    let cache = settings::cache_dir();
    let extractor = Arc::new(platform::spawn_extractor);
    let index = Arc::new(Index::with_cache(cache.join(INDEX_DIR)).extracting(cache.join(CONTENT_DIR), extractor));
    index.rank(app::ranking(&settings));
    index.rewalk_every(settings.index.rewalk.every());
    index.start(app::drives(&settings), settings.index.folders.clone(), |update| {
        platform::send(match update {
            Update::Contents => Event::IndexChanged,
            Update::Count => Event::IndexCounted,
        })
    });
    index.full_text(settings.index.full_text);
    if let Err(error) = platform::start_watching() {
        log::error!("{error:#}");
    }
    let shown = index.clone();
    match gpui_windows::WindowsPlatform::new(false) {
        Ok(windows) => {
            Application::with_platform(Rc::new(windows)).with_assets(assets::Assets).run(move |cx: &mut App| {
                cx.set_text_rendering_mode(TextRenderingMode::Grayscale);
                app::bind_keys(&settings.actions, cx);
                cx.set_global(settings);
                cx.set_global(catalog);
                cx.set_global(History::load());
                cx.set_global(update::Updates::default());

                app::start(shown, receiver, cx);
            })
        }
        Err(error) => {
            log::error!("{error:#}");
            index.close_content();
            index.save();
        }
    }
    platform::stop_watching();
}

fn start_logging() -> Option<flexi_logger::LoggerHandle> {
    use flexi_logger::{Cleanup, Criterion, FileSpec, Logger, Naming};
    Logger::try_with_env_or_str("info")
        .ok()?
        .log_to_file(FileSpec::default().directory(settings::data_dir().join("logs")).suppress_timestamp())
        .rotate(Criterion::Size(LOG_LIMIT), Naming::Numbers, Cleanup::KeepLogFiles(LOG_FILES))
        .append()
        .start()
        .ok()
}
