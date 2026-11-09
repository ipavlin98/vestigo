use windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW;

const NAMED: [(&str, u16); 16] = [
    ("space", 0x20),
    ("enter", 0x0D),
    ("tab", 0x09),
    ("backspace", 0x08),
    ("delete", 0x2E),
    ("insert", 0x2D),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pagedown", 0x22),
    ("up", 0x26),
    ("down", 0x28),
    ("left", 0x25),
    ("right", 0x27),
    ("escape", 0x1B),
    ("pause", 0x13),
];
const PUNCTUATION: [(char, u16); 11] = [
    ('`', 0xC0),
    ('-', 0xBD),
    ('=', 0xBB),
    ('[', 0xDB),
    (']', 0xDD),
    ('\\', 0xDC),
    (';', 0xBA),
    ('\'', 0xDE),
    (',', 0xBC),
    ('.', 0xBE),
    ('/', 0xBF),
];
const F1: u16 = 0x70;
const SHIFT_STATE: u8 = 1;
const MAX_FUNCTION_KEY: u16 = 24;

pub(super) fn virtual_key(key: &str) -> Option<u16> {
    if let Some((_, code)) = NAMED.iter().find(|(name, _)| *name == key) {
        return Some(*code);
    }
    if let Some(number) = key.strip_prefix('f').and_then(|number| number.parse::<u16>().ok()) {
        return (1..=MAX_FUNCTION_KEY).contains(&number).then(|| F1 + number - 1);
    }
    let mut chars = key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else { return None };
    match ch {
        'a'..='z' => Some(u16::from(ch.to_ascii_uppercase() as u8)),
        '0'..='9' => Some(u16::from(ch as u8)),
        _ => PUNCTUATION.iter().find(|(known, _)| *known == ch).map(|(_, code)| *code),
    }
}

pub(crate) fn layout_key(key: &str) -> Option<(String, bool)> {
    let mut chars = key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else { return Some((key.to_owned(), false)) };
    if ch.is_ascii_lowercase() {
        return Some((key.to_owned(), false));
    }
    let mut units = [0u16; 2];
    let [unit] = ch.encode_utf16(&mut units) else { return None };
    let scan = unsafe { VkKeyScanW(*unit) };
    if scan == -1 {
        return None;
    }
    let [code, state] = scan.to_le_bytes();
    Some((key_name(u16::from(code))?, state & SHIFT_STATE != 0))
}

fn key_name(code: u16) -> Option<String> {
    let ch = char::from_u32(u32::from(code))?;
    match ch {
        'A'..='Z' => Some(ch.to_ascii_lowercase().to_string()),
        '0'..='9' => Some(ch.to_string()),
        _ => PUNCTUATION.iter().find(|(_, known)| *known == code).map(|(ch, _)| ch.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{key_name, virtual_key};

    #[test]
    fn virtual_keys_name_the_key_they_sit_on() {
        assert_eq!(key_name(0x41).as_deref(), Some("a"));
        assert_eq!(key_name(0x31).as_deref(), Some("1"));
        assert_eq!(key_name(0xBC).as_deref(), Some(","));
        assert_eq!(key_name(0x70), None);
    }

    #[test]
    fn keys_map_to_virtual_key_codes() {
        assert_eq!(virtual_key("a"), Some(0x41));
        assert_eq!(virtual_key("z"), Some(0x5A));
        assert_eq!(virtual_key("7"), Some(0x37));
        assert_eq!(virtual_key("f1"), Some(0x70));
        assert_eq!(virtual_key("f24"), Some(0x87));
        assert_eq!(virtual_key("space"), Some(0x20));
        assert_eq!(virtual_key("/"), Some(0xBF));
        assert_eq!(virtual_key("f25"), None);
        assert_eq!(virtual_key("ы"), None);
    }
}
