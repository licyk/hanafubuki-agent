// Derived from OpenAI Codex (https://github.com/openai/codex),
// `codex-rs/core/src/tasks/review.rs` (reading the reviewer's answer as
// JSON, the first JSON object in it, else its text as the explanation),
// `codex-rs/protocol/src/review_format.rs` (the findings as plain text),
// `codex-rs/prompts/src/review_exit.rs` and
// `codex-rs/prompts/templates/review/exit_success.xml` and
// `exit_interrupted.xml` (what the parent conversation is told when the
// review ends) at commit `57a808a392`.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0
// (http://www.apache.org/licenses/LICENSE-2.0).
// Modified for hanafubuki-agent: the answer is read field by field, so a
// finding that does not read is left out and the others kept, a missing
// field takes its default, and a finding may lack a location or a
// priority; the exit message is committed to the parent as continuation
// context rather than a user message, and no assistant message follows it.

//! A review's outcome: the reviewer's answer read as findings, the findings
//! as text, and the note the parent conversation reads.

use std::path::PathBuf;

use hanafubuki_agent_kernel::{
    AgentContinuationHookGeneration, AgentContinuationHookId, AgentContinuationHookProviderId,
    AgentSessionId, ConversationItem,
};
use serde::Serialize;
use serde_json::{Map, Value};

/// What a review found, read from its reviewer's final answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewOutput {
    pub findings: Vec<AgentReviewFinding>,
    /// The reviewer's verdict, `patch is correct` or `patch is incorrect`
    /// as the rubric asks; empty when it gave none.
    pub overall_correctness: String,
    /// Why; the whole answer when it was not the JSON the rubric asks for.
    pub overall_explanation: String,
    pub overall_confidence_score: f32,
    /// The review session, a child of the reviewed one.
    pub child_session: AgentSessionId,
}

/// One issue the reviewer flagged.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewFinding {
    pub title: String,
    pub body: String,
    pub confidence_score: f32,
    /// 0 (P0, drop everything) to 3 (P3, nice to have), when given.
    pub priority: Option<i32>,
    pub code_location: Option<AgentReviewCodeLocation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentReviewCodeLocation {
    pub absolute_file_path: PathBuf,
    pub line_range: AgentReviewLineRange,
}

/// Inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AgentReviewLineRange {
    pub start: u32,
    pub end: u32,
}

/// What the user reads when a review produced nothing to show.
const REVIEW_FALLBACK_MESSAGE: &str = "Reviewer failed to output a response.";

impl AgentReviewOutput {
    /// The review as the user reads it: the explanation, then the findings,
    /// each with its location and body.
    pub fn render_text(&self) -> String {
        let mut sections = Vec::new();
        let explanation = self.overall_explanation.trim();
        if !explanation.is_empty() {
            sections.push(explanation.to_string());
        }
        if !self.findings.is_empty() {
            sections.push(findings_block(&self.findings).trim().to_string());
        }
        if sections.is_empty() {
            REVIEW_FALLBACK_MESSAGE.to_string()
        } else {
            sections.join("\n\n")
        }
    }
}

/// Reads `answer`: the JSON the rubric asks for, else the first JSON object
/// in it, else its text as the explanation.
pub(super) fn parse_review_answer(
    answer: &str,
    child_session: AgentSessionId,
) -> AgentReviewOutput {
    let object = review_object(answer.trim()).or_else(|| {
        let start = answer.find('{')?;
        let end = answer.rfind('}')?;
        (start < end)
            .then(|| review_object(&answer[start..=end]))
            .flatten()
    });
    let Some(object) = object else {
        return AgentReviewOutput {
            findings: Vec::new(),
            overall_correctness: String::new(),
            overall_explanation: answer.to_string(),
            overall_confidence_score: 0.0,
            child_session,
        };
    };
    AgentReviewOutput {
        findings: object
            .get("findings")
            .and_then(Value::as_array)
            .map(|findings| findings.iter().filter_map(finding).collect())
            .unwrap_or_default(),
        overall_correctness: text(&object, "overall_correctness"),
        overall_explanation: text(&object, "overall_explanation"),
        overall_confidence_score: score(&object, "overall_confidence_score"),
        child_session,
    }
}

