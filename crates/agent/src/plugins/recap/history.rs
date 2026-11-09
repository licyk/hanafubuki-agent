// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/tui/src/app/recap_history.rs` at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the exchanges are collected from the
// session's committed messages, handed over newest first, instead of the
// interface's history cells; the byte limit is the recap prompt's own.

//! The conversation a recap reads: the latest exchanges of user requests
//! and assistant answers, never tool output or reasoning. A newer
//! unanswered request is kept once; excerpts keep both ends of a text
//! within the byte limit.

use std::ops::ControlFlow;

use super::prompt::history_max_bytes;
use crate::conversation_reader::ConversationMessage;

/// How many answered exchanges a recap reads.
pub(super) const RECAP_HISTORY_MAX_TURNS: usize = 8;
const OMITTED_HISTORY: &str = "[Earlier exchanges omitted]\n\n";
const EXCERPT_MARKER: &str = "\n[... excerpted ...]\n";

/// A user's request (adjacent messages and steering joined) and what the
/// assistant answered it with.
#[derive(Default)]
pub(super) struct Exchange {
    user: String,
    assistant: String,
}

impl Exchange {
    fn fields(&self) -> impl Iterator<Item = (&'static str, &str)> {
        let user_label = if self.assistant.is_empty() {
            "Pending user request"
        } else {
            "User"
        };
        [
            (user_label, self.user.as_str()),
            ("Assistant", self.assistant.as_str()),
        ]
        .into_iter()
        .filter(|(_, text)| !text.is_empty())
    }
}

/// The latest exchanges, gathered from messages handed over newest first.
#[derive(Default)]
pub(super) struct RecentExchanges {
    exchanges: Vec<Exchange>,
    current: Exchange,
    answered: usize,
}

impl RecentExchanges {
    /// Takes the next older message; breaks once enough exchanges are
    /// complete.
    pub(super) fn push(&mut self, message: ConversationMessage) -> ControlFlow<()> {
        let (is_user, text) = match message {
            ConversationMessage::User(text) => (true, text),
            ConversationMessage::Assistant(text) => (false, text),
        };
        let mut content = text.trim().to_owned();
        if content.is_empty() {
            return ControlFlow::Continue(());
        }
        // Newest first, an answer before a request belongs to the
        // preceding exchange.
        if !is_user && !self.current.user.is_empty() {
            self.answered += usize::from(!self.current.assistant.is_empty());
            self.exchanges.push(std::mem::take(&mut self.current));
            if self.answered == RECAP_HISTORY_MAX_TURNS {
                return ControlFlow::Break(());
            }
        }
        let field = if is_user {
            &mut self.current.user
        } else {
            &mut self.current.assistant
        };
        if !field.is_empty() {
            content.push_str("\n\n");
            content.push_str(field);
        }
        *field = content;
        ControlFlow::Continue(())
    }

    /// The exchanges, oldest first.
    pub(super) fn finish(mut self) -> Vec<Exchange> {
        if !self.current.user.is_empty() {
            self.exchanges.push(self.current);
        }
        self.exchanges.reverse();
        self.exchanges
    }
}

/// `exchanges` as the recap prompt's conversation, within its byte limit;
/// empty when there is none.
pub(super) fn recap_history(exchanges: &[Exchange]) -> String {
    let Some(latest) = exchanges.last() else {
        return String::new();
    };
    let max_bytes = history_max_bytes();
    let blocks = exchanges
        .iter()
        .map(|exchange| {
            exchange
                .fields()
                .map(|(label, text)| format!("{label}: {text}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .collect::<Vec<_>>();
    let mut bytes = blocks.iter().map(String::len).sum::<usize>() + 2 * (blocks.len() - 1);
    if bytes <= max_bytes {
        return blocks.join("\n\n");
    }

    // Keep the newest answer and any newer unanswered correction together.
    let retained = if latest.assistant.is_empty() { 2 } else { 1 };
    let oldest_retained = exchanges.len().saturating_sub(retained);
    let mut start = 0;
    while bytes > max_bytes - OMITTED_HISTORY.len() && start < oldest_retained {
        bytes -= blocks[start].len() + 2;
        start += 1;
    }
    let omission = if start > 0 { OMITTED_HISTORY } else { "" };
    let budget = max_bytes - omission.len();
    if bytes <= budget {
        return format!("{omission}{}", blocks[start..].join("\n\n"));
    }

    let fields = exchanges[start..]
        .iter()
        .flat_map(Exchange::fields)
        .collect::<Vec<_>>();
    let field_count = fields.len();
    let overhead = fields
        .iter()
        .map(|(label, _)| label.len() + 2)
        .sum::<usize>()
        + 2 * (field_count - 1);
    let mut remaining = budget.saturating_sub(overhead);
    let excerpts = fields
        .iter()
        .enumerate()
        .map(|(index, (label, text))| {
            let share = remaining / (field_count - index);
            // Reserve a share for later fields, without wasting space on
            // short replies.
            let reserved = fields[index + 1..]
                .iter()
                .map(|(_, text)| text.len().min(share))
                .sum::<usize>();
            let excerpt = excerpt(text, remaining - reserved);
            remaining -= excerpt.len();
            format!("{label}: {excerpt}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("{omission}{excerpts}")
}

/// `text` within `max_bytes`, its middle cut out when it is longer.
fn excerpt(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let Some(content_bytes) = max_bytes.checked_sub(EXCERPT_MARKER.len()) else {
        return text[..text.floor_char_boundary(max_bytes)].to_owned();
    };
    let head = text.floor_char_boundary(content_bytes / 2);
    let tail = text.ceil_char_boundary(text.len() - (content_bytes - content_bytes / 2));
    format!("{}{EXCERPT_MARKER}{}", &text[..head], &text[tail..])
}

#[cfg(test)]
mod tests;
