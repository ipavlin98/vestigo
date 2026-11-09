use super::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum List {
    Filters,
    Actions,
    Commands,
    Web,
    Favorites,
    MenuTools,
}

impl Settings {
    pub(crate) fn count(&self, list: List) -> usize {
        match list {
            List::Filters => self.filters.len(),
            List::Actions => self.actions.len(),
            List::Commands => self.commands.len(),
            List::Web => self.web.len(),
            List::Favorites => self.menu.favorites.len(),
            List::MenuTools => self.menu.tools.len(),
        }
    }

    pub(crate) fn remove_item(&mut self, list: List, position: usize) -> bool {
        match list {
            List::Filters => take(&mut self.filters, position).is_some(),
            List::Actions => take(&mut self.actions, position).is_some(),
            List::Commands => {
                let Some(command) = take(&mut self.commands, position) else { return false };
                self.menu.forget_command(&command, &self.commands);
                true
            }
            List::Web => take(&mut self.web, position).is_some(),
            List::Favorites => take(&mut self.menu.favorites, position).is_some(),
            List::MenuTools => take(&mut self.menu.tools, position).is_some(),
        }
    }

    pub(crate) fn move_item(&mut self, list: List, from: usize, to: usize) -> bool {
        match list {
            List::Filters => shift(&mut self.filters, from, to),
            List::Actions => shift(&mut self.actions, from, to),
            List::Commands => shift(&mut self.commands, from, to),
            List::Web => shift(&mut self.web, from, to),
            List::Favorites => shift(&mut self.menu.favorites, from, to),
            List::MenuTools => shift(&mut self.menu.tools, from, to),
        }
    }

    pub(crate) fn rename_command(&mut self, position: usize, name: &str) {
        let Some(command) = self.commands.get_mut(position) else { return };
        let old = std::mem::replace(&mut command.name, name.to_owned());
        if command.builtin.is_none() {
            self.menu.rename_command(&old, name);
        }
    }
}

pub(crate) fn after_move(position: usize, from: usize, to: usize) -> usize {
    if position == from {
        to
    } else if from < position && position <= to {
        position - 1
    } else if to <= position && position < from {
        position + 1
    } else {
        position
    }
}

pub(crate) fn after_removal(position: usize, removed: usize) -> Option<usize> {
    match position.cmp(&removed) {
        std::cmp::Ordering::Less => Some(position),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(position - 1),
    }
}

fn take<T>(items: &mut Vec<T>, position: usize) -> Option<T> {
    (position < items.len()).then(|| items.remove(position))
}

fn shift<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{CommandSpec, MenuTool};

    fn custom(name: &str) -> CommandSpec {
        CommandSpec { name: name.to_owned(), builtin: None, ..CommandSpec::default() }
    }

    #[test]
    fn shift_moves_an_item_down_and_up() {
        let mut items = vec!['a', 'b', 'c', 'd'];
        assert!(shift(&mut items, 0, 2));
        assert_eq!(items, ['b', 'c', 'a', 'd']);
        assert!(shift(&mut items, 3, 0));
        assert_eq!(items, ['d', 'b', 'c', 'a']);
    }

    #[test]
    fn shift_ignores_the_same_place_and_positions_past_the_end() {
        let mut items = vec!['a', 'b'];
        assert!(!shift(&mut items, 1, 1));
        assert!(!shift(&mut items, 0, 2));
        assert!(!shift(&mut items, 2, 0));
        assert_eq!(items, ['a', 'b']);
    }

    #[test]
    fn positions_keep_pointing_at_the_same_item_after_a_move() {
        let before = ['a', 'b', 'c', 'd', 'e'];
        for (from, to) in [(0, 3), (3, 0), (1, 2), (4, 1), (2, 2)] {
            let mut after = before.to_vec();
            shift(&mut after, from, to);
            for (position, item) in before.iter().enumerate() {
                assert_eq!(after[after_move(position, from, to)], *item, "{item} after moving {from} to {to}");
            }
        }
    }

    #[test]
    fn positions_keep_pointing_at_the_same_item_after_a_removal() {
        let before = ['a', 'b', 'c', 'd'];
        let mut after = before.to_vec();
        after.remove(1);
        assert_eq!(after_removal(1, 1), None);
        for position in [0, 2, 3] {
            assert_eq!(after_removal(position, 1).map(|kept| after[kept]), Some(before[position]));
        }
    }

    #[test]
    fn a_removed_command_leaves_the_menu() {
        let mut settings = Settings { commands: vec![custom("Build"), custom("Deploy")], ..Settings::default() };
        settings.menu.tools = vec![MenuTool::Custom("Build".to_owned()), MenuTool::Custom("Deploy".to_owned())];
        assert!(settings.remove_item(List::Commands, 0));
        assert_eq!(settings.menu.tools, [MenuTool::Custom("Deploy".to_owned())]);
        assert!(!settings.remove_item(List::Commands, 5));
    }

    #[test]
    fn a_renamed_command_stays_in_the_menu() {
        let mut settings = Settings { commands: vec![custom("Build")], ..Settings::default() };
        settings.menu.tools = vec![MenuTool::Custom("Build".to_owned())];
        settings.rename_command(0, "Make");
        assert_eq!(settings.commands[0].name, "Make");
        assert_eq!(settings.menu.tools, [MenuTool::Custom("Make".to_owned())]);
    }
}