/// The note the reviewed conversation reads when the review ends: its
/// findings, or that it was interrupted when there is no `output`.
pub(super) fn review_exit_note(
    output: Option<&AgentReviewOutput>,
) -> Result<ConversationItem, String> {
    let content = match output {
        Some(output) => {
            let mut results = output.overall_explanation.trim().to_string();
            if !output.findings.is_empty() {
                results.push('\n');
                results.push_str(&findings_block(&output.findings));
            }
            format!(
                "<user_action>\n  <context>User initiated a review task. Here's the full review output from reviewer model. User may select one or more comments to resolve.</context>\n  <action>review</action>\n  <results>\n  {results}\n  </results>\n  </user_action>\n"
            )
        }
        None => "<user_action>\n  <context>User initiated a review task, but was interrupted. If user asks about this, tell them to re-initiate a review with `/review` and wait for it to complete.</context>\n  <action>review</action>\n  <results>\n  None.\n  </results>\n</user_action>\n\n".to_string(),
    };
    Ok(ConversationItem::ContinuationContext {
        provider_id: AgentContinuationHookProviderId::parse("agent".to_string())
            .map_err(|error| error.to_string())?,
        hook_id: AgentContinuationHookId::parse("review-exit".to_string())
            .map_err(|error| error.to_string())?,
        generation: REVIEW_EXIT_GENERATION,
        content,
    })
}

const REVIEW_EXIT_GENERATION: AgentContinuationHookGeneration = {
    let mut bytes = [0; 32];
    bytes[31] = 1;
    AgentContinuationHookGeneration(bytes)
};

/// The findings as plain text: a header, then each finding's title,
/// location and indented body.
fn findings_block(findings: &[AgentReviewFinding]) -> String {
    let mut lines = vec![String::new()];
    lines.push(if findings.len() > 1 {
        "Full review comments:".to_string()
    } else {
        "Review comment:".to_string()
    });
    for finding in findings {
        lines.push(String::new());
        lines.push(match &finding.code_location {
            Some(location) => format!(
                "- {} — {}:{}-{}",
                finding.title,
                location.absolute_file_path.display(),
                location.line_range.start,
                location.line_range.end
            ),
            None => format!("- {}", finding.title),
        });
        lines.extend(finding.body.lines().map(|line| format!("  {line}")));
    }
    lines.join("\n")
}

/// `text` as a review's JSON object, if it is one with any of a review's
/// fields.
fn review_object(text: &str) -> Option<Map<String, Value>> {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    [
        "findings",
        "overall_correctness",
        "overall_explanation",
        "overall_confidence_score",
    ]
    .iter()
    .any(|field| object.contains_key(*field))
    .then_some(object)
}

/// A finding with a title or a body; what does not read takes its default.
fn finding(value: &Value) -> Option<AgentReviewFinding> {
    let object = value.as_object()?;
    let finding = AgentReviewFinding {
        title: text(object, "title"),
        body: text(object, "body"),
        confidence_score: score(object, "confidence_score"),
        priority: object
            .get("priority")
            .and_then(Value::as_i64)
            .and_then(|priority| i32::try_from(priority).ok()),
        code_location: object.get("code_location").and_then(code_location),
    };
    (!finding.title.is_empty() || !finding.body.is_empty()).then_some(finding)
}

fn code_location(value: &Value) -> Option<AgentReviewCodeLocation> {
    let line = |name: &str| {
        value
            .get("line_range")?
            .get(name)?
            .as_u64()
            .and_then(|line| u32::try_from(line).ok())
    };
    Some(AgentReviewCodeLocation {
        absolute_file_path: PathBuf::from(value.get("absolute_file_path")?.as_str()?),
        line_range: AgentReviewLineRange {
            start: line("start")?,
            end: line("end")?,
        },
    })
}

fn text(object: &Map<String, Value>, field: &str) -> String {
    object
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn score(object: &Map<String, Value>, field: &str) -> f32 {
    object
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_default() as f32
}
