// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/filter/` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the host passes its filters with each call
// instead of registering them in a process-wide registry, and a filter
// cannot throw.

use crate::model::{Message, MessageFiltersConfig};

/// What a filter sees of one message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageFilterContext<'a> {
    /// The text as the earlier filters left it.
    pub text: &'a str,
    pub role: &'static str,
    pub message_index: usize,
    pub total_messages: usize,
    pub tool_name: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterAction {
    Keep,
    Modify(String),
    Drop,
}

/// A host's message filter. Its decision must depend on the context alone,
/// or the bytes already sent change.
pub trait MessageFilter {
    fn name(&self) -> &str;
    /// Only the last message it modifies or drops gets that decision; every
    /// earlier match is dropped.
    fn keep_last_only(&self) -> bool {
        false
    }
    fn filter(&self, context: &MessageFilterContext<'_>) -> FilterAction;
}

/// The view after the filters, and what they did.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterResult {
    pub messages: Vec<Message>,
    pub parts_filtered: usize,
    pub parts_dropped: usize,
    pub parts_modified: usize,
}

/// Runs the enabled filters over the view's texts: the immediate ones over
/// every message in order, then each keep-last-only one from the newest
/// message back. A dropped text becomes empty; the message stays.
pub fn apply_message_filters(
    messages: Vec<Message>,
    config: &MessageFiltersConfig,
    filters: &[&dyn MessageFilter],
) -> FilterResult {
    let active: Vec<&dyn MessageFilter> = filters
        .iter()
        .copied()
        .filter(|filter| config.filters.get(filter.name()) != Some(&false))
        .collect();
    let mut result = FilterResult {
        messages,
        parts_filtered: 0,
        parts_dropped: 0,
        parts_modified: 0,
    };
    if !config.enabled || active.is_empty() {
        return result;
    }
    let (keep_last, immediate): (Vec<_>, Vec<_>) = active
        .into_iter()
        .partition(|filter| filter.keep_last_only());
    let total = result.messages.len();
    for index in 0..total {
        let message = &result.messages[index];
        if message.text.is_empty() {
            continue;
        }
        let mut current = message.text.clone();
        for filter in &immediate {
            let context = context(message, &current, index, total);
            match filter.filter(&context) {
                FilterAction::Keep => continue,
                FilterAction::Drop => {
                    current.clear();
                    result.parts_dropped += 1;
                }
                FilterAction::Modify(text) => {
                    current = text;
                    result.parts_modified += 1;
                }
            }
            result.parts_filtered += 1;
        }
        if current != result.messages[index].text {
            result.messages[index].text = current;
        }
    }
    for filter in keep_last {
        apply_keep_last(&mut result, filter, total);
    }
    result
}

fn apply_keep_last(result: &mut FilterResult, filter: &dyn MessageFilter, total: usize) {
    let mut found_last = false;
    for index in (0..total).rev() {
        let message = &result.messages[index];
        if message.text.is_empty() {
            continue;
        }
        let decision = filter.filter(&context(message, &message.text, index, total));
        if decision == FilterAction::Keep {
            continue;
        }
        if found_last {
            result.parts_filtered += 1;
            result.parts_dropped += 1;
            result.messages[index].text.clear();
            continue;
        }
        found_last = true;
        if let FilterAction::Modify(text) = decision {
            result.parts_filtered += 1;
            result.parts_modified += 1;
            result.messages[index].text = text;
        }
    }
}

fn context<'a>(
    message: &'a Message,
    text: &'a str,
    index: usize,
    total: usize,
) -> MessageFilterContext<'a> {
    MessageFilterContext {
        text,
        role: message.role.name(),
        message_index: index,
        total_messages: total,
        tool_name: message.tool_name.as_deref(),
    }
}
