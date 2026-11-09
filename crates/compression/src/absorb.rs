// Derived from billion-context (https://github.com/ranxianglei/billion-context),
// `kernel/src/absorb.ts` of acp-kernel 0.0.189 at commit `4be21b08e`.
// Copyright (c) 2026 ranxianglei. MIT License with an additional attribution
// term (NOTICE).
// Modified for hanafubuki-agent: the tool's name is fixed; the host passes
// the full history, so a target an active block covers is refused as not
// visible; tokens are the crate's estimate; the record's time is the
// caller's; a rejected argument returns its reason instead of logging it.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::model::{AbsorbRecord, Config, Kind, Message, State};
use crate::number::to_fixed;
use crate::prompts::{ABSORB_TOOL_NAME, ACP_TOOL_NAMES};
use crate::protect::{
    collect_failed_compress_call_ids, collect_latest_protected, collect_protected_tool_call_ids,
    is_message_latest_protected, is_message_protected_with_pairing, match_tool_pattern,
    LatestProtected,
};
use crate::refs::BLOCKED_REF;
use crate::render::default_count_tokens;
use crate::store::is_stored_placeholder_text;

/// Ends a tool result the model must absorb.
pub const ABSORB_PROMPT_MARKER: &str = "[ACP absorb]";

/// The demand appended to a tool result to absorb.
pub fn build_absorb_prompt(r#ref: &str, tokens: u64) -> String {
    let tool = ABSORB_TOOL_NAME;
    format!(
        "{ABSORB_PROMPT_MARKER} This tool result (~{} tokens) will be REMOVED from context. Your IMMEDIATE next action: call {tool}({{ ref: \"{ref}\", summary: \"...\" }}) — summary = distilled essentials only (outcome, key values, exact paths:lines, error text verbatim, decisions). Afterwards work from your summary; do NOT re-run this tool. If the result contains nothing you need, call {tool} with summary \"(nothing needed)\".",
        format_token_count(tokens)
    )
}

/// The system-prompt section the host adds while absorption is on.
pub fn build_absorb_system_prompt() -> String {
    let tool = ABSORB_TOOL_NAME;
    format!(
        "INSTANT TOOL-RESULT ABSORPTION ({tool})\n\nSome tool results end with a {ABSORB_PROMPT_MARKER} instruction. When you see one, your IMMEDIATE next action must be calling {tool}({{ ref, summary }}) — distill that tool result's essentials into summary: outcome, key values, exact paths:lines, error text verbatim, decisions. The original output is then removed from context; your {tool} summary becomes the only durable record of it, so distill carefully. Never call another tool or answer the user before absorbing a marked result. Do not re-run the original tool afterwards — work from your summary. {tool} calls are ordinary context: the regular compression system may fold them later like any other message."
    )
}

fn format_token_count(tokens: u64) -> String {
    if tokens < 1000 {
        tokens.to_string()
    } else if tokens < 10_000 {
        format!("{}K", to_fixed(tokens as f64 / 1000.0, 1))
    } else {
        format!("{}K", (tokens as f64 / 1000.0 + 0.5).floor())
    }
}

/// What a candidate check needs to know about the whole view.
struct Context<'a> {
    latest: Option<&'a LatestProtected>,
    protected_calls: &'a HashSet<String>,
    failed_calls: &'a HashSet<String>,
}

/// Whether a tool result may be asked to be absorbed: not a placeholder,
/// not an ACP tool's, not protected, not excluded, and with images when
/// only those are absorbed.
pub fn is_absorb_candidate(message: &Message, config: &Config) -> bool {
    is_candidate_with_protected_calls(message, config, &HashSet::new())
}

/// [`is_absorb_candidate`], with a result also protected through its call.
pub(crate) fn is_candidate_with_protected_calls(
    message: &Message,
    config: &Config,
    protected_calls: &HashSet<String>,
) -> bool {
    let context = Context {
        latest: None,
        protected_calls,
        failed_calls: &HashSet::new(),
    };
    is_candidate(message, config, &context)
}

fn is_candidate(message: &Message, config: &Config, context: &Context<'_>) -> bool {
    if message.kind != Kind::ToolResult
        || message.tool_call_id.is_none()
        || is_stored_placeholder_text(&message.text)
        || is_acp_tool(message.tool_name.as_deref())
        || is_message_protected_with_pairing(
            message,
            config,
            context.protected_calls,
            context.failed_calls,
        )
        || context
            .latest
            .is_some_and(|latest| is_message_latest_protected(message, latest))
    {
        return false;
    }
    let settings = &config.absorb;
    let excluded = message.tool_name.as_deref().is_some_and(|name| {
        settings
            .exclude_tools
            .iter()
            .any(|pattern| match_tool_pattern(name, pattern))
    });
    !excluded && !(settings.media_only && message.images.is_empty())
}

pub(crate) fn is_acp_tool(tool_name: Option<&str>) -> bool {
    tool_name.is_some_and(|name| name == ABSORB_TOOL_NAME || ACP_TOOL_NAMES.contains(&name))
}

