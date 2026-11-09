use super::{Launcher, Mode, RECENT_LIMIT};
use crate::index::Hit;
use crate::platform::{Host, HostKind};
use crate::ui::{DialogSources, Listing, Sources};
use gpui::{AppContext, Context, Task};

impl Launcher {
    pub(super) fn suggest(&mut self, cx: &mut Context<Self>) {
        let suggestions = match self.mode() {
            Some(Mode::Launcher) => {
                if let Some(hits) = self.recent.clone() {
                    self.show(Vec::new(), hits, Vec::new(), Listing::Recent, cx);
                }
                self.filling = Some(self.refresh_recent(cx));
                return;
            }
            Some(Mode::Dialog(host, ..)) => match self.offered(host) {
                Some(hits) => {
                    self.filling = None;
                    self.show(Vec::new(), hits, Vec::new(), Listing::Folders, cx);
                    return;
                }
                None => {
                    let came_from = self.session.as_ref().and_then(|session| session.came_from);
                    Suggestions::Dialog(DialogSources::of(host, came_from, cx))
                }
            },
            Some(Mode::Explorer(_)) | None => Suggestions::Recent(None),
        };
        let listing = suggestions.listing();
        let dialog = self.host().filter(|host| host.kind == HostKind::Dialog);
        self.filling = Some(cx.spawn(async move |this, cx| {
            let hits = cx.background_spawn(async move { suggestions.collect() }).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(session) = this.session.as_mut() {
                    session.offered = dialog.map(|host| (host, hits.clone()));
                }
                this.show(Vec::new(), hits, Vec::new(), listing, cx);
            });
        }));
    }

    fn offered(&self, host: Host) -> Option<Vec<Hit>> {
        let (offered, hits) = self.session.as_ref()?.offered.as_ref()?;
        (*offered == host).then(|| hits.clone())
    }

    pub(super) fn refresh_recent(&self, cx: &mut Context<Self>) -> Task<()> {
        let suggestions = Suggestions::Recent(Sources::of(cx));
        cx.spawn(async move |this, cx| {
            let hits = cx.background_spawn(async move { suggestions.collect() }).await;
            let _ = this.update(cx, |this, cx| this.recent_found(hits, cx));
        })
    }

    fn recent_found(&mut self, hits: Vec<Hit>, cx: &mut Context<Self>) {
        if self.recent.as_ref() == Some(&hits) {
            return;
        }
        self.recent = Some(hits.clone());
        if self.mode() == Some(Mode::Launcher) && self.menu.is_none() && self.input.read(cx).text().is_empty() {
            self.show(Vec::new(), hits, Vec::new(), Listing::Recent, cx);
        }
    }
}

enum Suggestions {
    Recent(Option<Sources>),
    Dialog(DialogSources),
}

impl Suggestions {
    fn listing(&self) -> Listing {
        match self {
            Self::Recent(_) => Listing::Recent,
            Self::Dialog(_) => Listing::Folders,
        }
    }

    fn collect(self) -> Vec<Hit> {
        match self {
            Self::Recent(sources) => sources.map(|sources| sources.collect(RECENT_LIMIT)).unwrap_or_default(),
            Self::Dialog(sources) => sources.collect(),
        }
    }
}
