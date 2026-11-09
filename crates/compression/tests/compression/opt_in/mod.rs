// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the contract tests of the opt-in features in `kernel/tests` of acp-kernel
// 0.0.189 at commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License
// with an additional attribution term (NOTICE).

mod absorb;
mod crush;
mod crush_turn;
mod debt;
mod defaults;
mod filter;
mod latch;
mod rules;
mod store;
mod store_turn;

use hanafubuki_agent_compression::{
    process_turn_with, Config, ContentStore, Message, MessageFilter, RenderStrategy, State,
    TurnExtras, TurnInput, TurnOutput,
};

/// One request with the content store and filters, tags off unless asked.
pub fn turn_with(
    messages: &[Message],
    state: State,
    config: &Config,
    token_count: u64,
    content_store: ContentStore,
    filters: &[&dyn MessageFilter],
) -> (TurnOutput, ContentStore) {
    process_turn_with(
        TurnInput {
            messages,
            state,
            config,
            token_count,
            token_count_source: None,
            render: RenderStrategy::None,
            clock: None,
        },
        TurnExtras {
            content_store,
            filters,
        },
    )
}

/// One request with an empty content store and no filters.
pub fn turn(messages: &[Message], state: State, config: &Config, token_count: u64) -> TurnOutput {
    turn_with(
        messages,
        state,
        config,
        token_count,
        ContentStore::default(),
        &[],
    )
    .0
}

pub fn text_of<'a>(messages: &'a [Message], id: &str) -> &'a str {
    messages
        .iter()
        .find(|message| message.id == id)
        .map_or("", |message| message.text.as_str())
}

pub fn has(messages: &[Message], id: &str) -> bool {
    messages.iter().any(|message| message.id == id)
}