/// Drops the call and result of every absorbed tool result; the model's
/// `absorb` call stays as the record.
pub fn hide_absorbed_messages(messages: Vec<Message>, state: &State) -> Vec<Message> {
    if state.absorbed.is_empty() {
        return messages;
    }
    let hidden: HashSet<&str> = state
        .absorbed
        .iter()
        .flat_map(|record| [&record.call_message_id, &record.result_message_id])
        .filter(|id| !id.is_empty())
        .map(String::as_str)
        .collect();
    messages
        .into_iter()
        .filter(|message| !hidden.contains(message.id.as_str()))
        .collect()
}

/// The view with the absorb demand on its large tool results.
#[derive(Clone, Debug, PartialEq)]
pub struct AbsorbPrompts {
    pub messages: Vec<Message>,
    pub prompted_count: usize,
}

/// Appends the absorb demand to every large tool result not absorbed yet.
/// It is view text, never persisted: it shows on each request until the
/// model absorbs the result, which then hides.
pub fn append_absorb_prompts(
    messages: Vec<Message>,
    state: &State,
    config: &Config,
    token_count: u64,
) -> AbsorbPrompts {
    let settings = &config.absorb;
    let limit = config.model_context_limit as f64;
    let below_threshold = settings.context_threshold_pct > 0.0
        && limit > 0.0
        && (token_count as f64) < settings.context_threshold_pct * limit;
    if !settings.enabled || below_threshold {
        return AbsorbPrompts {
            messages,
            prompted_count: 0,
        };
    }
    let absorbed: HashSet<&str> = state
        .absorbed
        .iter()
        .map(|record| record.result_message_id.as_str())
        .collect();
    let latest = collect_latest_protected(&messages, config);
    let protected_calls = collect_protected_tool_call_ids(&messages, config);
    let failed_calls = collect_failed_compress_call_ids(&messages);
    let context = Context {
        latest: Some(&latest),
        protected_calls: &protected_calls,
        failed_calls: &failed_calls,
    };
    let mut prompted_count = 0;
    let mut out = Vec::with_capacity(messages.len());
    for message in &messages {
        let prompt = (is_candidate(message, config, &context)
            && !absorbed.contains(message.id.as_str())
            && !message.text.contains(ABSORB_PROMPT_MARKER))
        .then(|| demand(message, state, config))
        .flatten();
        match prompt {
            Some(prompt) => {
                prompted_count += 1;
                out.push(Message {
                    text: format!("{}\n\n{prompt}", message.text),
                    ..message.clone()
                });
            }
            None => out.push(message.clone()),
        }
    }
    AbsorbPrompts {
        messages: out,
        prompted_count,
    }
}

fn demand(message: &Message, state: &State, config: &Config) -> Option<String> {
    let tokens = default_count_tokens(&message.text);
    // An image is not in the text, so a media result is asked for whatever
    // its text's size.
    let media_exempt = config.absorb.media_only && !message.images.is_empty();
    if !media_exempt && tokens < config.absorb.min_tool_tokens {
        return None;
    }
    let r#ref = state
        .message_refs
        .by_raw
        .get(&message.id)
        .filter(|r#ref| *r#ref != BLOCKED_REF)?;
    Some(build_absorb_prompt(r#ref, tokens))
}

/// An `absorb` call's arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedAbsorb {
    pub r#ref: String,
    pub summary: String,
}

/// Reads an `absorb` call's arguments leniently: `ref`, `messageId` or `of`
/// for the ref, `summary` or `content` for the summary, and a JSON string
/// holding the object for providers that stringify it. The error is the
/// reason, for the host's log.
pub fn parse_absorb_input(input: &Value) -> Result<ParsedAbsorb, String> {
    let parsed;
    let object = match input {
        Value::Object(object) => object,
        Value::String(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => {
                parsed = object;
                &parsed
            }
            _ => return Err(not_an_object("string")),
        },
        other => return Err(not_an_object(js_type(other))),
    };
    match (
        pick_string(object, &["ref", "messageId", "of"]),
        pick_string(object, &["summary", "content"]),
    ) {
        (Some(r#ref), Some(summary)) => Ok(ParsedAbsorb {
            r#ref: r#ref.trim().to_owned(),
            summary: summary.to_owned(),
        }),
        _ => Err(format!(
            "[acp-absorb-input] rejected: need ref (string) + summary (string); keys: {}",
            object.keys().cloned().collect::<Vec<_>>().join(",")
        )),
    }
}

fn not_an_object(kind: &str) -> String {
    format!("[acp-absorb-input] rejected: not an object ({kind})")
}

fn js_type(value: &Value) -> &'static str {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

fn pick_string<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| object.get(*key)?.as_str())
}

/// One `absorb` call.
pub struct AbsorbInput<'a> {
    pub r#ref: &'a str,
    pub summary: &'a str,
    /// The tool-call id of the `absorb` call.
    pub absorb_call_id: Option<&'a str>,
    /// The full history.
    pub messages: &'a [Message],
    pub state: &'a State,
    pub config: &'a Config,
    /// Milliseconds since the Unix epoch, for the record.
    pub now_ms: u64,
}

