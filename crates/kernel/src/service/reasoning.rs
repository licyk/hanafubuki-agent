//! The reasoning a session's request carries: the configured effort,
//! narrowed to what the route is known to take, with the route's largest
//! output; and what a Provider's refusal of the parameter teaches.

use hanafubuki_agent_model_api::{
    ModelMetadataSource, ModelReasoningConfig, ModelReasoningSupport,
};

use super::model_facts::configured_facts;
use super::*;

impl AgentService {
    /// The reasoning `record`'s next request carries
    /// (`request_reasoning_effort`, from the route's resolved reasoning
    /// fact; a transport that names no route has nothing known), with the
    /// route's largest output, unless only models.dev knows it. Logs once per session when the configured
    /// effort is narrowed to another, or dropped.
    pub(super) fn request_reasoning(
        &self,
        record: &mut SessionRecord,
    ) -> Option<ModelReasoningConfig> {
        let config = &record.settings.config;
        let configured = config.reasoning_effort.as_ref()?;
        let facts = record
            .settings
            .model
            .route_key(&config.model)
            .map(|route| {
                self.inner.model_metadata.resolve(
                    Some(&route),
                    &config.model,
                    configured_facts(config),
                    now_millis_u64(),
                )
            })
            .unwrap_or_default();
        let support = facts.reasoning.map(|fact| fact.value);
        let effort = hanafubuki_agent_model_api::request_reasoning_effort(
            Some(configured),
            support.as_ref(),
        );
        if effort.as_ref() != Some(configured) && !record.runtime.reasoning_narrowing_logged {
            log::warn!(
                "agent_diag stage=model.reasoning.narrowed session_id={} configured={configured} sent={} reason={}",
                record.session_id,
                effort.as_ref().map_or("none", |effort| effort.as_str()),
                match support {
                    Some(ModelReasoningSupport::Unsupported) => "unsupported",
                    _ => "not_listed",
                },
            );
            record.runtime.reasoning_narrowing_logged = true;
        }
        Some(ModelReasoningConfig {
            effort: effort?,
            // A third party's number never changes a request on its own.
            max_output_tokens: facts
                .max_output_tokens
                .filter(|fact| fact.source != ModelMetadataSource::Catalog)
                .map(|fact| fact.value),
        })
    }

    /// Learns that the route of `model_id` over `model` refuses a reasoning
    /// parameter, so its next requests carry none; a transport that names
    /// no route learns nothing.
    pub(super) fn record_unsupported_reasoning(&self, model: &dyn ModelSession, model_id: &str) {
        if let Some(route) = model.route_key(model_id) {
            self.inner
                .model_metadata
                .learn_reasoning_unsupported(&route, now_millis_u64());
        }
    }
}

#[cfg(test)]
mod tests;
