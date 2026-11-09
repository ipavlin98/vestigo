use serde::{Deserialize, Serialize};
use std::fmt;

const NAMED_KEYS: [(&str, &str); 16] = [
    ("space", "Space"),
    ("enter", "Enter"),
    ("tab", "Tab"),
    ("backspace", "Backspace"),
    ("delete", "Del"),
    ("insert", "Ins"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "PgUp"),
    ("pagedown", "PgDn"),
    ("up", "↑"),
    ("down", "↓"),
    ("left", "←"),
    ("right", "→"),
    ("escape", "Esc"),
    ("pause", "Pause"),
];
const PUNCTUATION: &str = "`-=[]\\;',./";
const MAX_FUNCTION_KEY: u8 = 24;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct Shortcut {
    pub(crate) ctrl: bool,
    pub(crate) alt: bool,
    pub(crate) shift: bool,
    pub(crate) win: bool,
    pub(crate) key: String,
}

impl Shortcut {
    pub(crate) fn new(ctrl: bool, alt: bool, shift: bool, win: bool, key: &str) -> Option<Self> {
        let key = key.to_lowercase();
        known_key(&key).then_some(Self { ctrl, alt, shift, win, key })
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let parts: Vec<String> = text.split(['-', '+']).map(|part| part.trim().to_lowercase()).collect();
        let (key, modifiers) = match parts.as_slice() {
            [.., empty, last] if empty.is_empty() && last.is_empty() => ("-", parts.get(..parts.len() - 2)?),
            [modifiers @ .., key] => (key.as_str(), modifiers),
            [] => return None,
        };
        let mut shortcut = Self { ctrl: false, alt: false, shift: false, win: false, key: key.to_owned() };
        for modifier in modifiers {
            match modifier.as_str() {
                "ctrl" | "control" => shortcut.ctrl = true,
                "alt" => shortcut.alt = true,
                "shift" => shortcut.shift = true,
                "win" | "cmd" | "super" => shortcut.win = true,
                _ => return None,
            }
        }
        known_key(&shortcut.key).then_some(shortcut)
    }

    pub(crate) fn has_modifier(&self) -> bool {
        self.ctrl || self.alt || self.win || self.shift && self.key.len() > 1
    }

    pub(crate) fn spares_typing(&self) -> bool {
        self.has_modifier() || self.key.chars().count() > 1
    }

    pub(crate) fn keystroke(&self) -> String {
        let mut parts = Vec::new();
        for (on, name) in [(self.ctrl, "ctrl"), (self.alt, "alt"), (self.shift, "shift"), (self.win, "win")] {
            if on {
                parts.push(name);
            }
        }
        parts.push(&self.key);
        parts.join("-")
    }
}

fn known_key(key: &str) -> bool {
    let function = key.strip_prefix('f').and_then(|number| number.parse::<u8>().ok());
    let single =
        key.chars().count() == 1 && key.chars().all(|ch| ch.is_ascii_alphanumeric() || PUNCTUATION.contains(ch));
    single
        || NAMED_KEYS.iter().any(|(name, _)| *name == key)
        || function.is_some_and(|n| (1..=MAX_FUNCTION_KEY).contains(&n))
}

impl Shortcut {
    pub(crate) fn parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")]
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, name)| name.to_owned())
            .collect();
        parts.push(match NAMED_KEYS.iter().find(|(name, _)| *name == self.key) {
            Some((_, label)) => (*label).to_owned(),
            None => self.key.to_uppercase(),
        });
        parts
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.parts().join("+"))
    }
}

impl TryFrom<String> for Shortcut {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text).ok_or_else(|| format!("unknown shortcut {text:?}"))
    }
}

impl From<Shortcut> for String {
    fn from(shortcut: Shortcut) -> Self {
        shortcut.keystroke()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_round_trip_through_text() {
        let shortcut = Shortcut::parse("Ctrl+Shift+R").expect("shortcut");
        assert_eq!(shortcut.keystroke(), "ctrl-shift-r");
        assert_eq!(shortcut.to_string(), "Ctrl+Shift+R");
        assert_eq!(Shortcut::parse(&shortcut.keystroke()), Some(shortcut));
        assert_eq!(Shortcut::parse("alt-space").map(|shortcut| shortcut.to_string()).as_deref(), Some("Alt+Space"));
        assert_eq!(Shortcut::parse("ctrl--").map(|shortcut| shortcut.key), Some("-".to_owned()));
        assert_eq!(Shortcut::parse("win-f12").map(|shortcut| shortcut.to_string()).as_deref(), Some("Win+F12"));
    }

    #[test]
    fn unknown_keys_and_modifiers_are_rejected() {
        assert!(Shortcut::parse("hyper-k").is_none());
        assert!(Shortcut::parse("ctrl-f25").is_none());
        assert!(Shortcut::parse("ctrl-ы").is_none());
        assert!(Shortcut::parse("").is_none());
    }

    #[test]
    fn a_plain_key_is_not_a_global_shortcut() {
        assert!(!Shortcut::parse("k").expect("shortcut").has_modifier());
        assert!(!Shortcut::parse("shift-k").expect("shortcut").has_modifier());
        assert!(Shortcut::parse("shift-f2").expect("shortcut").has_modifier());
        assert!(Shortcut::parse("alt-k").expect("shortcut").has_modifier());
        assert!(!Shortcut::parse("k").expect("shortcut").spares_typing());
        assert!(Shortcut::parse("delete").expect("shortcut").spares_typing());
        assert!(Shortcut::parse("ctrl-k").expect("shortcut").spares_typing());
    }
}
