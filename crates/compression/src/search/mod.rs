// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/search.ts`, `kernel/src/search/index.ts` and
// `kernel/src/search/types.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: search is synchronous with a caller-owned
// feature cache; a message's role is the crate's own (a system message
// weighs as a tool's); `folded_message_inputs` builds the folded originals
// from the full history.

mod cache;
mod score;
mod tokenizer;

use std::collections::HashMap;

pub use cache::{DocCache, DocCacheInfo, DocFeatures, DEFAULT_DOC_CACHE_CAP};
pub use score::Algorithm;
pub use tokenizer::{char_bigrams, stem_word, term_frequencies, tokenize};

use crate::model::{Block, Message, Role, State};
use crate::refs::BLOCKED_REF;
use crate::render::count_message_tokens;
use crate::text::{clamp_prefix, clamp_window, utf16_len};
use tokenizer::is_space;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDocKind {
    Block,
    Message,
}

/// A document search scores: a block's topic and summary, or a message's
/// original text.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchDoc {
    pub kind: SearchDocKind,
    /// `b3` for a block, `m00350` for a message.
    pub r#ref: String,
    pub text: String,
    pub title: String,
    /// A message's role, which weighs its score.
    pub role: Option<Role>,
    /// The block itself, or the block that folded the message: the one to
    /// decompress for detail.
    pub block_id: Option<String>,
    pub tier: Option<u8>,
    pub tokens: Option<u64>,
}

/// A message the host offers to search.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageInput {
    pub r#ref: String,
    pub role: Role,
    pub text: String,
    pub tokens: Option<u64>,
    /// The block that folded it; `None` while it is visible.
    pub block_id: Option<String>,
    pub tier: Option<u8>,
}

/// Score multipliers: the user's intent over the assistant's reasoning
/// over tool noise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoleWeights {
    pub user: f64,
    pub assistant: f64,
    /// Tool and system messages.
    pub tool: f64,
    pub block: f64,
}

impl Default for RoleWeights {
    fn default() -> Self {
        Self {
            user: 1.5,
            assistant: 1.0,
            tool: 0.6,
            block: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchOptions {
    pub algorithm: Algorithm,
    pub limit: usize,
    /// In UTF-16 code units.
    pub preview_length: usize,
    pub min_score: f64,
    pub role_weights: RoleWeights,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            algorithm: Algorithm::Hybrid,
            limit: 10,
            preview_length: 200,
            min_score: 0.01,
            role_weights: RoleWeights::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchResult {
    pub kind: SearchDocKind,
    /// The ref to decompress.
    pub r#ref: String,
    pub block_id: Option<String>,
    pub tier: u8,
    pub score: f64,
    pub title: String,
    /// The text around the first query term found.
    pub preview: String,
    pub role: Option<Role>,
    pub tokens: Option<u64>,
}

/// Every block, active or not: a consumed block's summary is history the
/// model can still act on.
pub fn block_docs(state: &State) -> Vec<SearchDoc> {
    state
        .blocks
        .iter()
        .map(|block| SearchDoc {
            kind: SearchDocKind::Block,
            r#ref: block.block_id.clone(),
            text: format!(
                "{} {}",
                block.topic.as_deref().unwrap_or_default(),
                block.summary
            ),
            title: block
                .topic
                .clone()
                .unwrap_or_else(|| block.block_id.clone()),
            role: None,
            block_id: Some(block.block_id.clone()),
            tier: Some(block.tier),
            tokens: Some(block.compressed_tokens),
        })
        .collect()
}

pub fn message_docs(inputs: &[MessageInput]) -> Vec<SearchDoc> {
    inputs
        .iter()
        .map(|input| SearchDoc {
            kind: SearchDocKind::Message,
            r#ref: input.r#ref.clone(),
            text: input.text.clone(),
            title: format!("{}: {}", input.role.name(), clamp_prefix(&input.text, 60)),
            role: Some(input.role),
            block_id: input.block_id.clone(),
            tier: input.tier,
            tokens: input.tokens,
        })
        .collect()
}

/// The messages active blocks have folded out of the view, with their refs
/// and the block that holds each, in history order: the originals a search
/// finds detail in that no summary kept.
pub fn folded_message_inputs(state: &State, messages: &[Message]) -> Vec<MessageInput> {
    let owner: HashMap<&str, &Block> = state
        .active_blocks()
        .flat_map(|block| {
            block
                .effective_message_ids
                .iter()
                .map(move |id| (id.as_str(), block))
        })
        .collect();
    messages
        .iter()
        .filter_map(|message| {
            let block = owner.get(message.id.as_str())?;
            let r#ref = state
                .message_refs
                .by_raw
                .get(&message.id)
                .filter(|r#ref| *r#ref != BLOCKED_REF)?;
            Some(MessageInput {
                r#ref: r#ref.clone(),
                role: message.role,
                text: message.text.clone(),
                tokens: Some(count_message_tokens(message)),
                block_id: Some(block.block_id.clone()),
                tier: Some(block.tier),
            })
        })
        .collect()
}

/// Scores `docs` against `query` and returns the best `limit` at or above
/// `min_score`, best first.
pub fn search_blocks(
    docs: &[SearchDoc],
    query: &str,
    options: &SearchOptions,
    cache: &mut DocCache,
) -> Vec<SearchResult> {
    if docs.is_empty() {
        return Vec::new();
    }
    let scores = score::score(options.algorithm, docs, query, cache);
    let mut scored: Vec<(&SearchDoc, f64)> = docs
        .iter()
        .zip(scores)
        .map(|(doc, score)| (doc, score * role_weight(doc, &options.role_weights)))
        .filter(|(_, score)| *score >= options.min_score)
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(options.limit);
    scored
        .into_iter()
        .map(|(doc, score)| SearchResult {
            kind: doc.kind,
            r#ref: doc.r#ref.clone(),
            block_id: doc.block_id.clone(),
            tier: doc.tier.unwrap_or(1),
            score,
            title: doc.title.clone(),
            preview: make_preview(&doc.text, query, options.preview_length),
            role: doc.role,
            tokens: doc.tokens,
        })
        .collect()
}

fn role_weight(doc: &SearchDoc, weights: &RoleWeights) -> f64 {
    match (doc.kind, doc.role) {
        (SearchDocKind::Block, _) => weights.block,
        (SearchDocKind::Message, Some(Role::User)) => weights.user,
        (SearchDocKind::Message, Some(Role::Assistant)) => weights.assistant,
        (SearchDocKind::Message, _) => weights.tool,
    }
}

/// About `length` units of `text` centered on the first query term it
/// holds (terms of two or more units, tried in query order); its head when
/// none.
fn make_preview(text: &str, query: &str, length: usize) -> String {
    if text.is_empty() {
        return String::new();
    }
    let query = query.to_lowercase();
    let lower = text.to_lowercase();
    let hit = query
        .trim()
        .split(is_space)
        .filter(|term| utf16_len(term) > 1)
        .find_map(|term| lower.find(term));
    let Some(hit) = hit else {
        return clamp_prefix(text, length).to_owned();
    };
    // The offset is taken in the lowercased text and applied to the
    // original, as the kernel's `indexOf` does.
    let hit = utf16_len(&lower[..hit]);
    let total = utf16_len(text);
    let half = (length / 2).saturating_sub(10);
    let start = hit.saturating_sub(half);
    let end = total.min(start + length);
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < total { "…" } else { "" };
    format!("{prefix}{}{suffix}", clamp_window(text, start, end).trim())
}
