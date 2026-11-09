// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/view-latch.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: no transform has a configuration yet, so
// the configuration digest is constant until the opt-in transforms add
// theirs.

use std::collections::{BTreeMap, HashMap};

use crate::model::{Config, Message, State, ViewLatch};
use crate::refs::BLOCKED_REF;

/// What the latch is released by when it changes: the configuration of the
/// transforms between the pre-transform snapshot and the latch. The token
/// count is deliberately not in it; pressure is what the latch gates.
pub fn view_latch_config_digest(_config: &Config) -> String {
    "{}".to_owned()
}

/// The fold-gated view latch. Whatever the transforms made of a message
/// (rewritten, hidden or left alone) the first time it was seen is replayed
/// on every later request until a fold (which clears the latch) or a change
/// of the transforms' configuration, so a transform re-deciding with the
/// context never rewrites old bytes on a quiet request. `pre` is the view
/// before the transforms, `post` after them; messages without a numbered
/// ref pass through.
pub fn apply_view_latch(
    pre: &[Message],
    post: Vec<Message>,
    state: &mut State,
    config_digest: &str,
) -> Vec<Message> {
    let config_changed = state.view_latch_config_digest.as_deref() != Some(config_digest);
    // A fold clears the latches and keeps the digest: nothing to replay.
    let previous =
        (!config_changed && !state.view_latches.is_empty()).then(|| state.view_latches.clone());
    let transformed: HashMap<&str, &Message> = post
        .iter()
        .map(|message| (message.id.as_str(), message))
        .collect();

    let mut out = Vec::with_capacity(pre.len());
    let mut latches: BTreeMap<String, ViewLatch> = BTreeMap::new();
    let mut latched_transform = false;
    for message in pre {
        let transformed = transformed.get(message.id.as_str()).copied();
        let Some(r#ref) = state
            .message_refs
            .by_raw
            .get(&message.id)
            .filter(|r#ref| *r#ref != BLOCKED_REF)
        else {
            out.extend(transformed.cloned());
            continue;
        };
        match previous.as_ref().and_then(|latches| latches.get(r#ref)) {
            Some(latch) => {
                // Replay: a transform's fresh output waits for the next fold.
                if !latch.hidden {
                    match &latch.text {
                        None => out.extend(transformed.map(|_| message.clone())),
                        Some(text) => {
                            latched_transform = true;
                            out.push(match transformed {
                                Some(transformed) if &transformed.text == text => {
                                    transformed.clone()
                                }
                                _ => Message {
                                    text: text.clone(),
                                    ..message.clone()
                                },
                            });
                        }
                    }
                }
                latches.insert(r#ref.clone(), latch.clone());
            }
            None => {
                // First sight is a history change of its own: latch what the
                // transforms made of the message now.
                let latch = match transformed {
                    None => ViewLatch {
                        text: None,
                        hidden: true,
                    },
                    Some(transformed) if transformed.text != message.text => {
                        latched_transform = true;
                        out.push(transformed.clone());
                        ViewLatch {
                            text: Some(transformed.text.clone()),
                            hidden: false,
                        }
                    }
                    Some(_) => {
                        out.push(message.clone());
                        ViewLatch::default()
                    }
                };
                latches.insert(r#ref.clone(), latch);
            }
        }
    }

    // A steady state leaves the state as it was, and with no transform
    // output to pin there is nothing to record.
    if config_changed || state.view_latches != latches {
        if previous.is_none() && !latched_transform {
            return post;
        }
        state.view_latches = latches;
        state.view_latch_config_digest = Some(config_digest.to_owned());
    }
    out
}
