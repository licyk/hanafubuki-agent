use hanafubuki_agent_model_api::{ModelImageInputCapability, ModelRouteKey};

use super::{now_millis_u64, AgentError, AgentService, AgentSessionId, SessionCell, SessionRecord};

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
                .model_metadata
                .observed_image_input(&route_key, now_millis_u64());
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
