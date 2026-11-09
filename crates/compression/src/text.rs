// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/truncate.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
//
// Every length the kernel compares against a threshold is a JavaScript
// string length: UTF-16 code units. They are measured the same way here, so
// limits and token estimates match the original for every script.

/// The length of `text` in UTF-16 code units.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// At most `max_units` UTF-16 code units of `text`, never splitting a
/// character.
pub(crate) fn clamp_prefix(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        units += character.len_utf16();
        if units > max_units {
            return &text[..index];
        }
    }
    text
}

/// Collapses every whitespace run to one space (JavaScript's
/// `replace(/\s+/g, " ")`).
pub(crate) fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(character);
            in_space = false;
        }
    }
    out
}

/// The last `max_units` UTF-16 code units of `text` at most, never
/// splitting a character.
pub(crate) fn clamp_suffix(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (index, character) in text.char_indices().rev() {
        units += character.len_utf16();
        if units > max_units {
            return &text[index + character.len_utf8()..];
        }
    }
    text
}

#[cfg(test)]
mod tests;
