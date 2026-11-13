// Modified from OpenAI Codex rust-v0.93.0 (Apache-2.0).

use ratatui::crossterm::event::KeyCode;
use ratatui::crossterm::event::KeyEvent;
use ratatui::crossterm::event::KeyEventKind;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::style::Style;
use ratatui::text::Span;

#[cfg(test)]
const ALT_PREFIX: &str = "⌥ + ";
#[cfg(all(not(test), target_os = "macos"))]
const ALT_PREFIX: &str = "⌥ + ";
#[cfg(all(not(test), not(target_os = "macos")))]
const ALT_PREFIX: &str = "alt + ";
const CTRL_PREFIX: &str = "ctrl + ";
const SHIFT_PREFIX: &str = "shift + ";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KeyBinding {
    key: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyBinding {
    pub(crate) const fn new(key: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn is_press(&self, event: KeyEvent) -> bool {
        self.key == event.code
            && self.modifiers == event.modifiers
            && (event.kind == KeyEventKind::Press || event.kind == KeyEventKind::Repeat)
    }
}

pub(crate) const fn plain(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::NONE)
}

pub(crate) const fn shift(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::SHIFT)
}

pub(crate) const fn ctrl(key: KeyCode) -> KeyBinding {
    KeyBinding::new(key, KeyModifiers::CONTROL)
}

fn modifiers_to_string(modifiers: KeyModifiers) -> String {
    let mut result = String::new();
    if modifiers.contains(KeyModifiers::CONTROL) {
        result.push_str(CTRL_PREFIX);
    }
    if modifiers.contains(KeyModifiers::SHIFT) {
        result.push_str(SHIFT_PREFIX);
    }
    if modifiers.contains(KeyModifiers::ALT) {
        result.push_str(ALT_PREFIX);
    }
    result
}

impl From<KeyBinding> for Span<'static> {
    fn from(binding: KeyBinding) -> Self {
        (&binding).into()
    }
}
impl From<&KeyBinding> for Span<'static> {
    fn from(binding: &KeyBinding) -> Self {
        let KeyBinding { key, modifiers } = binding;
        let modifiers = modifiers_to_string(*modifiers);
        let key = match key {
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Char(' ') => "space".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::PageUp => "pgup".to_string(),
            KeyCode::PageDown => "pgdn".to_string(),
            // Shift+Tab, as the keymap keeps it.
            KeyCode::BackTab => "shift + tab".to_string(),
            _ => format!("{key}").to_ascii_lowercase(),
        };
        Span::styled(format!("{modifiers}{key}"), key_hint_style())
    }
}

fn key_hint_style() -> Style {
    Style::default().dim()
}

pub(crate) fn has_ctrl_or_alt(mods: KeyModifiers) -> bool {
    (mods.contains(KeyModifiers::CONTROL) || mods.contains(KeyModifiers::ALT)) && !is_altgr(mods)
}

#[cfg(windows)]
#[inline]
pub(crate) fn is_altgr(mods: KeyModifiers) -> bool {
    mods.contains(KeyModifiers::ALT) && mods.contains(KeyModifiers::CONTROL)
}

#[cfg(not(windows))]
#[inline]
pub(crate) fn is_altgr(_mods: KeyModifiers) -> bool {
    false
}

/// A key in one canonical form: an uppercase letter as Shift and the lower
/// case one, a C0 control character as Ctrl and its letter, Shift+Tab as
/// BackTab. Terminals differ in which of these forms they report.
pub(crate) fn normalize_key_parts(
    key: KeyCode,
    mut modifiers: KeyModifiers,
) -> (KeyCode, KeyModifiers) {
    if key == KeyCode::BackTab || (key == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT)) {
        modifiers.remove(KeyModifiers::SHIFT);
        return (KeyCode::BackTab, modifiers);
    }
    let KeyCode::Char(ch) = key else {
        return (key, modifiers);
    };
    if modifiers.is_empty() {
        if let Some(ctrl_char) = c0_control_char_to_ctrl_char(ch) {
            return normalize_key_parts(KeyCode::Char(ctrl_char), KeyModifiers::CONTROL);
        }
    }
    if ch.is_ascii_uppercase() {
        modifiers.insert(KeyModifiers::SHIFT);
        return (KeyCode::Char(ch.to_ascii_lowercase()), modifiers);
    }
    (key, modifiers)
}

fn c0_control_char_to_ctrl_char(ch: char) -> Option<char> {
    let code = u32::from(ch);
    match code {
        0x00 => Some(' '),
        0x01..=0x1a => char::from_u32(code - 0x01 + u32::from('a')),
        0x1c..=0x1f => char::from_u32(code - 0x1c + u32::from('4')),
        _ => None,
    }
}
