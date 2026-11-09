// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// the host side of its `absorb`, `acp_retrieve` and `acp_rule` tools at
// commit `4be21b08e`. Copyright (c) 2026 ranxianglei. MIT License with an
// additional attribution term (NOTICE).
// Modified for hanafubuki-agent: the tools run on the session's record, and
// an export goes to the session's export file.

//! The opt-in features' tools, each planned only while its feature is on:
//! `absorb` distils a large tool result, `acp_retrieve` returns an original
//! the content store replaced, `acp_rule` records persistent reminders.

use hanafubuki_agent_compression::{
    apply_absorb, apply_retrieve, execute_rule, note_retrieval, parse_absorb_input, AbsorbInput,
    Retrieval,
};

use super::*;

pub(super) fn absorb(
    log: &MessageLog<'_>,
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    call_id: &str,
    arguments: &Value,
    now_ms: u64,
) -> CompressionToolRun {
    let parsed = match parse_absorb_input(arguments) {
        Ok(parsed) => parsed,
        Err(reason) => return output_run(text_output(reason, true), None),
    };
    let outcome = apply_absorb(AbsorbInput {
        r#ref: &parsed.r#ref,
        summary: &parsed.summary,
        absorb_call_id: Some(call_id),
        messages: &log.messages,
        state,
        config,
        now_ms,
    });
    // "Already absorbed" is a success that changes nothing.
    let recorded = outcome.state.absorbed.len() != state.absorbed.len();
    output_run(
        text_output(outcome.result_text, !outcome.ok),
        recorded.then_some(outcome.state),
    )
}

/// An original too large to inline is exported to the session's file
/// `<ref>.txt`, so the retrieval brings only a pointer back into the
/// context.
pub(super) fn retrieve(
    content_store: Option<&AgentContentStore>,
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    arguments: &Value,
) -> CompressionToolRun {
    let r#ref = arguments
        .get("ref")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let empty = AgentContentStore::default();
    let retrieval = apply_retrieve(
        content_store.unwrap_or(&empty),
        r#ref,
        Some(config.content_store.retrieve_inline_tokens),
    );
    let mut counted = state.clone();
    note_retrieval(&mut counted);
    match retrieval {
        Retrieval::NotFound { tool_result_text } => {
            output_run(text_output(tool_result_text, true), None)
        }
        Retrieval::Inline {
            tool_result_text, ..
        } => output_run(text_output(tool_result_text, false), Some(counted)),
        Retrieval::Export {
            entry,
            text,
            file_name,
            lines,
        } => CompressionToolRun {
            export: Some(ToolExport::retrieval(
                file_name,
                r#ref.trim().to_owned(),
                entry,
                text,
                lines,
            )),
            ..output_run(text_output(String::new(), false), Some(counted))
        },
    }
}

pub(super) fn rule(
    config: &hanafubuki_agent_compression::Config,
    state: &AgentCompressionState,
    arguments: &Value,
) -> CompressionToolRun {
    let mut next = state.clone();
    let result = execute_rule(arguments, &mut next, config);
    let changed = next.rules != state.rules || next.next_rule_id != state.next_rule_id;
    output_run(
        text_output(result.text, !result.ok),
        changed.then_some(next),
    )
}
