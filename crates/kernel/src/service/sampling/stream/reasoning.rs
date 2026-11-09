//! The text of one reasoning item: what of it is shown as it streams, and
//! what the step keeps.

use hanafubuki_agent_model_api::ModelReasoningTextKind;

use super::*;

const PART_SEPARATOR: &str = "\n\n";

/// One reasoning item of the response being assembled.
#[derive(Clone, Debug)]
pub(in crate::service) struct PendingReasoning {
    pub(in crate::service) reasoning_id: AgentReasoningId,
    started: Instant,
    summary: StreamedText,
    raw: StreamedText,
    /// The bytes the item's `ReasoningDelta` events carried.
    shown_bytes: usize,
    completed: Option<CompletedReasoning>,
}

/// One kind of an item's streamed text, its parts joined by a blank line and
/// cut at the reasoning text bound.
#[derive(Clone, Debug, Default)]
struct StreamedText {
    text: String,
    part: Option<u32>,
    truncated: bool,
}

#[derive(Clone, Debug)]
struct CompletedReasoning {
    text: String,
    truncated: bool,
    duration_millis: u64,
}

impl PendingReasoning {
    pub(in crate::service) fn new(reasoning_id: AgentReasoningId) -> Self {
        Self {
            reasoning_id,
            started: Instant::now(),
            summary: StreamedText::default(),
            raw: StreamedText::default(),
            shown_bytes: 0,
            completed: None,
        }
    }

    pub(in crate::service) fn is_completed(&self) -> bool {
        self.completed.is_some()
    }

    /// Adds a streamed fragment and returns what of it to show, if anything.
    pub(in crate::service) fn push(
        &mut self,
        kind: ModelReasoningTextKind,
        index: u32,
        delta: &str,
        limit: usize,
    ) -> Option<String> {
        let streamed = match kind {
            ModelReasoningTextKind::Summary => &mut self.summary,
            ModelReasoningTextKind::Raw => &mut self.raw,
        };
        let separator = if !streamed.text.is_empty() && streamed.part != Some(index) {
            PART_SEPARATOR
        } else {
            ""
        };
        streamed.part = Some(index);
        let piece = format!("{separator}{delta}");
        let kept = bounded_prefix(&piece, limit.saturating_sub(streamed.text.len()));
        streamed.truncated |= kept.len() < piece.len();
        streamed.text.push_str(kept);
        let shown = bounded_prefix(kept, limit.saturating_sub(self.shown_bytes));
        self.shown_bytes += shown.len();
        (!shown.is_empty()).then(|| shown.to_string())
    }

    /// Completes the item with the text of its completed item, and returns
    /// the rest of the kept text still to show and the item's duration.
    ///
    /// The kept text is the summary when there is one, else the raw text,
    /// each from the completed item when it carries any, else as streamed.
    /// When the kept text continues what was shown, the rest is shown; when
    /// it does not (an item that streamed both kinds), the commit replaces
    /// what was shown.
    pub(in crate::service) fn complete(
        &mut self,
        summary: Vec<String>,
        content: Vec<String>,
        limit: usize,
    ) -> (Option<String>, u64) {
        let (text, truncated) = match (
            joined_parts(&summary, limit),
            &self.summary,
            joined_parts(&content, limit),
            &self.raw,
        ) {
            (Some(kept), ..) => kept,
            (None, streamed, ..) if !streamed.text.is_empty() => {
                (streamed.text.clone(), streamed.truncated)
            }
            (None, _, Some(kept), _) => kept,
            (None, _, None, streamed) => (streamed.text.clone(), streamed.truncated),
        };
        let shown = match (self.summary.text.is_empty(), self.raw.text.is_empty()) {
            (true, true) => Some(""),
            (false, true) => Some(self.summary.text.as_str()),
            (true, false) => Some(self.raw.text.as_str()),
            (false, false) => None,
        };
        let tail = shown
            .and_then(|shown| text.strip_prefix(shown))
            .map(|tail| bounded_prefix(tail, limit.saturating_sub(self.shown_bytes)))
            .filter(|tail| !tail.is_empty())
            .map(str::to_string);
        let duration_millis = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.completed = Some(CompletedReasoning {
            text,
            truncated,
            duration_millis,
        });
        (tail, duration_millis)
    }

    /// The conversation item of a completed reasoning item.
    pub(in crate::service) fn into_item(self, sampling_step: u32) -> Option<ConversationItem> {
        let completed = self.completed?;
        Some(ConversationItem::Reasoning {
            text: completed.text,
            duration_millis: completed.duration_millis,
            sampling_step,
            truncated: completed.truncated,
        })
    }
}

impl AgentService {
    /// Shows the rest of a completed reasoning item's text and ends it.
    pub(super) async fn reasoning_completed(
        &self,
        identity: TurnIdentity,
        reasoning_id: AgentReasoningId,
        tail: Option<String>,
        duration_millis: u64,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        if let Some(delta) = tail {
            self.emit(
                identity,
                AgentEventKind::ReasoningDelta {
                    reasoning_id,
                    delta,
                },
                event_sink,
            )
            .await?;
        }
        self.emit(
            identity,
            AgentEventKind::ReasoningCompleted {
                reasoning_id,
                duration_millis,
            },
            event_sink,
        )
        .await?;
        self.reasoning_item_completed(identity, event_sink).await
    }

    /// Ends the reasoning items a response that stopped early left open,
    /// keeping their text as streamed.
    pub(super) async fn complete_unfinished_reasoning(
        &self,
        identity: TurnIdentity,
        response: &mut StreamedResponse,
        event_sink: &dyn AgentEventSink,
    ) -> Result<(), AgentError> {
        let limit = self.inner.limits.maximum_reasoning_text_bytes;
        for item_id in &response.item_order {
            let Some(PendingModelItem::Reasoning(reasoning)) =
                response.pending_items.get_mut(item_id)
            else {
                continue;
            };
            if reasoning.is_completed() {
                continue;
            }
            let (tail, duration_millis) = reasoning.complete(Vec::new(), Vec::new(), limit);
            let reasoning_id = reasoning.reasoning_id;
            self.reasoning_completed(identity, reasoning_id, tail, duration_millis, event_sink)
                .await?;
        }
        Ok(())
    }
}

/// The non-empty `parts` joined by a blank line and cut at `limit`, with
/// whether they were cut, or `None` when no part has text.
fn joined_parts(parts: &[String], limit: usize) -> Option<(String, bool)> {
    let joined = parts
        .iter()
        .filter(|part| !part.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(PART_SEPARATOR);
    if joined.is_empty() {
        return None;
    }
    let kept = bounded_prefix(&joined, limit);
    Some((kept.to_string(), kept.len() < joined.len()))
}

/// The longest prefix of `text` of at most `limit` bytes that ends on a
/// character boundary.
fn bounded_prefix(text: &str, limit: usize) -> &str {
    &text[..text.floor_char_boundary(limit)]
}
