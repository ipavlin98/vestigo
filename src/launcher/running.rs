use super::Launcher;
use crate::index::Boosts;
use crate::platform::{self, RunningApp};
use futures::FutureExt;
use gpui::{AppContext, Context, Task};
use rustc_hash::FxHashMap;
use std::sync::Arc;

const RUNNING_BOOST: i32 = 60;

pub(super) type Running = Arc<FxHashMap<String, RunningApp>>;

impl Launcher {
    pub(super) fn find_running(&mut self, cx: &mut Context<Self>) {
        let task = match self.host() {
            None => cx.background_spawn(async {
                let mut running = FxHashMap::default();
                for (path, app) in platform::running_apps() {
                    running.entry(path).or_insert(app);
                }
                Arc::new(running)
            }),
            Some(_) => Task::ready(Running::default()),
        };
        self.running = task.shared();
    }

    pub(super) fn running_app(&self, path: &str) -> Option<RunningApp> {
        self.running.clone().now_or_never()?.get(&path.to_lowercase()).copied()
    }
}

pub(super) fn with_running(mut boosts: Boosts, running: &Running) -> Boosts {
    for path in running.keys() {
        *boosts.entry(path.clone()).or_default() += RUNNING_BOOST;
    }
    boosts
}
