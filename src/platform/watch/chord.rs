use super::super::keys::virtual_key;
use super::super::{Binding, DialogKey, Hotkey};
use crate::settings::Shortcut;

const KEY_G: u16 = 0x47;
const KEY_1: u16 = 0x31;
const KEY_9: u16 = 0x39;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Chord {
    pub(super) key: u16,
    pub(super) ctrl: bool,
    pub(super) alt: bool,
    pub(super) shift: bool,
    pub(super) win: bool,
}

impl Chord {
    pub(super) fn of(shortcut: &Shortcut) -> Option<Self> {
        let Shortcut { ctrl, alt, shift, win, .. } = *shortcut;
        Some(Self { key: virtual_key(&shortcut.key)?, ctrl, alt, shift, win })
    }

    pub(super) fn in_dialog(self) -> Option<DialogKey> {
        if !self.ctrl || self.alt || self.shift || self.win {
            return None;
        }
        match self.key {
            KEY_G => Some(DialogKey::Switch),
            key @ KEY_1..=KEY_9 => Some(DialogKey::Row(usize::from(key - KEY_1))),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct Chords {
    pub(super) global: Vec<(Chord, Hotkey)>,
    pub(super) explorer: Vec<(Chord, Hotkey)>,
}

impl Chords {
    pub(super) fn new(bindings: &[Binding]) -> Self {
        let mut chords = Self::default();
        for binding in bindings {
            let Some(chord) = Chord::of(&binding.shortcut) else { continue };
            let list = if binding.explorer { &mut chords.explorer } else { &mut chords.global };
            if !list.iter().any(|(known, _)| *known == chord) {
                list.push((chord, binding.hotkey));
            }
        }
        chords
    }

    pub(super) fn in_explorer(&self, pressed: Chord) -> Option<Hotkey> {
        self.explorer.iter().find(|(chord, _)| *chord == pressed).map(|(_, hotkey)| *hotkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(text: &str, hotkey: Hotkey, explorer: bool) -> Binding {
        Binding { shortcut: Shortcut::parse(text).expect("shortcut"), hotkey, explorer }
    }

    #[test]
    fn explorer_chords_match_exact_modifiers() {
        let chords = Chords::new(&[
            binding("ctrl-shift-r", Hotkey::Command(7), true),
            binding("alt-space", Hotkey::Search, false),
        ]);
        let pressed = Chord { key: 0x52, ctrl: true, alt: false, shift: true, win: false };
        assert_eq!(chords.in_explorer(pressed), Some(Hotkey::Command(7)));
        assert_eq!(chords.in_explorer(Chord { shift: false, ..pressed }), None);
        assert_eq!(chords.in_explorer(Chord { key: 0x20, ctrl: false, alt: true, shift: false, win: false }), None);
        assert_eq!(chords.global.len(), 1);
    }

    #[test]
    fn ctrl_g_and_ctrl_digits_reach_the_dialog_bar() {
        let ctrl = |key| Chord { key, ctrl: true, alt: false, shift: false, win: false };
        assert_eq!(ctrl(0x47).in_dialog(), Some(DialogKey::Switch));
        assert_eq!(ctrl(0x31).in_dialog(), Some(DialogKey::Row(0)));
        assert_eq!(ctrl(0x39).in_dialog(), Some(DialogKey::Row(8)));
        assert_eq!(ctrl(0x30).in_dialog(), None);
        assert_eq!(Chord { shift: true, ..ctrl(0x31) }.in_dialog(), None);
        assert_eq!(Chord { ctrl: false, ..ctrl(0x47) }.in_dialog(), None);
    }

    #[test]
    fn a_second_binding_of_the_same_keys_is_ignored() {
        let chords =
            Chords::new(&[binding("ctrl-k", Hotkey::Window, false), binding("ctrl-k", Hotkey::Action(2), false)]);
        assert_eq!(chords.global.iter().map(|(_, hotkey)| *hotkey).collect::<Vec<_>>(), [Hotkey::Window]);
    }
}