/// The state to persist and the tool result for the model.
#[derive(Clone, Debug, PartialEq)]
pub struct AbsorbOutcome {
    pub state: State,
    pub ok: bool,
    pub result_text: String,
}

/// Applies an `absorb` call: checks the target tool result, records it and
/// reports. The pair hides from the next view on; the call is an action of
/// its own, so it releases the view latch and the hide is not held back
/// until a fold.
pub fn apply_absorb(input: AbsorbInput<'_>) -> AbsorbOutcome {
    let refuse = |text: String| AbsorbOutcome {
        state: input.state.clone(),
        ok: false,
        result_text: text,
    };
    let summary = input.summary.trim();
    if summary.is_empty() {
        return refuse(
            "absorb failed: summary is empty — provide the distilled key info of the tool result."
                .to_owned(),
        );
    }
    let r#ref = input.r#ref.trim();
    let target = match find_target(&input, r#ref) {
        Ok(target) => target,
        Err(Rejection::Refused(text)) => return refuse(text),
        Err(Rejection::AlreadyAbsorbed) => {
            return AbsorbOutcome {
                state: input.state.clone(),
                ok: true,
                result_text: format!("already absorbed ({}) — no change.", input.r#ref),
            }
        }
    };
    let call = input.messages.iter().find(|message| {
        message.kind == Kind::ToolCall && message.tool_call_id == target.tool_call_id
    });
    let tokens = default_count_tokens(&target.text);
    let summary_tokens = default_count_tokens(summary);
    let mut state = input.state.clone();
    state.absorbed.push(AbsorbRecord {
        tool_call_id: target.tool_call_id.clone().unwrap_or_default(),
        call_message_id: call.map(|call| call.id.clone()).unwrap_or_default(),
        result_message_id: target.id.clone(),
        absorb_call_id: input.absorb_call_id.map(str::to_owned),
        summary: summary.to_owned(),
        tokens_reclaimed: tokens,
        created_at: input.now_ms,
    });
    state.stats.absorbed_tokens += tokens;
    state.view_latches.clear();
    let bloat = if summary_tokens >= tokens && tokens > 0 {
        format!(
            " WARNING: your summary (~{} tokens) is not smaller than the original (~{} tokens) — distill harder next time.",
            format_token_count(summary_tokens),
            format_token_count(tokens)
        )
    } else {
        String::new()
    };
    AbsorbOutcome {
        state,
        ok: true,
        result_text: format!(
            "absorbed {} (~{} tokens → summary ~{}). The original tool output is now hidden; your summary is the durable record.{bloat}",
            input.r#ref,
            format_token_count(tokens),
            format_token_count(summary_tokens)
        ),
    }
}

enum Rejection {
    Refused(String),
    AlreadyAbsorbed,
}

fn find_target<'a>(input: &AbsorbInput<'a>, r#ref: &str) -> Result<&'a Message, Rejection> {
    let shown = input.r#ref;
    let refused = |text: String| Err(Rejection::Refused(text));
    let Some(raw_id) = input.state.message_refs.by_ref.get(r#ref) else {
        return refused(format!("absorb failed: ref {shown} does not exist in this session (it may be hidden, already compressed, or stale)."));
    };
    if input
        .state
        .absorbed
        .iter()
        .any(|record| &record.result_message_id == raw_id)
    {
        return Err(Rejection::AlreadyAbsorbed);
    }
    let covered = input.state.covered_message_ids().contains(raw_id.as_str());
    let Some(target) = input
        .messages
        .iter()
        .find(|message| &message.id == raw_id)
        .filter(|_| !covered)
    else {
        return refused(format!(
            "absorb failed: ref {shown} is not visible in this session (hidden or compressed)."
        ));
    };
    if target.kind != Kind::ToolResult {
        return refused(format!(
            "absorb failed: ref {shown} is a {}, not a tool result.",
            kind_name(target.kind)
        ));
    }
    let settings = &input.config.absorb;
    if settings.media_only && target.images.is_empty() {
        return refused(format!("absorb failed: {shown} carries no media payload — absorb is scoped to image-bearing results (compress.absorb.mediaOnly)."));
    }
    let tool = target.tool_name.as_deref().unwrap_or("undefined");
    if is_acp_tool(target.tool_name.as_deref()) {
        return refused(format!(
            "absorb failed: {tool} is an ACP-managed tool result — it is not absorbable."
        ));
    }
    let config = input.config;
    let messages = input.messages;
    if is_message_protected_with_pairing(
        target,
        config,
        &collect_protected_tool_call_ids(messages, config),
        &collect_failed_compress_call_ids(messages),
    ) || is_message_latest_protected(target, &collect_latest_protected(messages, config))
    {
        return refused(format!(
            "absorb failed: {tool} is a protected tool — its results must stay visible."
        ));
    }
    if target.tool_call_id.is_none() {
        return refused(format!(
            "absorb failed: ref {shown} has no tool-call id — cannot pair it for hiding."
        ));
    }
    Ok(target)
}

/// The kernel's content type names, as its refusals print them.
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "text",
        Kind::ToolCall => "tool-call",
        Kind::ToolResult => "tool-result",
        Kind::Reasoning => "reasoning",
    }
}
