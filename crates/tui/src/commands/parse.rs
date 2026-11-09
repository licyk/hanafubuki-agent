// Modified from OpenAI Codex 57a808a392 (Apache-2.0).

//! What a submitted text is (Codex's `slash_dispatch.rs` split): a command
//! with the text after its name when it takes any, a `/name` that names
//! none, or a message. The composer reads what is typed with it, and so is
//! what a preset or `/config` runs.

use super::{find, CommandSpec};

pub(crate) enum ParsedInput<'a> {
    /// The command, with its arguments (empty when it takes none).
    Command(&'static CommandSpec, String),
    /// `/name` naming no command: not sent.
    Unrecognized(&'a str),
    /// A message, as it is: text that is no command, text after a command
    /// that takes none, a leading space, a path with a slash in its name.
    Message,
}

pub(crate) fn parse_input(text: &str) -> ParsedInput<'_> {
    let Some((name, rest)) = slash_name(text.lines().next().unwrap_or("")) else {
        return ParsedInput::Message;
    };
    match find(name) {
        Some(command) if command.inline_args => {
            let args = slash_name(text).map_or("", |(_, args)| args);
            ParsedInput::Command(command, args.trim_end().to_string())
        }
        Some(command) if rest.is_empty() => ParsedInput::Command(command, String::new()),
        Some(_) => ParsedInput::Message,
        None if name.contains('/') => ParsedInput::Message,
        None => ParsedInput::Unrecognized(name),
    }
}

/// The `/name` a line starts with and what follows it, trimmed at the start.
fn slash_name(line: &str) -> Option<(&str, &str)> {
    let stripped = line.strip_prefix('/')?;
    let name_end = stripped
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(stripped.len(), |(idx, _)| idx);
    let name = &stripped[..name_end];
    if name.is_empty() {
        return None;
    }
    Some((name, stripped[name_end..].trim_start()))
}
