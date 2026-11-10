use super::Shell;
use crate::app_window::{Page, Section};
use crate::i18n::Language;
use crate::platform;
use crate::update::{self, Release, Trouble, Updates, Version};
use futures::channel::oneshot;
use gpui::{App, Context, Task};
use std::time::Duration;

const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
pub(super) struct Updater {
    schedule: Option<Task<()>>,
    announced: Option<Version>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Check {
    Auto,
    Manual,
}

impl Shell {
    pub(super) fn schedule_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.updater.schedule = enabled.then(|| {
            cx.spawn(async move |this, cx| {
                let mut wait = FIRST_CHECK;
                loop {
                    cx.background_executor().timer(wait).await;
                    if this.update(cx, |this, cx| this.check_updates(Check::Auto, cx)).is_err() {
                        return;
                    }
                    wait = CHECK_INTERVAL;
                }
            })
        });
    }

    pub(super) fn check_updates(&mut self, check: Check, cx: &mut Context<Self>) {
        let busy = matches!(cx.global::<Updates>(), Updates::Checking | Updates::Downloading(_));
        if busy || (check == Check::Auto && matches!(cx.global::<Updates>(), Updates::InstallFailed(..))) {
            return;
        }
        if check == Check::Manual {
            cx.set_global(Updates::Checking);
        }
        cx.spawn(async move |this, cx| {
            let found = cx.background_executor().spawn(async move { update::check() }).await;
            let _ = this.update(cx, |this, cx| this.checked(check, found, cx));
        })
        .detach();
    }

    fn checked(&mut self, check: Check, found: Result<Option<Release>, Trouble>, cx: &mut Context<Self>) {
        match (found, check) {
            (Ok(Some(release)), _) => {
                log::info!("Update available: {}", release.version);
                if check == Check::Auto {
                    self.announce(&release.version, cx);
                }
                cx.set_global(Updates::Available(release));
            }
            (Ok(None), Check::Manual) => cx.set_global(Updates::Latest),
            (Ok(None), Check::Auto) => log::info!("No updates"),
            (Err(trouble), Check::Manual) => {
                log::warn!("Couldn't check for updates: {trouble:?}");
                cx.set_global(Updates::CheckFailed(trouble));
            }
            (Err(trouble), Check::Auto) => log::warn!("Couldn't check for updates: {trouble:?}"),
        }
    }

    fn announce(&mut self, version: &Version, cx: &App) {
        if self.updater.announced.as_ref() == Some(version) {
            return;
        }
        self.updater.announced = Some(version.clone());
        let language = *cx.global::<Language>();
        let title = language.update_announced(&version.to_string());
        if !platform::announce(&title, language.strings().update_notice) {
            log::info!("No tray icon to announce the update");
        }
    }

    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
        let (Updates::Available(release) | Updates::InstallFailed(release, _)) = cx.global::<Updates>().clone() else {
            return;
        };
        cx.set_global(Updates::Downloading(release.clone()));
        let language = *cx.global::<Language>();
        cx.spawn(async move |this, cx| {
            let prepared = {
                let release = release.clone();
                cx.background_executor().spawn(async move { update::prepare(&release) }).await
            };
            let started = match prepared {
                Ok(handover) => {
                    let (sender, receiver) = oneshot::channel();
                    platform::on_shell_thread(move || {
                        let _ = sender.send(handover.start(language));
                    });
                    receiver.await.unwrap_or_else(|_| Err(Trouble::Failed("the update didn't start".into())))
                }
                Err(trouble) => Err(trouble),
            };
            let _ = this.update(cx, |_, cx| match started {
                Ok(()) => {
                    log::info!("Quitting to update to {}", release.version);
                    cx.quit();
                }
                Err(trouble) => {
                    log::warn!("Couldn't update to {}: {trouble:?}", release.version);
                    cx.set_global(Updates::InstallFailed(release, trouble));
                }
            });
        })
        .detach();
    }

    pub(super) fn show_update(&mut self, cx: &mut Context<Self>) {
        self.open_app_window(Page::Settings(Section::About), None, cx);
    }
}
