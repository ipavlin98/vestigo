use super::results::ResultList;
use super::trail::{Entered, Entering, Trail, go_to_mark, set_mark};
use super::{EnterContaining, EnterFolder, GoToMark, Jump, LeaveFolder, SetMark};
use crate::index::{Cancel, Token};
use crate::input::TextInput;
use gpui::{AppContext, Context, Entity, Task, Window};
use std::future::Future;

pub(crate) struct Browse<'a> {
    pub(crate) input: &'a Entity<TextInput>,
    pub(crate) results: &'a mut ResultList,
    pub(crate) trail: &'a mut Trail,
}

pub(crate) trait Browser: Sized + 'static {
    fn browse(&mut self) -> Option<Browse<'_>>;

    fn selection_moved(&mut self, cx: &mut Context<Self>);

    fn jump(&mut self, jump: &Jump, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        browse.results.jump(jump.to);
        self.selection_moved(cx);
        cx.notify();
    }

    fn enter_folder(&mut self, _: &EnterFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        let entering = Entering::inside(browse.results, browse.input.read(cx).text());
        enter(entering, cx);
    }

    fn enter_containing(&mut self, _: &EnterContaining, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        let entering = Entering::beside(browse.results, browse.input.read(cx).text());
        enter(entering, cx);
    }

    fn leave_folder(&mut self, _: &LeaveFolder, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        let Some((text, row)) = browse.trail.back(browse.input.read(cx).text()) else { return };
        browse.results.reselect(row);
        browse.input.update(cx, |input, cx| input.set_text(&text, cx));
    }

    fn set_mark(&mut self, mark: &SetMark, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        set_mark(mark.letter, browse.results, cx);
        cx.notify();
    }

    fn go_to_mark(&mut self, mark: &GoToMark, _: &mut Window, cx: &mut Context<Self>) {
        let Some(browse) = self.browse() else { return };
        let typed = browse.input.read(cx).text().to_owned();
        match go_to_mark(mark.letter, browse.results, &typed, cx) {
            Some(entered) => go(self, entered, cx),
            None => cx.notify(),
        }
    }
}

fn enter<V: Browser>(entering: Option<Entering>, cx: &mut Context<V>) {
    let Some(entering) = entering else { return };
    cx.spawn(async move |this, cx| {
        let Some(entered) = cx.background_spawn(async move { entering.resolve() }).await else { return };
        let _ = this.update(cx, |this, cx| go(this, entered, cx));
    })
    .detach();
}

fn go<V: Browser>(this: &mut V, entered: Entered, cx: &mut Context<V>) {
    let Some(browse) = this.browse() else { return };
    let (text, select) = browse.trail.enter(entered);
    browse.results.reselect(select);
    browse.input.update(cx, |input, cx| input.set_text(&text, cx));
}

pub(crate) fn cancellable<V, R, F>(
    cx: &mut Context<V>,
    work: impl FnOnce(Token) -> F,
    done: impl FnOnce(&mut V, R, &mut Context<V>) + 'static,
) -> Task<()>
where
    V: 'static,
    R: Send + 'static,
    F: Future<Output = Option<R>> + Send + 'static,
{
    let cancel = Cancel::new();
    let work = work(cancel.token());
    cx.spawn(async move |this, cx| {
        let found = cx.background_spawn(work).await;
        drop(cancel);
        let Some(found) = found else { return };
        let _ = this.update(cx, |this, cx| done(this, found, cx));
    })
}
