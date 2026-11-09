// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! Key strings as `tui.keymap` writes them (Codex's `parse_keybinding`):
//! modifiers then a key, joined with `-` (`ctrl-t`, `shift-enter`,
//! `page-down`, `f3`, `alt-<`); two strokes separated by a space are a
//! chord (`ctrl-x ctrl-s`), and `leader K` is the leader key then `K`.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::key_hint::{self, normalize_key_parts};

/// The highest function key a key string names.
pub(crate) const MAX_FUNCTION_KEY: u8 = 24;

/// One key with its modifiers, in [`normalize_key_parts`]'s form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct KeyStroke {
    code: KeyCodeOrd,
    modifiers: u8,
}

/// `KeyCode` is not `Ord`; a stroke is kept as what it compares by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum KeyCodeOrd {
    Char(char),
    F(u8),
    Named(u8),
}

/// The named keys, as a key string writes them, by [`KeyCodeOrd::Named`].
const NAMED: [(&str, KeyCode); 15] = [
    ("enter", KeyCode::Enter),
    ("tab", KeyCode::Tab),
    ("backtab", KeyCode::BackTab),
    ("backspace", KeyCode::Backspace),
    ("esc", KeyCode::Esc),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("page-up", KeyCode::PageUp),
    ("page-down", KeyCode::PageDown),
];

const MODIFIERS: [(&str, KeyModifiers); 4] = [
    ("ctrl", KeyModifiers::CONTROL),
    ("alt", KeyModifiers::ALT),
    ("shift", KeyModifiers::SHIFT),
    ("super", KeyModifiers::SUPER),
];

impl KeyStroke {
    pub(crate) fn new(code: KeyCode, modifiers: KeyModifiers) -> Option<Self> {
        let (code, modifiers) = normalize_key_parts(code, modifiers);
        let code = match code {
            KeyCode::Char(c) => KeyCodeOrd::Char(c),
            KeyCode::F(n) => KeyCodeOrd::F(n),
            other => KeyCodeOrd::Named(NAMED.iter().position(|(_, named)| *named == other)? as u8),
        };
        let modifiers = modifiers
            & (KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SHIFT
                | KeyModifiers::SUPER);
        Some(Self {
            code,
            modifiers: modifiers.bits(),
        })
    }

    /// The key `event` reports; `None` for a release or a key no string
    /// names.
    pub(crate) fn of(event: &KeyEvent) -> Option<Self> {
        if event.kind == KeyEventKind::Release {
            return None;
        }
        Self::new(event.code, event.modifiers)
    }

    pub(crate) fn code(&self) -> KeyCode {
        match self.code {
            KeyCodeOrd::Char(c) => KeyCode::Char(c),
            KeyCodeOrd::F(n) => KeyCode::F(n),
            KeyCodeOrd::Named(index) => NAMED[usize::from(index)].1,
        }
    }

    pub(crate) fn modifiers(&self) -> KeyModifiers {
        KeyModifiers::from_bits_truncate(self.modifiers)
    }

    /// Whether `event` is this key pressed (or repeated). Shifted
    /// punctuation matches with or without Shift: terminals differ.
    pub(crate) fn matches(&self, event: &KeyEvent) -> bool {
        let Some(pressed) = Self::of(event) else {
            return false;
        };
        if pressed == *self {
            return true;
        }
        let lenient = matches!(self.code, KeyCodeOrd::Char(c) if c.is_ascii_punctuation())
            && !self.modifiers().contains(KeyModifiers::SHIFT);
        lenient
            && pressed.code == self.code
            && pressed.modifiers().difference(KeyModifiers::SHIFT) == self.modifiers()
    }

    /// A key that types text: a character with no modifier but Shift.
    pub(crate) fn is_text(&self) -> bool {
        matches!(self.code, KeyCodeOrd::Char(_))
            && self.modifiers().difference(KeyModifiers::SHIFT).is_empty()
    }

    /// The hint the interface draws for it.
    pub(crate) fn hint(&self) -> key_hint::KeyBinding {
        key_hint::KeyBinding::new(self.code(), self.modifiers())
    }

    /// Parses one stroke (`ctrl-t`, `shift-enter`, `G`, `f3`).
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let invalid = || {
            format!(
                "`{text}` is not a key: write one like `ctrl-a`, `shift-enter`, `page-down` or `f3`"
            )
        };
        let mut modifiers = KeyModifiers::NONE;
        let mut rest = text;
        while let Some((head, tail)) = rest.split_once('-') {
            let Some((_, modifier)) = MODIFIERS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(head))
            else {
                break;
            };
            if tail.is_empty() {
                return Err(invalid());
            }
            modifiers |= *modifier;
            rest = tail;
        }
        let lower = rest.to_ascii_lowercase();
        let code = match lower.as_str() {
            "space" => KeyCode::Char(' '),
            "minus" => KeyCode::Char('-'),
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "escape" => KeyCode::Esc,
            "return" => KeyCode::Enter,
            "del" => KeyCode::Delete,
            name => match NAMED.iter().find(|(named, _)| *named == name) {
                Some((_, code)) => *code,
                None => {
                    let mut chars = rest.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) if !c.is_whitespace() && !c.is_control() => {
                            KeyCode::Char(c)
                        }
                        _ => match lower.strip_prefix('f').map(str::parse::<u8>) {
                            Some(Ok(n)) if (1..=MAX_FUNCTION_KEY).contains(&n) => KeyCode::F(n),
                            _ => return Err(invalid()),
                        },
                    }
                }
            },
        };
        Self::new(code, modifiers).ok_or_else(invalid)
    }

    /// The key string that names it (`ctrl-t`).
    pub(crate) fn spec(&self) -> String {
        let mut text = String::new();
        for (name, modifier) in MODIFIERS {
            if self.modifiers().contains(modifier) {
                text.push_str(name);
                text.push('-');
            }
        }
        match self.code {
            KeyCodeOrd::Char(' ') => text.push_str("space"),
            KeyCodeOrd::Char('-') => text.push_str("minus"),
            KeyCodeOrd::Char(c) => text.push(c),
            KeyCodeOrd::F(n) => text.push_str(&format!("f{n}")),
            KeyCodeOrd::Named(index) => text.push_str(NAMED[usize::from(index)].0),
        }
        text
    }
}

/// What an action is bound to: a key, two keys in a row, or the leader
/// key then a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Binding {
    Key(KeyStroke),
    Chord(KeyStroke, KeyStroke),
    Leader(KeyStroke),
}

impl Binding {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let strokes: Vec<&str> = text.split_whitespace().collect();
        match strokes.as_slice() {
            [key] => KeyStroke::parse(key).map(Self::Key),
            ["leader", key] => KeyStroke::parse(key).map(Self::Leader),
            [first, second] => Ok(Self::Chord(
                KeyStroke::parse(first)?,
                KeyStroke::parse(second)?,
            )),
            _ => Err(format!(
                "`{text}` is not a key: write one key (`ctrl-t`), two (`ctrl-x ctrl-s`) or \
                 `leader` and a key (`leader p`)"
            )),
        }
    }

    pub(crate) fn spec(&self) -> String {
        match self {
            Self::Key(key) => key.spec(),
            Self::Chord(first, second) => format!("{} {}", first.spec(), second.spec()),
            Self::Leader(key) => format!("leader {}", key.spec()),
        }
    }
}

#[cfg(test)]
mod tests;
