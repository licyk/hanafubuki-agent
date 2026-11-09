// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `executeSearchContext`, `evidenceSnippet`, `pickAnchorTerm`,
// `markFirstMatch`, `poolTermFrequency` and `narrowingHint` in
// `src/decompress-shared.ts` at commit `4be21b08e`. Copyright (c) 2026
// ranxianglei. MIT License with an additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the folded originals are the history's own
// messages (the crate's `folded_message_inputs`), not a per-block cache, so a
// hit names the message's ref; no plan-aware steering (the host's is off
// unless configured, and the Agent has no such setting), no derived or
// foreign sessions.

//! `search_context`: the block summaries, active and folded away, then the
//! folded originals no summary line already listed.

use std::collections::{HashMap, HashSet};

use hanafubuki_agent_compression::{
    block_docs, folded_message_inputs, message_docs, search_blocks, Algorithm, Block, DocCache,
    SearchOptions, SearchResult,
};

use super::decompress::covered_messages;
use super::*;

const SNIPPET_TOTAL: usize = 200;
const SNIPPET_AHEAD: usize = 80;

/// Every block, active or folded away, ranked by the literal occurrences of
/// the query's terms, then the folded messages the same way. Substring with
/// a floor of almost zero, as billion-context's host searches: one literal
/// occurrence is a hit and none is no match, where the hybrid search's
/// fuzzy n-grams would turn a query that matches nothing into weak matches.
pub(super) fn search_context(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    arguments: &Value,
    cache: &mut DocCache,
) -> CompressionToolRun {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if query.is_empty() {
        return output_run(
            text_output(
                "[search_context FAILED: query is required]".to_owned(),
                true,
            ),
            None,
        );
    }
    read_only(search(log, state, query, arguments, cache))
}

