use gpui::SharedString;
use std::ops::Range;

#[cfg(test)]
mod tests;

const DEPTH: usize = 100;

pub(super) type Snapshot = (SharedString, Range<usize>);

#[derive(Default)]
pub(super) struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    typing: bool,
}

impl History {
    pub(super) fn record(&mut self, before: Snapshot, typing: bool) {
        if !(typing && self.typing) {
            push(&mut self.undo, before);
        }
        self.redo.clear();
        self.typing = typing;
    }

    pub(super) fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        step(&mut self.undo, &mut self.redo, current, &mut self.typing)
    }

    pub(super) fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        step(&mut self.redo, &mut self.undo, current, &mut self.typing)
    }

    pub(super) fn pause(&mut self) {
        self.typing = false;
    }

    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

fn step(from: &mut Vec<Snapshot>, to: &mut Vec<Snapshot>, current: Snapshot, typing: &mut bool) -> Option<Snapshot> {
    let target = from.pop()?;
    push(to, current);
    *typing = false;
    Some(target)
}

fn push(stack: &mut Vec<Snapshot>, snapshot: Snapshot) {
    if stack.len() == DEPTH {
        stack.remove(0);
    }
    stack.push(snapshot);
}
