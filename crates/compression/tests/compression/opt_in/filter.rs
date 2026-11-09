// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// tests of `kernel/tests` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Ports the message-filter cases of `truncate-keep-filter.test.ts` (its
// truncation cases are in `truncate.rs`); adds a filter turned off by name
// and the step's place in a turn.

use std::collections::BTreeMap;

use hanafubuki_agent_compression::{
    apply_message_filters, Config, FilterAction, Message, MessageFilter, MessageFilterContext,
    MessageFiltersConfig, Role, State,
};

use super::{text_of, turn_with};
use hanafubuki_agent_compression::ContentStore;

/// A filter from a closure.
pub struct Filter<F> {
    pub name: &'static str,
    pub keep_last_only: bool,
    pub decide: F,
}

impl<F: Fn(&MessageFilterContext<'_>) -> FilterAction> MessageFilter for Filter<F> {
    fn name(&self) -> &str {
        self.name
    }

    fn keep_last_only(&self) -> bool {
        self.keep_last_only
    }

    fn filter(&self, context: &MessageFilterContext<'_>) -> FilterAction {
        (self.decide)(context)
    }
}

pub fn filters_on(names: &[&str]) -> MessageFiltersConfig {
    MessageFiltersConfig {
        enabled: true,
        filters: names
            .iter()
            .map(|name| ((*name).to_owned(), true))
            .collect(),
    }
}

fn message(id: &str, text: &str) -> Message {
    Message::text(id, Role::User, text)
}

fn drop_noise() -> Filter<impl Fn(&MessageFilterContext<'_>) -> FilterAction> {
    Filter {
        name: "drop-noise",
        keep_last_only: false,
        decide: |context: &MessageFilterContext<'_>| {
            if context.text.contains("NOISE") {
                FilterAction::Drop
            } else {
                FilterAction::Keep
            }
        },
    }
}

#[test]
fn off_the_filters_do_nothing() {
    let noise = drop_noise();
    let messages = vec![message("a", "hello NOISE")];
    let out = apply_message_filters(
        messages.clone(),
        &MessageFiltersConfig::default(),
        &[&noise],
    );
    assert_eq!(out.parts_filtered, 0);
    assert_eq!(out.messages, messages);
}

#[test]
fn an_immediate_filter_drops_matching_text() {
    let noise = drop_noise();
    let messages = vec![message("a", "clean"), message("b", "has NOISE inside")];
    let out = apply_message_filters(messages, &filters_on(&["drop-noise"]), &[&noise]);
    assert_eq!(out.parts_dropped, 1);
    assert_eq!(out.messages[1].text, "");
    assert_eq!(out.messages[0].text, "clean");
}

#[test]
fn a_keep_last_only_filter_keeps_the_last_match() {
    let dedup = Filter {
        name: "dedup",
        keep_last_only: true,
        decide: |context: &MessageFilterContext<'_>| {
            if context.text.starts_with("REPEAT:") {
                FilterAction::Modify(context.text.to_owned())
            } else {
                FilterAction::Keep
            }
        },
    };
    let messages = vec![
        message("a", "REPEAT:directive"),
        message("b", "normal"),
        message("c", "REPEAT:directive-updated"),
    ];
    let out = apply_message_filters(messages, &filters_on(&["dedup"]), &[&dedup]);
    assert_eq!(out.messages[0].text, "");
    assert_eq!(out.messages[1].text, "normal");
    assert_eq!(out.messages[2].text, "REPEAT:directive-updated");
}

#[test]
fn a_filter_turned_off_by_name_does_not_run() {
    let noise = drop_noise();
    let config = MessageFiltersConfig {
        enabled: true,
        filters: BTreeMap::from([("drop-noise".to_owned(), false)]),
    };
    let out = apply_message_filters(vec![message("a", "NOISE")], &config, &[&noise]);
    assert_eq!(out.messages[0].text, "NOISE");
}

#[test]
fn a_turn_runs_the_hosts_filters_when_on() {
    let noise = drop_noise();
    let messages = vec![message("a", "first"), message("b", "has NOISE inside")];
    let mut config = Config::new(100_000);
    let (off, _) = turn_with(
        &messages,
        State::default(),
        &config,
        0,
        ContentStore::default(),
        &[&noise],
    );
    assert_eq!(text_of(&off.messages, "b"), "has NOISE inside");
    config.message_filters = filters_on(&["drop-noise"]);
    let (on, _) = turn_with(
        &messages,
        State::default(),
        &config,
        0,
        ContentStore::default(),
        &[&noise],
    );
    assert_eq!(text_of(&on.messages, "b"), "");
}
