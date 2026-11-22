use std::collections::BTreeMap;

use serde_json::Value;

use super::step::Step;
use crate::tolerant_decode::{token_count, token_usage, UsageFields};
use crate::{
    ModelError, ModelEvent, ModelResponseMetadata, ModelResult, ModelRouteKey, ModelUsage,
};

pub(in crate::gemini) struct Decoder {
    route: ModelRouteKey,
    response_id: Option<String>,
    steps: BTreeMap<u64, PendingStep>,
    usage: Option<ModelUsage>,
}

struct PendingStep {
    step: Step,
    stopped: bool,
}

impl Decoder {
    pub(in crate::gemini) fn new(route: ModelRouteKey) -> Self {
        Self {
            route,
            response_id: None,
            steps: BTreeMap::new(),
            usage: None,
        }
    }

    pub(in crate::gemini) fn decode(
        &mut self,
        value: &Value,
        metadata: &ModelResponseMetadata,
    ) -> ModelResult<Vec<ModelEvent>> {
        let interaction = &value["interaction"];
        let mut events = Vec::new();
        match value["event_type"].as_str() {
            Some("interaction.created") => {
                let id = identity(interaction, "id")?;
                self.response_id = Some(id.to_owned());
                events.push(ModelEvent::ResponseStarted {
                    response_id: id.to_owned(),
                    metadata: metadata.clone(),
                });
            }
            Some("interaction.status_update") => {
                self.usage = usage(value.get("usage")).or(self.usage);
            }
            Some("step.start") => {
                let index = index(value)?;
                let id = self.response_id.as_deref().ok_or_else(|| {
                    ModelError::Protocol(
                        "Gemini Interactions step preceded interaction.created".into(),
                    )
                })?;
                let item_id = format!("{id}-step-{index}");
                if let Some(step) = Step::start(&value["step"], item_id, &mut events)? {
                    self.steps.insert(
                        index,
                        PendingStep {
                            step,
                            stopped: false,
                        },
                    );
                }
            }
            Some("step.delta") => {
                if let Some(step) = self
                    .steps
                    .get_mut(&index(value)?)
                    .filter(|step| !step.stopped)
                {
                    if let Some(event) = step.step.apply(&value["delta"]) {
                        events.push(event);
                    }
                }
            }
            Some("step.stop") => {
                self.usage = usage(value.get("usage")).or(self.usage);
                if let Some(step) = self.steps.get_mut(&index(value)?) {
                    step.stopped = true;
                }
                // Parallel calls may stop out of order; commit the original step order into history.
                while self
                    .steps
                    .first_key_value()
                    .is_some_and(|(_, step)| step.stopped)
                {
                    if let Some((_, step)) = self.steps.pop_first() {
                        events.push(ModelEvent::OutputItemCompleted {
                            item: step.step.finish(&self.route),
                        });
                    }
                }
            }
            Some("interaction.completed") => {
                let id = identity(interaction, "id")?;
                if self.response_id.as_deref() != Some(id) {
                    return Err(ModelError::Protocol(
                        "Gemini Interactions completion does not match the active interaction"
                            .into(),
                    ));
                }
                let status = identity(interaction, "status")?;
                let usage = usage(interaction.get("usage")).or(self.usage);
                let terminal =
                    match status {
                        "completed" | "requires_action" => {
                            if !self.steps.is_empty() {
                                return Err(ModelError::StreamInterrupted(
                                    "Gemini Interactions completed with an unfinished step".into(),
                                ));
                            }
                            ModelEvent::ResponseCompleted {
                                response_id: id.into(),
                                usage,
                                end_turn: Some(status == "completed"),
                            }
                        }
                        "incomplete" | "requires_continuation" => ModelEvent::ResponseIncomplete {
                            response_id: id.into(),
                            usage,
                            reason: status.into(),
                        },
                        "failed" => {
                            return Err(ModelError::Remote {
                                status: None,
                                code: Some("failed".into()),
                                message: "Gemini interaction failed".into(),
                                request_id: metadata.request_id.clone(),
                            })
                        }
                        "cancelled" => return Err(ModelError::Cancelled),
                        _ => return Err(ModelError::Protocol(
                            "Gemini Interactions completion has an unrecognized terminal status"
                                .into(),
                        )),
                    };
                events.push(terminal);
            }
            _ => {}
        }
        Ok(events)
    }
}

pub(super) fn identity<'a>(value: &'a Value, field: &str) -> ModelResult<&'a str> {
    value[field]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ModelError::Protocol(format!("Gemini Interactions event requires {field}")))
}

fn index(value: &Value) -> ModelResult<u64> {
    value["index"]
        .as_u64()
        .ok_or_else(|| ModelError::Protocol("Gemini Interactions step requires an index".into()))
}

fn usage(value: Option<&Value>) -> Option<ModelUsage> {
    let value = value?;
    let mut usage = token_usage(
        "gemini_interactions",
        Some(value),
        &UsageFields {
            input: "total_input_tokens",
            output: "total_output_tokens",
            cache_read: &["/total_cached_tokens"],
        },
    )?;
    let thought = token_count("gemini_interactions", value, "total_thought_tokens")
        .ok()
        .flatten()
        .unwrap_or(0);
    usage.output_tokens = usage.output_tokens.saturating_add(thought);
    Some(usage)
}