fn search(
    log: &MessageLog<'_>,
    state: &AgentCompressionState,
    query: &str,
    arguments: &Value,
    cache: &mut DocCache,
) -> String {
    let limit = positive_number(arguments.get("limit")).unwrap_or(5);
    let terms = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let options = |limit| SearchOptions {
        algorithm: Algorithm::Substring,
        limit,
        min_score: 1e-9,
        ..SearchOptions::default()
    };
    let ranked = search_blocks(
        &block_docs(state),
        query,
        &options(state.blocks.len()),
        cache,
    )
    .iter()
    .filter_map(|result| state.block(&result.r#ref))
    .collect::<Vec<_>>();
    let frequency = pool_term_frequency(&ranked, &terms);
    let blocks = &ranked[..ranked.len().min(limit)];
    let originals = original_hits(log, state, query, blocks, limit, cache);
    if blocks.is_empty() && originals.is_empty() {
        if state.blocks.is_empty() {
            return "[No compressed blocks exist yet — nothing to search.]".to_owned();
        }
        return format!(
            "[No blocks matched \"{query}\" among {} block(s). Scope = compressed topics/summaries (active + folded-away) + the folded original text — NOT visible messages or other sessions; absence here ≠ never-discussed.]",
            state.blocks.len()
        );
    }
    let lines = blocks
        .iter()
        .map(|block| {
            let topic = block.topic.as_deref().unwrap_or("(no topic)");
            let preview = evidence_snippet(&block.summary, &terms, &frequency);
            let span = covered_messages(state, block)
                .map(|covered| format!(" [{} · {} msgs]", covered.text, covered.raws.len()))
                .unwrap_or_default();
            let inactive = if block.active {
                String::new()
            } else {
                format!(
                    " [inactive — folded away; decompress {} restores it]",
                    block.block_id
                )
            };
            format!(
                "{} (T{}) \"{topic}\"{span}{inactive}\n  {preview}",
                block.block_id, block.tier
            )
        })
        .collect::<Vec<_>>();
    let truncated = if ranked.len() > blocks.len() {
        format!(
            "\n\n[truncated: showing {} of {} candidates by relevance — pass limit:{} to see the rest]",
            blocks.len(),
            ranked.len(),
            ranked.len()
        )
    } else {
        String::new()
    };
    let tip = narrowing_hint(&terms, blocks, &ranked)
        .map(|tip| format!("\n\n{tip}"))
        .unwrap_or_default();
    let original_section = if originals.is_empty() {
        String::new()
    } else {
        let lines = originals
            .iter()
            .map(|(block, hit)| {
                let span = covered_messages(state, block)
                    .map(|covered| format!(" [{} · {} msgs]", covered.text, covered.raws.len()))
                    .unwrap_or_default();
                format!(
                    "{} (original text, not in any summary, at {}){span}\n  {}",
                    block.block_id, hit.r#ref, hit.preview
                )
            })
            .collect::<Vec<_>>();
        format!(
            "\n\nOriginal-text hit(s) — verbatim folded content that no summary mentions; decompress the block (or the range around the ref) for the full record:\n{}",
            lines.join("\n")
        )
    };
    let header = if blocks.is_empty() {
        format!(
            "Found 0 summary block(s) for \"{query}\" — but {} folded original(s) match:",
            originals.len()
        )
    } else {
        format!("Found {} block(s) for \"{query}\":", blocks.len())
    };
    format!(
        "{header}\n\n{}{truncated}{tip}{original_section}",
        lines.join("\n\n")
    )
}

/// The best folded message of each block the summary lines did not list,
/// at most `limit`.
fn original_hits<'a>(
    log: &MessageLog<'_>,
    state: &'a AgentCompressionState,
    query: &str,
    listed: &[&Block],
    limit: usize,
    cache: &mut DocCache,
) -> Vec<(&'a Block, SearchResult)> {
    let docs = message_docs(&folded_message_inputs(state, &log.messages));
    let options = SearchOptions {
        algorithm: Algorithm::Substring,
        limit: docs.len(),
        min_score: 1e-9,
        ..SearchOptions::default()
    };
    let mut seen = listed
        .iter()
        .map(|block| block.block_id.clone())
        .collect::<HashSet<_>>();
    let mut hits = Vec::new();
    for hit in search_blocks(&docs, query, &options, cache) {
        if hits.len() >= limit {
            break;
        }
        let Some(block) = hit.block_id.as_deref().and_then(|id| state.block(id)) else {
            continue;
        };
        if seen.insert(block.block_id.clone()) {
            hits.push((block, hit));
        }
    }
    hits
}

/// How many of `pool`'s blocks hold each term.
fn pool_term_frequency(pool: &[&Block], terms: &[String]) -> HashMap<String, usize> {
    let haystacks = pool.iter().map(|block| haystack(block)).collect::<Vec<_>>();
    terms
        .iter()
        .filter(|term| !term.is_empty())
        .map(|term| {
            let count = haystacks
                .iter()
                .filter(|haystack| haystack.contains(term.as_str()))
                .count();
            (term.clone(), count)
        })
        .collect()
}

/// About 200 units of `summary` around its most distinctive query term (the
/// one the fewest blocks hold, the earliest on a tie), marked 【】; its head
/// when no term is in it (the hit is then in the topic, shown already).
fn evidence_snippet(summary: &str, terms: &[String], frequency: &HashMap<String, usize>) -> String {
    let lower = summary.to_lowercase();
    let anchor = terms
        .iter()
        .filter(|term| !term.is_empty())
        .filter_map(|term| {
            let index = utf16_len(&lower[..lower.find(term.as_str())?]);
            Some((frequency.get(term).copied().unwrap_or(0), index, term))
        })
        .min_by_key(|(count, index, _)| (*count, *index));
    let total = utf16_len(summary);
    let Some((_, index, term)) = anchor else {
        return if total > SNIPPET_TOTAL {
            format!("{}...", utf16_window(summary, 0, SNIPPET_TOTAL))
        } else {
            summary.to_owned()
        };
    };
    let start = index.saturating_sub(SNIPPET_AHEAD);
    let end = total.min(start + SNIPPET_TOTAL);
    let window = mark_first_match(utf16_window(summary, start, end), term);
    format!(
        "{}{window}{}",
        if start > 0 { "... " } else { "" },
        if end < total { " ..." } else { "" }
    )
}

fn mark_first_match(text: &str, needle: &str) -> String {
    let lower = text.to_lowercase();
    let Some(found) = lower.find(needle) else {
        return text.to_owned();
    };
    let start = utf16_len(&lower[..found]);
    let end = start + utf16_len(needle);
    let total = utf16_len(text);
    format!(
        "{}\u{3010}{}\u{3011}{}",
        utf16_window(text, 0, start),
        utf16_window(text, start, end),
        utf16_window(text, end, total)
    )
}

/// For a query of three or more terms: a term no returned block holds that
/// lower-ranked candidates do (the fewest, the longest on a tie), so the
/// model narrows to it rather than reading its absence as never discussed.
fn narrowing_hint(terms: &[String], returned: &[&Block], ranked: &[&Block]) -> Option<String> {
    if terms.len() < 3 {
        return None;
    }
    let lower = &ranked[returned.len()..];
    if lower.is_empty() {
        return None;
    }
    let holds = |block: &Block, term: &str| haystack(block).contains(term);
    let (term, count) = terms
        .iter()
        .filter(|term| !term.is_empty())
        .filter(|term| !returned.iter().any(|block| holds(block, term)))
        .map(|term| {
            let count = lower.iter().filter(|block| holds(block, term)).count();
            (term, count)
        })
        .filter(|(_, count)| *count > 0)
        .min_by_key(|(term, count)| (*count, std::cmp::Reverse(utf16_len(term))))?;
    Some(format!(
        "[tip: none of the top {} results contain \"{term}\" but {count} lower-ranked candidate(s) do — try searching \"{term}\" alone to surface its block directly]",
        returned.len()
    ))
}

fn haystack(block: &Block) -> String {
    format!(
        "{}\n{}",
        block.topic.as_deref().unwrap_or_default(),
        block.summary
    )
    .to_lowercase()
}
