use std::collections::HashMap;

use hanafubuki_agent_model_api::{ModelImageInputCapability, ModelRouteKey};

use super::{now_millis, AgentError, AgentService, AgentSessionId, SessionCell, SessionRecord};

const MAXIMUM_IMAGE_CAPABILITY_ENTRIES: usize = 256;
const IMAGE_CAPABILITY_TTL_MILLIS: u128 = 6 * 60 * 60 * 1_000;

#[derive(Clone, Copy)]
struct CapabilityEntry {
    capability: ModelImageInputCapability,
    observed_at: u128,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct ModelImageCapabilityCache {
    entries: HashMap<ModelRouteKey, CapabilityEntry>,
    next_sequence: u64,
}

impl ModelImageCapabilityCache {
    pub(super) fn capability(
        &mut self,
        key: &ModelRouteKey,
        now: u128,
    ) -> ModelImageInputCapability {
        self.expire(now);
        self.entries
            .get(key)
            .map(|entry| entry.capability)
            .unwrap_or(ModelImageInputCapability::Unknown)
    }

    pub(super) fn record_unsupported(&mut self, key: ModelRouteKey, now: u128) {
        self.expire(now);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.entries.insert(
            key,
            CapabilityEntry {
                capability: ModelImageInputCapability::Unsupported,
                observed_at: now,
                sequence: self.next_sequence,
            },
        );
        while self.entries.len() > MAXIMUM_IMAGE_CAPABILITY_ENTRIES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    fn expire(&mut self, now: u128) {
        self.entries.retain(|_, entry| {
            now.saturating_sub(entry.observed_at) <= IMAGE_CAPABILITY_TTL_MILLIS
        });
    }
}

impl AgentService {
    /// Applies the latest observed image input capability of the session's
    /// model route.
    pub(super) async fn refresh_session_image_input_capability(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        if let Some((route_key, observed)) = self.observed_image_input(&cell)? {
            let mut record = cell.lock_after_persistence().await?;
            apply_observed_image_input(&mut record, &route_key, observed);
        }
        Ok(())
    }

    /// `refresh_session_image_input_capability` for synchronous callers off
    /// the async runtime.
    pub(super) fn refresh_session_image_input_capability_blocking(
        &self,
        session_id: AgentSessionId,
    ) -> Result<(), AgentError> {
        let cell = self.session(session_id)?;
        if let Some((route_key, observed)) = self.observed_image_input(&cell)? {
            let mut record = cell.lock_after_persistence_blocking()?;
            apply_observed_image_input(&mut record, &route_key, observed);
        }
        Ok(())
    }

    fn observed_image_input(
        &self,
        cell: &SessionCell,
    ) -> Result<Option<(ModelRouteKey, ModelImageInputCapability)>, AgentError> {
        let route_key = {
            let record = cell.lock()?;
            record
                .settings
                .model
                .route_key(&record.settings.config.model)
        };
        Ok(route_key.map(|route_key| {
            let observed = self
                .inner
                .image_capabilities
                .lock()
                .capability(&route_key, now_millis());
            (route_key, observed)
        }))
    }
}

fn apply_observed_image_input(
    record: &mut SessionRecord,
    route_key: &ModelRouteKey,
    observed: ModelImageInputCapability,
) {
    if record
        .settings
        .model
        .route_key(&record.settings.config.model)
        .as_ref()
        == Some(route_key)
    {
        record.settings.effective_input_capabilities = record
            .settings
            .config
            .input_capabilities
            .effective(observed);
    }
}

#[cfg(test)]
mod tests;
